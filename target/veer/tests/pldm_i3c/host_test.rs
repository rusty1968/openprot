// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Host side of the VeeR PLDM-over-I3C firmware update test: a PLDM update agent
//! (`veer_pldm_ua`) updates one 512 byte component on the firmware's PLDM
//! firmware device, over MCTP over I3C through `caliptra_mcu_hw_model`.
//!
//! The firmware's FD asks for the image in 180 byte chunks, so the update is
//! three `RequestFirmwareData` exchanges, then `TransferComplete`,
//! `VerifyComplete` and `ApplyComplete`, and the host activates the image. The
//! firmware exits PASS once it has applied it and its idle timeout has run out. The same binary runs on the
//! emulator and, through the model's I3C forwarding, on the VCK190.

use caliptra_mcu_hw_model::McuHwModel;
use caliptra_mcu_testing_common::i3c_socket::BufferedStream;
use std::net::{SocketAddr, TcpListener, TcpStream};
use test_boot_files::BootFiles;
use veer_host_util::{wait_for_log, MAX_PACKET, WRITE_GAP};
use veer_pldm_ua::{run_update, UpdateConfig};

/// Logged by the FD once it is running and waiting for the update agent.
const READY: &str = "pldm fd: waiting for update agent";

/// The component the host serves; must match what the firmware expects (the FD
/// takes the size from `UpdateComponent`, so this is what it downloads).
const IMAGE_SIZE: u32 = 512;

/// The FD's chunk cap (`XFER_CAP` in `pldm_fd.rs`).
const FD_CHUNK: u32 = 180;

#[test]
fn pldm_i3c_firmware_update_host_test() {
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

    let report = run_update(
        &mut hw,
        &mut stream,
        &mut log,
        target_addr,
        UpdateConfig {
            image_size: IMAGE_SIZE,
            // Offer a chunk as large as the image; the FD takes the smaller of
            // this and its own cap.
            max_transfer_size: IMAGE_SIZE,
            write_gap: WRITE_GAP,
            max_packet: MAX_PACKET,
        },
    );
    println!("pldm-i3c host: {report:?}");

    assert!(report.applied, "the FD never reported ApplyComplete");
    assert_eq!(
        report.downloaded, IMAGE_SIZE as usize,
        "the whole image should have been requested"
    );
    assert_eq!(
        report.chunks,
        IMAGE_SIZE.div_ceil(FD_CHUNK),
        "the FD should ask for the image in chunks of its cap"
    );

    hw.step_until_exit_success()
        .expect("firmware did not exit with PASS");
    drop(stream);
}
