// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Host side of the VeeR MCTP-over-I3C echo test, written the way
//! caliptra-mcu-sw's own MCTP tests drive their firmware: boot through
//! `caliptra_mcu_hw_model` with an I3C socket, send an MCTP request as private
//! writes, then wait for the target's IBIs and read each reply packet
//! (`BufferedStream::receive_ibi` and `receive_private_read`, which checks and
//! strips the PEC). Single-threaded: the model is only advanced by `step_until`.
//!
//! Two requests go through one boot (the firmware echoes `ECHO_REQUESTS`, then
//! exits PASS):
//!
//! 1. A short payload: one packet each way.
//! 2. A payload larger than the MTU: the host fragments the request into several
//!    packets, which the target reassembles; the target fragments the echo, and
//!    the i3c server holds one staged packet at a time, so the MCTP server waits
//!    out `Busy` between fragments until each has been read. The host
//!    reassembles the reply.
//!
//! Each packet is DSP0233: the MCTP packet alone (nothing precedes it; the PEC
//! that `send_private_write` appends covers the address byte and the packet).
//!
//! The same binary runs on the emulator (x86_64) and on the VCK190, where the
//! model forwards the socket to the FPGA's I3C host controller.

use caliptra_mcu_hw_model::McuHwModel;
use caliptra_mcu_testing_common::i3c_socket::BufferedStream;
use std::net::{SocketAddr, TcpListener, TcpStream};
use test_boot_files::BootFiles;
use veer_host_util::{drain_log, read_message, send_message, wait_for_log, Message};

const TARGET_EID: u8 = 8;
const HOST_EID: u8 = 9;
/// The message type the echo app listens for (`ECHO_MSG_TYPE` in `//services/mctp/echo`).
const ECHO_MSG_TYPE: u8 = 1;

/// Logged by `echo.rs` once its listener is armed.
const READY: &str = "mctp echo: waiting for request";

/// The payloads the host sends and expects echoed back: one packet, then
/// several.
fn payloads() -> [Vec<u8>; 2] {
    let short = b"mctp-i3c".to_vec();
    let long: Vec<u8> = (0..600u32).map(|i| (i % 251) as u8).collect();
    [short, long]
}

/// Send `payload` as one MCTP request and read the echo. Checks the reply is a
/// response to this request (EIDs swapped, tag owner clear, same tag); the
/// packet-level checks are in `read_message`.
fn echo(
    hw: &mut impl McuHwModel,
    stream: &mut BufferedStream,
    log: &mut String,
    target_addr: u8,
    payload: &[u8],
    tag: u8,
) -> Message {
    let mut body = vec![ECHO_MSG_TYPE];
    body.extend_from_slice(payload);
    println!("mctp-i3c host: request of {} bytes", payload.len());
    let request = Message {
        dest: TARGET_EID,
        src: HOST_EID,
        tag_owner: true,
        tag,
        body,
        packets: 0,
    };
    send_message(hw, stream, log, target_addr, &request);

    let reply = read_message(hw, stream, log, target_addr);
    assert_eq!(reply.dest, HOST_EID, "reply destination EID");
    assert_eq!(reply.src, TARGET_EID, "reply source EID");
    assert!(!reply.tag_owner, "reply is a response (tag owner clear)");
    assert_eq!(reply.tag, tag, "reply tag");
    reply
}

#[test]
fn mctp_i3c_echo_host_test() {
    // A free port for the model's I3C socket.
    let i3c_port = TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();

    let files = BootFiles::from_env();
    let mut params = files.init_params();
    params.i3c_port = Some(i3c_port);
    let mut hw = caliptra_mcu_hw_model::new(params).expect("hardware model failed to boot");

    hw.start_i3c_controller();

    let port = hw.i3c_port().unwrap();
    let target_addr = hw.i3c_address().unwrap();

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = TcpStream::connect(addr).expect("failed to connect to the I3C socket");
    let mut stream = BufferedStream::new(stream);

    let mut log = String::new();

    // The listener must be armed before the request arrives.
    wait_for_log(&mut hw, &mut log, READY);

    for (n, payload) in payloads().iter().enumerate() {
        let reply = echo(
            &mut hw,
            &mut stream,
            &mut log,
            target_addr,
            payload,
            n as u8,
        );
        println!(
            "mctp-i3c host: echo {n}: {} packet(s), {} byte body",
            reply.packets,
            reply.body.len()
        );
        assert_eq!(reply.body[0] & 0x7f, ECHO_MSG_TYPE, "message type");
        assert_eq!(
            &reply.body[1..],
            &payload[..],
            "echoed payload (request {n})"
        );
        if n == 0 {
            assert_eq!(reply.packets, 1, "a short message is one packet");
        } else {
            assert!(
                reply.packets > 1,
                "the long message should have been fragmented"
            );
        }
    }
    drain_log(&mut hw, &mut log);

    hw.step_until_exit_success()
        .expect("firmware did not exit with PASS");
    drop(stream);
}
