// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Interrupt-driven I3C RX test.
//!
//! Unlike the polling smoke test, main never touches the I3C status
//! registers to discover a frame: the TTI RX-descriptor interrupt (IRQ 2 on
//! the VeeR PIC, wired to `i3c_interrupt_handler` via system.json5) is the
//! only path from frame arrival to reception. The handler masks the RX
//! interrupt enable (the IRQ is level-triggered) and sets a flag; main
//! drains the frame and exits 0 on the expected payload, 1 on a mismatch,
//! and 2 on timeout.

#![no_std]
#![no_main]

use caliptra_i3c_target::CaliptraI3cTarget;
use core::sync::atomic::{AtomicBool, Ordering};
use entry::exit;
use target_common::{declare_target, TargetInterface};
use console_backend as _;

// The declare_kernel_interrupt_handlers! expansion below imports
// kernel::interrupt_controller::InterruptController at module scope, which
// is what makes the enable_interrupt call in main resolve.

pub struct Target {}

// Emits PW_KERNEL_INTERRUPT_TABLE from the interrupt_table in system.json5.
codegen::declare_kernel_interrupt_handlers!();

/// I3C IRQ number on the emulated VeeR PIC; must match system.json5.
const I3C_IRQ: u32 = 2;

// AtomicBool with store/load only: riscv32imc has no atomic RMW instructions.
static RX_EVENT: AtomicBool = AtomicBool::new(false);

/// Program the VeeR external-interrupt redirect table (MEIVT).
///
/// The emulated VeeR core delivers an external interrupt by jumping to the
/// address stored at `MEIVT[irq]` (fast redirect), and requires the table to
/// live in DCCM. pw_kernel never programs MEIVT, so the first external
/// interrupt otherwise escalates to a "table not in DCCM" NMI whose vector
/// is also unprogrammed, and the CPU jumps to address 0 (the terminal
/// mcause=1/epc=0 exception previously seen with enable_rx_interrupt).
///
/// Pointing every entry at the kernel's standard trap vector (mtvec base)
/// makes the redirect behave exactly like an mtvec-vectored trap; the
/// kernel's PIC dispatch then reads the claim id from MEIHAP as usual.
fn init_meivt() {
    const DCCM_BASE: u32 = 0x5000_0000;
    const MAX_IRQ: u32 = 32;
    let mtvec: u32;
    // SAFETY: reading mtvec has no side effects.
    unsafe {
        core::arch::asm!("csrr {}, mtvec", out(reg) mtvec);
    }
    let trap_vector = mtvec & !0x3;
    for irq in 0..MAX_IRQ {
        // SAFETY: DCCM is dedicated data RAM, unused by this system image.
        unsafe {
            core::ptr::write_volatile((DCCM_BASE + irq * 4) as *mut u32, trap_vector);
        }
    }
    // SAFETY: MEIVT (VeeR-specific CSR 0xBC8) points the redirect table at
    // the block initialized above.
    unsafe {
        core::arch::asm!("csrw 0xbc8, {}", in(reg) DCCM_BASE);
    }
}

/// Referenced by name from system.json5; the generated wrapper calls this
/// with the concrete arch inside an interrupt guard.
pub fn i3c_interrupt_handler<K: kernel::Kernel>(_kernel: K) {
    // The RxDescStat IRQ is level-triggered (asserted while enable & status
    // are both set), so mask the enable here to deassert it; main drains
    // the descriptor afterwards.
    //
    // SAFETY: main only spins on RX_EVENT while the interrupt is enabled,
    // so this ephemeral handle cannot race main's register accesses.
    let mut i3c = unsafe { CaliptraI3cTarget::new() };
    i3c.disable_rx_interrupt();
    RX_EVENT.store(true, Ordering::SeqCst);
}

impl TargetInterface for Target {
    const NAME: &'static str = "Caliptra I3C IRQ Test";

    fn main() -> ! {
        // SAFETY: single call at boot; Caliptra ROM has already initialized
        // the I3C core and we are the only owner of the peripheral (the
        // interrupt handler's handle is sequenced by RX_EVENTS, see above).
        init_meivt();
        let mut i3c = unsafe { CaliptraI3cTarget::new() };
        i3c.enable_rx_interrupt();
        <arch_riscv::Arch as kernel::Arch>::InterruptController::enable_interrupt(I3C_IRQ);
        pw_log::info!("I3C IRQ test: waiting for private write");

        const MAX_POLLS: u32 = 10_000_000;
        let mut polls = 0u32;
        while !RX_EVENT.load(Ordering::SeqCst) {
            polls += 1;
            if polls >= MAX_POLLS {
                pw_log::info!("I3C IRQ test: timed out waiting for interrupt");
                exit(2);
            }
            core::hint::spin_loop();
        }
        pw_log::info!("I3C IRQ trace: interrupt observed");

        let mut buf = [0u8; 64];
        match i3c.rx_read(&mut buf) {
            Some(len) if len >= 4 && buf[..4] == [0x01, 0x02, 0x03, 0x04] => {
                pw_log::info!("I3C IRQ test: received expected payload OK");
                exit(0);
            }
            Some(len) => {
                pw_log::info!("I3C IRQ test: unexpected payload len={}", len as u32);
                exit(1);
            }
            None => {
                pw_log::info!("I3C IRQ test: interrupt fired but no descriptor");
                exit(1);
            }
        }
    }

    fn shutdown(code: u32) -> ! {
        match code {
            0 => pw_log::info!("PASS"),
            _ => pw_log::info!("FAIL: {}", code),
        }
        exit(code);
    }
}

declare_target!(Target);
