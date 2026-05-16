// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! AST10x0 HACE low-level register access.

use core::marker::PhantomData;

use ast1060_pac as device;

/// Safe wrapper around the AST10x0 HACE register block.
#[derive(Copy, Clone)]
pub struct HaceRegisters {
    ptr: *mut device::hace::RegisterBlock,
    _not_send: PhantomData<*mut ()>,
}

impl HaceRegisters {
    /// Create a register accessor from a raw HACE register block pointer.
    ///
    /// # Safety
    /// Caller must ensure:
    /// - `base` points to a valid HACE register block.
    /// - access to the HACE instance is serialized appropriately.
    pub const unsafe fn new(base: *const device::hace::RegisterBlock) -> Self {
        Self {
            ptr: base as *mut device::hace::RegisterBlock,
            _not_send: PhantomData,
        }
    }

    /// Create a register accessor for the global HACE instance.
    ///
    /// # Safety
    /// Caller must ensure access to the singleton HACE is coordinated.
    pub const unsafe fn new_global() -> Self {
        // SAFETY: Caller upholds the singleton access contract.
        unsafe { Self::new(device::Hace::ptr()) }
    }

    #[inline]
    pub(crate) fn regs(&self) -> &device::hace::RegisterBlock {
        // SAFETY: Constructor guarantees a valid HACE register block pointer.
        unsafe { &*self.ptr }
    }

    #[inline]
    pub(crate) fn clear_hash_intflag(&self) {
        self.regs().hace1c().write(|w| w.hash_intflag().set_bit());
    }

    #[inline]
    pub(crate) fn hash_intflag_is_set(&self) -> bool {
        self.regs().hace1c().read().hash_intflag().bit_is_set()
    }

    #[inline]
    pub(crate) fn program_hash_operation(
        &self,
        src_addr: u32,
        digest_addr: u32,
        data_len: u32,
        cmd: u32,
    ) {
        // SAFETY: Callers provide HACE-usable physical addresses and a valid command.
        self.regs().hace20().write(|w| unsafe { w.bits(src_addr) });
        self.regs().hace24().write(|w| unsafe { w.bits(digest_addr) });
        self.regs().hace28().write(|w| unsafe { w.bits(digest_addr) });
        self.regs().hace2c().write(|w| unsafe { w.bits(data_len) });
        self.regs().hace30().write(|w| unsafe { w.bits(cmd) });
    }

    #[inline]
    pub(crate) fn stop_hash_operation(&self) {
        // SAFETY: Writing 0 to command register is the defined idle/stop state.
        self.regs().hace30().write(|w| unsafe { w.bits(0) });
    }
}
