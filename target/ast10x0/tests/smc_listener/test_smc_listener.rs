// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]
#![no_main]

use pw_status::Result;
use test_smc_listener_codegen::{handle, signals};
use userspace::entry;
use userspace::syscall;
use userspace::time::Instant;

// FMC008 status bit layout surfaced by the SMC HAL.
const FMC008_DMA_COMPLETE_BIT: u8 = 11;
const FMC008_DMA_ERROR_BIT: u8 = 10;
const FMC008_WRITE_PROTECT_BIT: u8 = 9;

// Compact IPC payload for the companion app.
// [0] version
// [1] dma_complete_bit_index
// [2] dma_error_bit_index
// [3] write_protect_bit_index
// [4] listener-observed interrupt flags (bit0 = fmc irq seen)
const PAYLOAD_VERSION: u8 = 1;
const OBSERVED_FMC_IRQ_SEEN: u8 = 1 << 0;

fn run() -> Result<()> {
    // Mimic UART-listener pattern: wait on interrupt object, ack if observed,
    // then report a payload to the server process over IPC.
    let mut observed = 0u8;
    if let Ok(wait_return) = syscall::object_wait(
        handle::SMC_INTERRUPTS,
        signals::FMC_IRQ,
        Instant::from_ticks(1_000),
    ) {
        if wait_return.pending_signals.contains(signals::FMC_IRQ) && wait_return.user_data == 0 {
            observed |= OBSERVED_FMC_IRQ_SEEN;
            let _ = syscall::interrupt_ack(handle::SMC_INTERRUPTS, wait_return.pending_signals);
        }
    }

    const RECV_LEN: usize = 0;
    let send_buf = [
        PAYLOAD_VERSION,
        FMC008_DMA_COMPLETE_BIT,
        FMC008_DMA_ERROR_BIT,
        FMC008_WRITE_PROTECT_BIT,
        observed,
    ];
    let mut recv_buf = [0u8; RECV_LEN];

    let len = syscall::channel_transact(handle::IPC, &send_buf, &mut recv_buf, Instant::MAX)?;
    if len != RECV_LEN {
        return Err(pw_status::Error::OutOfRange);
    }

    Ok(())
}

#[entry]
fn entry() -> ! {
    // Only the server app triggers test shutdown.
    let _ = run();
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
