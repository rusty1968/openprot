// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

pub mod registers;

pub use registers::SmbMbox;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailboxRegError {
	OutOfRange,
}

/// Thin byte-window wrapper over the SMB mailbox register array.
pub struct Mailbox<'a, TMmio: ureg::Mmio + core::borrow::Borrow<TMmio>> {
	bytes: ureg::Array<256, ureg::RegRef<registers::meta::MailboxByte, &'a TMmio>>,
}

impl<'a, TMmio: ureg::Mmio + core::borrow::Borrow<TMmio>> Mailbox<'a, TMmio> {
	#[inline(always)]
	pub fn from_regs(regs: &'a registers::RegisterBlock<TMmio>) -> Self {
		Self {
			bytes: regs.mailbox(),
		}
	}

	#[inline(always)]
	pub fn read_byte(&self, idx: u8) -> Result<u8, MailboxRegError> {
		let i = idx as usize;
		self
			.bytes
			.get(i)
			.ok_or(MailboxRegError::OutOfRange)
			.map(|r| r.read())
	}

}

impl<'a, TMmio: ureg::MmioMut + core::borrow::Borrow<TMmio>> Mailbox<'a, TMmio> {

	#[inline(always)]
	pub fn write_byte(&self, idx: u8, val: u8) -> Result<(), MailboxRegError> {
		let i = idx as usize;
		self
			.bytes
			.get(i)
			.ok_or(MailboxRegError::OutOfRange)
			.map(|r| r.write(|_| val))
	}
}
