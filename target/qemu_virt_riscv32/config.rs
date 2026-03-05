// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]

use core::ops::Range;

pub use kernel_config::{
    ClintTimerConfigInterface, ExceptionMode, KernelConfigInterface, PlicConfigInterface,
    RiscVKernelConfigInterface,
};
use memory_config::{MemoryRegion, MemoryRegionType};

pub struct KernelConfig;

impl KernelConfigInterface for KernelConfig {
    const SYSTEM_CLOCK_HZ: u64 = 10_000_000;
}

impl RiscVKernelConfigInterface for KernelConfig {
    type Timer = TimerConfig;
    const MTIME_HZ: u64 = KernelConfig::SYSTEM_CLOCK_HZ;
    const PMP_ENTRIES: usize = 16;
    const PMP_USERSPACE_ENTRIES: Range<usize> = Range {
        start: 0usize,
        end: Self::PMP_ENTRIES,
    };
    const PMP_GRANULARITY: usize = 0;

    const KERNEL_MEMORY_REGIONS: &'static [MemoryRegion] = &[MemoryRegion::new(
        MemoryRegionType::ReadWriteExecutable,
        0x0000_0000,
        0xffff_fffc,
    )];

    fn get_exception_mode() -> ExceptionMode {
        ExceptionMode::Direct
    }
}

pub struct PlicConfig;

impl PlicConfigInterface for PlicConfig {
    const PLIC_BASE_ADDRESS: usize = 0x0c00_0000;
}

pub struct TimerConfig;

const TIMER_BASE: usize = 0x200_0000;

impl ClintTimerConfigInterface for TimerConfig {
    const MTIME_REGISTER: usize = TIMER_BASE + 0xbff8;
    const MTIMECMP_REGISTER: usize = TIMER_BASE + 0x4000;
}

pub struct Uart0Config;

impl kernel_uart::UartConfigInterface for Uart0Config {
    const BASE_ADDRESS: usize = 0x1000_0000;
    const IRQ: u32 = 10;
}
