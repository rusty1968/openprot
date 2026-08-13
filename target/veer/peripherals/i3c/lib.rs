// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Caliptra-SS I3C target peripheral driver.
//!
//! Drives the i3c-core HCI TTI (Target Transaction Interface) on behalf of
//! firmware running on the VeeR RISC-V core.  The i3c-core is always a
//! *target* (secondary) from the VeeR perspective — a BMC controller on the
//! I3C bus initiates all transfers.
//!
//! The Caliptra ROM initializes the I3C core before handing off to application
//! firmware.  This driver assumes the ROM has already run and only manages the
//! data path (interrupt enable/disable, RX drain, TX queue, IBI).
//!
//! # TTI data path
//!
//! - **Incoming write** (controller → target): hardware pushes a descriptor
//!   into `tti_rx_desc_queue_port` then `data_length` words into
//!   `tti_rx_data_port`.  Poll `rx_pending()` or enable the RX interrupt.
//! - **Outgoing read** (target → controller): firmware writes a descriptor
//!   to `tti_tx_desc_queue_port` then `data_length` words to
//!   `tti_tx_data_port`.
//! - **IBI**: write the IBI descriptor (MDB + payload length) to
//!   `tti_tti_ibi_port` then the payload words; hardware raises the IBI on
//!   the bus.

#![no_std]

use caliptra_ss_registers::i3c;

use core::marker::PhantomData;
use tock_registers::interfaces::{ReadWriteable, Readable, Writeable};

/// MIPI DCR value for an MCTP endpoint.
pub const MCTP_DCR: u8 = 0xCC;

/// Driver for the Caliptra-SS i3c-core in target (secondary) mode.
pub struct CaliptraI3cTarget {
    regs: *const i3c::regs::I3c,
    // !Send + !Sync: exclusive ownership of one physical peripheral
    _not_send_sync: PhantomData<*mut ()>,
}

impl CaliptraI3cTarget {
    /// Construct the driver over the Caliptra-SS I3C peripheral.
    ///
    /// # Safety
    ///
    /// Caller must ensure exclusive access to the I3C peripheral for the
    /// lifetime of this value.
    pub const unsafe fn new() -> Self {
        Self {
            regs: i3c::I3C_CSR_ADDR as *const i3c::regs::I3c,
            _not_send_sync: PhantomData,
        }
    }

    #[inline]
    fn regs(&self) -> &i3c::regs::I3c {
        // SAFETY: `new` guarantees a valid address and exclusive access.
        unsafe { &*self.regs }
    }

    // -------------------------------------------------------------------------
    // Interrupts
    // -------------------------------------------------------------------------

    /// Enable the RX descriptor threshold interrupt.
    pub fn enable_rx_interrupt(&mut self) {
        self.regs()
            .tti_interrupt_enable
            .modify(i3c::bits::InterruptEnable::RxDescStatEn::SET);
    }

    /// Disable the RX descriptor threshold interrupt.
    pub fn disable_rx_interrupt(&mut self) {
        self.regs()
            .tti_interrupt_enable
            .modify(i3c::bits::InterruptEnable::RxDescStatEn::CLEAR);
    }

    // -------------------------------------------------------------------------
    // TTI receive (controller → target private write)
    // -------------------------------------------------------------------------

    /// Return `true` if an RX descriptor is waiting in the TTI queue.
    pub fn rx_pending(&self) -> bool {
        self.regs()
            .tti_interrupt_status
            .is_set(i3c::bits::InterruptStatus::RxDescStat)
    }

    /// Drain one incoming write into `buf`.  Returns the number of bytes read,
    /// or `None` if no descriptor was present.
    pub fn rx_read(&mut self, buf: &mut [u8]) -> Option<usize> {
        let regs = self.regs();
        if !regs
            .tti_interrupt_status
            .is_set(i3c::bits::InterruptStatus::RxDescStat)
        {
            return None;
        }

        let desc = regs.tti_rx_desc_queue_port.get();
        // Lower 16 bits of descriptor carry the data length in bytes.
        let len = (desc & 0xffff) as usize;
        let nwords = (len + 3) / 4;

        let mut i = 0usize;
        for _ in 0..nwords {
            let word = regs.tti_rx_data_port.get();
            for &b in &word.to_le_bytes() {
                if let Some(slot) = buf.get_mut(i) {
                    *slot = b;
                }
                i += 1;
            }
        }

        // W1C — clear the status bit.
        regs.tti_interrupt_status
            .write(i3c::bits::InterruptStatus::RxDescStat::SET);

        Some(len.min(buf.len()))
    }

    // -------------------------------------------------------------------------
    // TTI transmit (target → controller private read response)
    // -------------------------------------------------------------------------

    /// Queue `data` as the response to the next private-read from the controller.
    ///
    /// The descriptor must be written before the data words: the hardware
    /// (and the emulator model) opens a new TX buffer on the descriptor
    /// write and appends subsequent data-port writes to it, matching the
    /// upstream caliptra-mcu-sw runtime driver.
    pub fn tx_write(&mut self, data: &[u8]) {
        let regs = self.regs();
        // Descriptor: data_length in lower 16 bits; saturate rather than truncate.
        regs.tti_tx_desc_queue_port
            .set(u32::try_from(data.len()).unwrap_or(u16::MAX as u32));
        let mut chunks = data.chunks_exact(4);
        for chunk in &mut chunks {
            let word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            regs.tti_tx_data_port.set(word);
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            let mut tmp = [0u8; 4];
            tmp[..rem.len()].copy_from_slice(rem);
            regs.tti_tx_data_port.set(u32::from_le_bytes(tmp));
        }
    }

    // -------------------------------------------------------------------------
    // IBI (In-Band Interrupt — target → controller unsolicited notification)
    // -------------------------------------------------------------------------

    /// Raise an IBI with the given Mandatory Data Byte and optional payload.
    /// Payload must be ≤255 bytes; excess bytes are silently dropped.
    ///
    /// The descriptor word must be written before the payload words: the
    /// hardware (and the emulator model) parses the first word written to
    /// the IBI port as the descriptor and takes the payload length from it.
    pub fn ibi_raise(&mut self, mdb: u8, payload: &[u8]) {
        let payload = &payload[..payload.len().min(255)];
        let regs = self.regs();
        // IBI descriptor: MDB in bits [31:24], payload length in bits [7:0].
        let desc = ((mdb as u32) << 24) | (payload.len() as u32 & 0xff);
        regs.tti_tti_ibi_port.set(desc);
        let mut chunks = payload.chunks_exact(4);
        for chunk in &mut chunks {
            let word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            regs.tti_tti_ibi_port.set(word);
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            let mut tmp = [0u8; 4];
            tmp[..rem.len()].copy_from_slice(rem);
            regs.tti_tti_ibi_port.set(u32::from_le_bytes(tmp));
        }
    }
}
