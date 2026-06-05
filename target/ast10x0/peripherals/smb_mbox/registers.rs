// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#[doc = r" A zero-sized type that represents ownership of the SWMBX peripheral."]
pub struct SmbMbox {
	_priv: (),
}

impl SmbMbox {
	pub const PTR: *mut u8 = 0x7e7b_0e00 as *mut u8;

	#[doc = r" # Safety"]
	#[doc = r""]
	#[doc = r" Caller must ensure singleton-style ownership of this peripheral."]
	#[inline(always)]
	pub const unsafe fn new() -> Self {
		Self { _priv: () }
	}

	#[doc = r" Returns a register block that can read SWMBX registers."]
	#[inline(always)]
	pub fn regs(&self) -> RegisterBlock<ureg::RealMmio<'_>> {
		RegisterBlock {
			ptr: Self::PTR,
			mmio: core::default::Default::default(),
		}
	}

	#[doc = r" Returns a register block that can read and write SWMBX registers."]
	#[inline(always)]
	pub fn regs_mut(&mut self) -> RegisterBlock<ureg::RealMmioMut<'_>> {
		RegisterBlock {
			ptr: Self::PTR,
			mmio: core::default::Default::default(),
		}
	}
}

#[derive(Clone, Copy)]
pub struct RegisterBlock<TMmio: ureg::Mmio + core::borrow::Borrow<TMmio>> {
	ptr: *mut u8,
	mmio: TMmio,
}

impl<TMmio: ureg::Mmio + core::default::Default> RegisterBlock<TMmio> {
	#[doc = r" # Safety"]
	#[doc = r""]
	#[doc = r" Caller must ensure ptr is valid for SWMBX register accesses."]
	#[inline(always)]
	pub unsafe fn new(ptr: *mut u8) -> Self {
		Self {
			ptr,
			mmio: core::default::Default::default(),
		}
	}
}

impl<TMmio: ureg::Mmio> RegisterBlock<TMmio> {
	#[doc = r" # Safety"]
	#[doc = r""]
	#[doc = r" Caller must ensure ptr is valid for SWMBX register accesses."]
	#[inline(always)]
	pub unsafe fn new_with_mmio(ptr: *mut u8, mmio: TMmio) -> Self {
		Self { ptr, mmio }
	}

	#[doc = "Raw mailbox byte accessor over offsets 0x00..=0xFF."]
	#[inline(always)]
	pub fn mailbox(&self) -> ureg::Array<256, ureg::RegRef<meta::MailboxByte, &TMmio>> {
		unsafe {
			ureg::Array::new_with_mmio(
				self.ptr.wrapping_add(0x00),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}

	#[inline(always)]
	pub fn cpld_identifier(&self) -> ureg::RegRef<meta::MailboxByte, &TMmio> {
		unsafe {
			ureg::RegRef::new_with_mmio(
				self.ptr.wrapping_add(register_offsets::CPLD_IDENTIFIER as usize),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}

	#[inline(always)]
	pub fn cpld_release_version(&self) -> ureg::RegRef<meta::MailboxByte, &TMmio> {
		unsafe {
			ureg::RegRef::new_with_mmio(
				self.ptr
					.wrapping_add(register_offsets::CPLD_RELEASE_VERSION as usize),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}

	#[inline(always)]
	pub fn cpld_rot_svn(&self) -> ureg::RegRef<meta::MailboxByte, &TMmio> {
		unsafe {
			ureg::RegRef::new_with_mmio(
				self.ptr.wrapping_add(register_offsets::CPLD_ROT_SVN as usize),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}

	#[inline(always)]
	pub fn platform_state(&self) -> ureg::RegRef<meta::MailboxByte, &TMmio> {
		unsafe {
			ureg::RegRef::new_with_mmio(
				self.ptr.wrapping_add(register_offsets::PLATFORM_STATE as usize),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}

	#[inline(always)]
	pub fn ufm_status_value(&self) -> ureg::RegRef<meta::MailboxByte, &TMmio> {
		unsafe {
			ureg::RegRef::new_with_mmio(
				self.ptr.wrapping_add(register_offsets::UFM_STATUS_VALUE as usize),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}

	#[inline(always)]
	pub fn ufm_command(&self) -> ureg::RegRef<meta::MailboxByte, &TMmio> {
		unsafe {
			ureg::RegRef::new_with_mmio(
				self.ptr.wrapping_add(register_offsets::UFM_COMMAND as usize),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}

	#[inline(always)]
	pub fn ufm_cmd_trigger_value(&self) -> ureg::RegRef<meta::MailboxByte, &TMmio> {
		unsafe {
			ureg::RegRef::new_with_mmio(
				self.ptr
					.wrapping_add(register_offsets::UFM_CMD_TRIGGER_VALUE as usize),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}

	#[inline(always)]
	pub fn ufm_write_fifo(&self) -> ureg::RegRef<meta::MailboxByte, &TMmio> {
		unsafe {
			ureg::RegRef::new_with_mmio(
				self.ptr.wrapping_add(register_offsets::UFM_WRITE_FIFO as usize),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}

	#[inline(always)]
	pub fn ufm_read_fifo(&self) -> ureg::RegRef<meta::MailboxByte, &TMmio> {
		unsafe {
			ureg::RegRef::new_with_mmio(
				self.ptr.wrapping_add(register_offsets::UFM_READ_FIFO as usize),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}

	#[inline(always)]
	pub fn acm_checkpoint(&self) -> ureg::RegRef<meta::MailboxByte, &TMmio> {
		unsafe {
			ureg::RegRef::new_with_mmio(
				self.ptr.wrapping_add(register_offsets::ACM_CHECKPOINT as usize),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}

	#[inline(always)]
	pub fn bios_checkpoint(&self) -> ureg::RegRef<meta::MailboxByte, &TMmio> {
		unsafe {
			ureg::RegRef::new_with_mmio(
				self.ptr.wrapping_add(register_offsets::BIOS_CHECKPOINT as usize),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}

	#[inline(always)]
	pub fn pch_update_intent(&self) -> ureg::RegRef<meta::MailboxByte, &TMmio> {
		unsafe {
			ureg::RegRef::new_with_mmio(
				self.ptr
					.wrapping_add(register_offsets::PCH_UPDATE_INTENT as usize),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}

	#[inline(always)]
	pub fn bmc_update_intent(&self) -> ureg::RegRef<meta::MailboxByte, &TMmio> {
		unsafe {
			ureg::RegRef::new_with_mmio(
				self.ptr
					.wrapping_add(register_offsets::BMC_UPDATE_INTENT as usize),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}

	#[inline(always)]
	pub fn bmc_checkpoint(&self) -> ureg::RegRef<meta::MailboxByte, &TMmio> {
		unsafe {
			ureg::RegRef::new_with_mmio(
				self.ptr.wrapping_add(register_offsets::BMC_CHECKPOINT as usize),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}

	#[inline(always)]
	pub fn pch_update_intent2(&self) -> ureg::RegRef<meta::MailboxByte, &TMmio> {
		unsafe {
			ureg::RegRef::new_with_mmio(
				self.ptr
					.wrapping_add(register_offsets::PCH_UPDATE_INTENT2 as usize),
				core::borrow::Borrow::borrow(&self.mmio),
			)
		}
	}
}

pub mod register_offsets {
	pub const CPLD_IDENTIFIER: u8 = 0x00;
	pub const CPLD_RELEASE_VERSION: u8 = 0x01;
	pub const CPLD_ROT_SVN: u8 = 0x02;
	pub const PLATFORM_STATE: u8 = 0x03;
	pub const RECOVERY_COUNT: u8 = 0x04;
	pub const LAST_RECOVERY_REASON: u8 = 0x05;
	pub const PANIC_EVENT_COUNT: u8 = 0x06;
	pub const LAST_PANIC_REASON: u8 = 0x07;
	pub const MAJOR_ERROR_CODE: u8 = 0x08;
	pub const MINOR_ERROR_CODE: u8 = 0x09;
	pub const UFM_STATUS_VALUE: u8 = 0x0a;
	pub const UFM_COMMAND: u8 = 0x0b;
	pub const UFM_CMD_TRIGGER_VALUE: u8 = 0x0c;
	pub const UFM_WRITE_FIFO: u8 = 0x0d;
	pub const UFM_READ_FIFO: u8 = 0x0e;
	pub const MCTP_WRITE_FIFO: u8 = 0x0f;
	pub const ACM_CHECKPOINT: u8 = 0x10;
	pub const BIOS_CHECKPOINT: u8 = 0x11;
	pub const PCH_UPDATE_INTENT: u8 = 0x12;
	pub const BMC_UPDATE_INTENT: u8 = 0x13;
	pub const BMC_CHECKPOINT: u8 = 0x60;
	pub const PCH_UPDATE_INTENT2: u8 = 0x61;
}

pub mod meta {
	pub struct MailboxByte;

	impl ureg::RegType for MailboxByte {
		type Raw = u8;
	}

	impl ureg::ReadableReg for MailboxByte {
		type ReadVal = u8;
	}

	impl ureg::WritableReg for MailboxByte {
		type WriteVal = u8;
	}

	impl ureg::ResettableReg for MailboxByte {
		const RESET_VAL: Self::Raw = 0;
	}
}
