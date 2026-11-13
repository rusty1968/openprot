// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Host side of the VeeR PLDM throughput benchmark, driven by caliptra-mcu-sw's
//! own update agent (`caliptra-mcu-pldm-ua`'s `PldmDaemon`) instead of the
//! hand-rolled one in `host_test.rs`.
//!
//! The native agent is what the earlier benchmark numbers on the VCK190 came
//! from, so this is the run to compare with them: it packetizes with
//! `MctpUtil`'s 64 byte payloads (68 byte packets, a load the target's receive
//! FIFO takes) and paces its writes the way caliptra-mcu-sw's tests do. The
//! daemon runs on its own threads and talks to the model's I3C socket through
//! its own connection; this thread steps the model, which only advances while
//! it is stepped, and watches the agent's state machine.
//!
//! The download is timed on the host's wall clock, from the agent entering
//! `Download` to leaving it for `Verify`. One line is printed:
//!
//! ```text
//! PLDM_THROUGHPUT: 4096 bytes in 1.234 s (3319.0 B/s, 3.2 KB/s), native, fpga
//! ```
//!
//! A benchmark, not a gate: nothing asserts on the number, only that the update
//! completed. Run it with `--test_arg=--nocapture`. The component is 4 KiB on the
//! emulator and 64 KiB on the VCK190; `PLDM_BENCH_IMAGE_SIZE` overrides it.

use caliptra_mcu_hw_model::McuHwModel;
use caliptra_mcu_pldm_common::protocol::firmware_update::ComponentClassification;
use caliptra_mcu_pldm_fw_pkg::{
    manifest::{
        ComponentImageInformation, Descriptor, DescriptorType, FirmwareDeviceIdRecord,
        PackageHeaderInformation, StringType,
    },
    FirmwareManifest,
};
use caliptra_mcu_pldm_ua::daemon::{Options, PldmDaemon};
use caliptra_mcu_pldm_ua::transport::{EndpointId, PldmTransport};
use caliptra_mcu_pldm_ua::{discovery_sm, update_sm};
use caliptra_mcu_testing_common::mctp_transport::MctpTransport;
use caliptra_mcu_testing_common::spawn_with_emulator_state;
use chrono::{TimeZone, Utc};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use test_boot_files::BootFiles;
use uuid::Uuid;
use veer_host_util::{on_fpga, wait_for_log};

/// Logged by the FD once it is running and waiting for the update agent.
const READY: &str = "pldm fd: waiting for update agent";

/// The device UUID the firmware device reports (`pldm_fd`'s `DEVICE_UUID`).
const DEVICE_UUID: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
];

/// The firmware device's endpoint id, and the agent's own. `MctpUtil` sends every
/// packet to the null EID (0), which the FD's stack accepts, and uses the
/// socket's `source` as the agent's EID: it must not be the FD's own, or the FD
/// would answer a packet that appears to come from itself.
const FD_EID: u8 = 8;
const UA_EID: u8 = 0x0a;

/// How long the whole update may take on the host's wall clock.
const UPDATE_TIMEOUT: Duration = Duration::from_secs(900);

fn image_size() -> usize {
    if let Ok(v) = std::env::var("PLDM_BENCH_IMAGE_SIZE") {
        return v.parse().expect("PLDM_BENCH_IMAGE_SIZE is a byte count");
    }
    if on_fpga() {
        64 * 1024
    } else {
        4096
    }
}

/// The package the agent serves: one firmware component for the device the
/// firmware reports, with the same identifier and a comparison stamp above the
/// device's, as `pldm_fd`'s `MockFdOps` advertises.
fn manifest(size: usize) -> FirmwareManifest {
    let image: Vec<u8> = (0..size).map(|i| (i as u8).wrapping_mul(7)).collect();
    FirmwareManifest {
        package_header_information: PackageHeaderInformation {
            package_header_identifier: Uuid::parse_str("7B291C996DB64208801B02026E463C78").unwrap(),
            package_header_format_revision: 1,
            package_release_date_time: Utc.with_ymd_and_hms(2025, 3, 1, 0, 0, 0).unwrap(),
            package_version_string_type: StringType::Utf8,
            package_version_string: Some("1.2.0-release".to_string()),
            package_header_size: 0, // computed when encoding
        },
        firmware_device_id_records: vec![FirmwareDeviceIdRecord {
            firmware_device_package_data: None,
            device_update_option_flags: 0x0,
            component_image_set_version_string_type: StringType::Utf8,
            component_image_set_version_string: Some("1.2.0".to_string()),
            applicable_components: Some(vec![0]),
            initial_descriptor: Descriptor {
                descriptor_type: DescriptorType::Uuid,
                descriptor_data: DEVICE_UUID.to_vec(),
            },
            additional_descriptors: None,
            reference_manifest_data: None,
        }],
        downstream_device_id_records: None,
        component_image_information: vec![ComponentImageInformation {
            classification: ComponentClassification::Firmware as u16,
            identifier: 0xffff,
            comparison_stamp: Some(0xffff_ffff),
            options: 0x0,
            requested_activation_method: 0x0002,
            version_string_type: StringType::Utf8,
            version_string: Some("soc-fw-1.2".to_string()),
            size: size as u32,
            image_data: Some(image),
            ..Default::default()
        }],
    }
}

#[test]
fn pldm_throughput_native_host_test() {
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

    let mut log = String::new();

    // The FD must be listening before the first command arrives.
    wait_for_log(&mut hw, &mut log, READY);

    let size = image_size();
    let socket = MctpTransport::new(port, target_addr.into())
        .create_socket(EndpointId(UA_EID), EndpointId(FD_EID))
        .expect("failed to connect to the I3C socket");
    let mut daemon = PldmDaemon::run(
        socket,
        Options {
            caliptra_mcu_pldm_fw_pkg: Some(manifest(size)),
            discovery_sm_actions: discovery_sm::DefaultActions {},
            update_sm_actions: update_sm::DefaultActions {},
            fd_tid: 0x01,
            rerun_count: 0,
        },
    )
    .expect("failed to start the PLDM daemon");

    // The agent's state is watched from a thread of its own, not from the
    // stepping loop: the daemon holds the state machine's lock while its first
    // send waits for emulator ticks, and ticks only advance while this thread
    // steps the model, so taking that lock here would deadlock the two.
    let finished = Arc::new(AtomicBool::new(false));
    let abort = Arc::new(AtomicBool::new(false));
    let monitor = {
        let (finished, abort) = (finished.clone(), abort.clone());
        spawn_with_emulator_state(move || {
            let started = Instant::now();
            let mut download_began: Option<Instant> = None;
            let mut transfer: Option<Duration> = None;
            let state = loop {
                let state = daemon.get_update_sm_state();
                match state {
                    update_sm::States::Download => {
                        download_began.get_or_insert_with(Instant::now);
                    }
                    update_sm::States::Verify
                    | update_sm::States::Apply
                    | update_sm::States::Activate
                    | update_sm::States::Done => {
                        if transfer.is_none() {
                            transfer = download_began.map(|t| t.elapsed());
                        }
                    }
                    _ => {}
                }
                if state == update_sm::States::Done
                    || abort.load(Ordering::Relaxed)
                    || started.elapsed() > UPDATE_TIMEOUT
                {
                    break state;
                }
                std::thread::sleep(Duration::from_millis(2));
            };
            if state == update_sm::States::Done {
                daemon.stop();
            } else {
                // Stopping joins threads that may be waiting on ticks the model
                // no longer produces; the process ends right after.
                std::mem::forget(daemon);
            }
            finished.store(true, Ordering::Relaxed);
            (state, transfer)
        })
    };

    let started = Instant::now();
    hw.step_until(|m| {
        let out = m.output().take(usize::MAX);
        if !out.is_empty() {
            print!("{out}");
            log.push_str(&out);
        }
        finished.load(Ordering::Relaxed)
            // The firmware reports FAIL and exits when the update goes wrong.
            || log.contains("FAIL:")
            || m.mci_fw_fatal_error().is_some()
            || started.elapsed() > UPDATE_TIMEOUT
    });
    abort.store(true, Ordering::Relaxed);
    assert_eq!(hw.mci_fw_fatal_error(), None, "firmware hit a fatal error");
    let (final_state, transfer) = monitor.join().expect("the monitor thread panicked");
    assert_eq!(
        final_state,
        update_sm::States::Done,
        "the update did not complete (agent state {final_state:?}); see the firmware log"
    );

    let transfer = transfer.expect("the agent never left Download");
    let secs = transfer.as_secs_f64();
    let bps = size as f64 / secs;
    println!(
        "PLDM_THROUGHPUT: {size} bytes in {secs:.3} s ({bps:.1} B/s, {:.1} KB/s), native, {}",
        bps / 1000.0,
        if on_fpga() { "fpga" } else { "emulator" },
    );

    hw.step_until_exit_success()
        .expect("firmware did not exit with PASS");
}
