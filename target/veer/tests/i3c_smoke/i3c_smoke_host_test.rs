// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Host-side harness for the VeeR I3C smoke test.
//!
//! Launches the emulator runner, waits for the firmware to report it is waiting
//! for a private write, then injects a valid private-write frame over the I3C
//! TCP socket (127.0.0.1:65534).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::string::{String, ToString};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use std::vec::Vec;

const I3C_HOST: &str = "127.0.0.1";
const I3C_PORT: u16 = 65534;
const DEFAULT_TARGET_ADDR: u8 = 0x08;
const PAYLOAD: [u8; 4] = [0x01, 0x02, 0x03, 0x04];

fn crc8_smbus(data: &[u8]) -> u8 {
    let mut crc: u8 = 0;
    for &value in data {
        crc ^= value;
        for _ in 0..8 {
            if (crc & 0x80) != 0 {
                crc = (crc << 1) ^ 0x07;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

fn connect_i3c_socket(timeout: Duration) -> Result<TcpStream, String> {
    let deadline = Instant::now() + timeout;
    let mut last_err: Option<String> = None;
    while Instant::now() < deadline {
        match TcpStream::connect((I3C_HOST, I3C_PORT)) {
            Ok(s) => return Ok(s),
            Err(e) => {
                last_err = Some(e.to_string());
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
    Err(format!(
        "timed out connecting to {}:{} ({})",
        I3C_HOST,
        I3C_PORT,
        last_err.unwrap_or_else(|| "unknown error".to_string())
    ))
}

fn make_private_write_header(target_addr: u8, data_len: u16) -> [u8; 9] {
    // IncomingHeader: to_addr:u8 + command:[u32;2] LE.
    // Private write command (rnw=0): word0 = 0, data_length in word1[23:16].
    let cmd_word0: u32 = 0;
    let cmd_word1: u32 = ((data_len as u32) & 0xFF) << 16;

    let mut out = [0u8; 9];
    out[0] = target_addr;
    out[1..5].copy_from_slice(&cmd_word0.to_le_bytes());
    out[5..9].copy_from_slice(&cmd_word1.to_le_bytes());
    out
}

fn send_private_write_on_stream(
    stream: &mut TcpStream,
    target_addr: u8,
    payload: &[u8],
) -> Result<(), String> {
    let mut pec_input = Vec::with_capacity(1 + payload.len());
    pec_input.push(target_addr << 1);
    pec_input.extend_from_slice(payload);
    let pec = crc8_smbus(&pec_input);

    let mut body = Vec::with_capacity(payload.len() + 1);
    body.extend_from_slice(payload);
    body.push(pec);

    let header = make_private_write_header(target_addr, body.len() as u16);
    stream
        .set_nonblocking(false)
        .map_err(|e| format!("failed to set blocking mode for header write: {}", e))?;
    stream
        .write_all(&header)
        .map_err(|e| format!("failed writing I3C header: {}", e))?;
    stream
        .set_nonblocking(true)
        .map_err(|e| format!("failed to set nonblocking mode for payload write: {}", e))?;
    stream
        .write_all(&body)
        .map_err(|e| format!("failed writing I3C body: {}", e))?;

    println!(
        "I3C HOST TRACE: wrote frame to addr=0x{target_addr:02x} header={:02x?} body={:02x?}",
        header,
        body
    );

    Ok(())
}

fn extract_target_addr(line: &str) -> Option<u8> {
    let marker = "target DynamicI3cAddress(";
    let start = line.find(marker)? + marker.len();
    let rest = &line[start..];
    let end = rest.find(')')?;
    let parsed = rest[..end].parse::<u16>().ok()?;
    u8::try_from(parsed).ok()
}

fn resolve_runner_path() -> PathBuf {
    let srcdir = std::env::var("TEST_SRCDIR")
        .expect("missing TEST_SRCDIR environment variable");
    let workspace = std::env::var("TEST_WORKSPACE")
        .expect("missing TEST_WORKSPACE environment variable");
    let workspace_root = Path::new(&srcdir).join(&workspace);

    let candidate = workspace_root.join(
        "target/veer/tests/i3c_smoke/i3c_smoke_runner.sh",
    );
    if candidate.exists() {
        return candidate;
    }

    panic!(
        "unable to locate i3c smoke runner at {:?}; TEST_SRCDIR={:?}, TEST_WORKSPACE={:?}",
        candidate,
        std::env::var("TEST_SRCDIR").ok(),
        std::env::var("TEST_WORKSPACE").ok(),
    );
}

fn resolve_runner_cwd(runner: &Path) -> PathBuf {
    if let (Ok(srcdir), Ok(workspace)) = (
        std::env::var("TEST_SRCDIR"),
        std::env::var("TEST_WORKSPACE"),
    ) {
        let root = Path::new(&srcdir).join(&workspace);
        if root.exists() {
            return root;
        }
    }
    runner
        .parent()
        .expect("runner path has no parent directory")
        .to_path_buf()
}

#[test]
fn i3c_smoke_host_test() {
    let runner = resolve_runner_path();
    let runner_cwd = resolve_runner_cwd(&runner);
    // Only the firmware's own log line counts as readiness: the emulator's
    // I3C socket opens minutes earlier (before the MCU ROM and recovery flow
    // finish), and frames sent that early race firmware boot.
    let ready_marker = "waiting for private write";

    let mut child = Command::new(&runner)
        .current_dir(runner_cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn i3c_smoke_runner");

    let stdout = child
        .stdout
        .take()
        .expect("failed to capture runner stdout");
    let stderr = child
        .stderr
        .take()
        .expect("failed to capture runner stderr");

    let mut stdout_reader = BufReader::new(stdout);
    let stderr_reader = BufReader::new(stderr);

    let sent = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let target_addr = Arc::new(AtomicU8::new(DEFAULT_TARGET_ADDR));
    let connect_attempts = Arc::new(AtomicU32::new(0));
    let write_successes = Arc::new(AtomicU32::new(0));
    let write_failures = Arc::new(AtomicU32::new(0));

    let sent_sender = Arc::clone(&sent);
    let ready_sender = Arc::clone(&ready);
    let done_sender = Arc::clone(&done);
    let target_sender = Arc::clone(&target_addr);
    let connect_attempts_sender = Arc::clone(&connect_attempts);
    let write_successes_sender = Arc::clone(&write_successes);
    let write_failures_sender = Arc::clone(&write_failures);
    let sender_thread = thread::spawn(move || {
        while !done_sender.load(Ordering::Relaxed) {
            if !ready_sender.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(5));
                continue;
            }

            let addr = target_sender.load(Ordering::Relaxed);
            println!("I3C HOST TRACE: readiness observed, connecting to {}:{} with addr=0x{addr:02x}", I3C_HOST, I3C_PORT);
            connect_attempts_sender.fetch_add(1, Ordering::Relaxed);
            match connect_i3c_socket(Duration::from_secs(5)) {
                Ok(mut stream) => {
                    // Keep sending until the runner exits: the firmware
                    // terminates the emulator as soon as one frame arrives,
                    // so exit is the natural stop condition.
                    while !done_sender.load(Ordering::Relaxed) {
                        match send_private_write_on_stream(&mut stream, addr, &PAYLOAD) {
                            Ok(()) => {
                                write_successes_sender.fetch_add(1, Ordering::Relaxed);
                                sent_sender.store(true, Ordering::Relaxed);
                            }
                            Err(e) => {
                                println!("I3C HOST TRACE: send failed: {}", e);
                                write_failures_sender.fetch_add(1, Ordering::Relaxed);
                                // Reconnect via the outer loop.
                                break;
                            }
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                }
                Err(e) => {
                    println!("I3C HOST TRACE: connect failed: {}", e);
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });

    let target_stderr = Arc::clone(&target_addr);
    let ready_stderr = Arc::clone(&ready);
    let stderr_thread = thread::spawn(move || {
        let mut reader = stderr_reader;
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .expect("failed to read runner stderr");
            if n == 0 {
                break;
            }
            eprint!("{}", line);

            if let Some(addr) = extract_target_addr(&line) {
                target_stderr.store(addr, Ordering::Relaxed);
            }
            if line.contains(ready_marker) {
                ready_stderr.store(true, Ordering::Relaxed);
            }
        }
    });

    let mut line = String::new();

    loop {
        line.clear();
        let n = stdout_reader
            .read_line(&mut line)
            .expect("failed to read runner stdout");
        if n == 0 {
            break;
        }

        print!("{}", line);

        if let Some(addr) = extract_target_addr(&line) {
            target_addr.store(addr, Ordering::Relaxed);
        }

        if line.contains(ready_marker) {
            ready.store(true, Ordering::Relaxed);
        }
    }

    done.store(true, Ordering::Relaxed);
    sender_thread
        .join()
        .expect("failed to join sender thread");

    stderr_thread
        .join()
        .expect("failed to join stderr reader thread");

    let status = child.wait().expect("failed to wait for runner");

    assert!(
        sent.load(Ordering::Relaxed),
        "did not send I3C payload; ready={}, target_addr=0x{:02x}, connect_attempts={}, write_successes={}, write_failures={}",
        ready.load(Ordering::Relaxed),
        target_addr.load(Ordering::Relaxed),
        connect_attempts.load(Ordering::Relaxed),
        write_successes.load(Ordering::Relaxed),
        write_failures.load(Ordering::Relaxed)
    );
    assert!(status.success(), "runner exited with status: {}", status);
}
