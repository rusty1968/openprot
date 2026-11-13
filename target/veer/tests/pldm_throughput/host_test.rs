// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Host side of the VeeR PLDM throughput benchmark: a PLDM update agent
//! (`veer_pldm_ua`) updates one component on the firmware's PLDM firmware device
//! over MCTP over I3C, and the download is timed on the host's wall clock, from
//! the first `RequestFirmwareData` to `TransferComplete`. That is how
//! caliptra-mcu-sw measures its own PLDM transfer speed, so the figure is
//! dominated by the per-chunk round trip (the host's pacing and the transport),
//! not the firmware's raw speed. It prints one line:
//!
//! ```text
//! PLDM_THROUGHPUT: 4096 bytes in 1.234 s (3319.0 B/s, 3.2 KB/s)
//! ```
//!
//! This is a benchmark, not a gate: nothing asserts on the number, only that the
//! update completed. Run it with `--test_arg=--nocapture` to see the line.
//!
//! The component is 4 KiB on the emulator and 64 KiB on the VCK190 (the size
//! caliptra-mcu-sw's own benchmark uses); `PLDM_BENCH_IMAGE_SIZE` overrides it.
//! The packets of one message the UA writes are paced 5 ms apart on the emulator
//! and 25 ms on the FPGA, where the controller's receive FIFO holds 256 bytes
//! and the model adds its own delay to every write. The pause is part of the
//! measured time, so compare runs on the same interface only.

use caliptra_mcu_hw_model::McuHwModel;
use caliptra_mcu_testing_common::i3c_socket::BufferedStream;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::Duration;
use test_boot_files::BootFiles;
use veer_host_util::{on_fpga, wait_for_log};
use veer_pldm_ua::{run_update, UpdateConfig};

/// Logged by the FD once it is running and waiting for the update agent.
const READY: &str = "pldm fd: waiting for update agent";

/// The FD's chunk cap (`XFER_CAP` in `pldm_fd.rs`).
const FD_CHUNK: u32 = 960;

fn image_size() -> u32 {
    if let Ok(v) = std::env::var("PLDM_BENCH_IMAGE_SIZE") {
        return v.parse().expect("PLDM_BENCH_IMAGE_SIZE is a byte count");
    }
    if on_fpga() {
        64 * 1024
    } else {
        4096
    }
}

fn write_gap() -> Duration {
    if on_fpga() {
        Duration::from_millis(25)
    } else {
        Duration::from_millis(5)
    }
}

#[test]
fn pldm_throughput_host_test() {
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

    // The FD must be listening before the first command arrives.
    wait_for_log(&mut hw, &mut log, READY);

    let size = image_size();
    let report = run_update(
        &mut hw,
        &mut stream,
        &mut log,
        target_addr,
        UpdateConfig {
            image_size: size,
            max_transfer_size: FD_CHUNK,
            write_gap: write_gap(),
        },
    );

    let transfer = report.transfer.expect("the FD never sent TransferComplete");
    let secs = transfer.as_secs_f64();
    let bps = report.bytes_per_second().expect("a timed transfer");
    println!(
        "PLDM_THROUGHPUT: {} bytes in {secs:.3} s ({bps:.1} B/s, {:.1} KB/s), {} chunks, {}",
        report.downloaded,
        bps / 1000.0,
        report.chunks,
        if on_fpga() { "fpga" } else { "emulator" },
    );

    assert!(report.applied, "the FD never reported ApplyComplete");
    assert_eq!(
        report.downloaded, size as usize,
        "the whole image should have been requested"
    );
    assert_eq!(
        report.chunks,
        size.div_ceil(FD_CHUNK),
        "the FD should ask for the image in chunks of its cap"
    );

    hw.step_until_exit_success()
        .expect("firmware did not exit with PASS");
    drop(stream);
}
