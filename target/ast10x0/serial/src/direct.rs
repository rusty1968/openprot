// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

use ast10x0_peripherals::uart::{Error as UartError, Rate, Usart};
use embedded_io::{ErrorType, Read, Write};
use openprot_mctp_transport_serial::{SerialError, SerialPort};

/// Direct HAL serial backend for AST10x0.
pub struct Ast10x0DirectSerial {
	uart: Usart,
}

impl Ast10x0DirectSerial {
	/// Create a direct serial backend using AST10x0 UART5.
	pub fn new_uart5() -> Self {
		// SAFETY: 0x7e78_4000 is UART5 MMIO base on AST10x0.
		let uart = unsafe { Usart::new(0x7e78_4000 as *const _) };
		// Keep RX notifications enabled; TX-empty IRQ is noisy for this runtime.
		uart.clear_tx_idle_interrupt();
		Self { uart }
	}

	/// Create a direct serial backend from an existing configured UART.
	pub const fn from_usart(uart: Usart) -> Self {
		Self { uart }
	}

	/// Enable RX data-available interrupts.
	pub fn enable_rx_data_available_interrupt(&self) {
		self.uart.set_rx_data_available_interrupt();
	}
}

impl ErrorType for Ast10x0DirectSerial {
	type Error = UartError;
}

impl Write for Ast10x0DirectSerial {
	fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
		self.uart.write(buf)
	}

	fn flush(&mut self) -> Result<(), Self::Error> {
		self.uart.flush()
	}
}

impl Read for Ast10x0DirectSerial {
	fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
		self.uart.read(buf)
	}
}

impl SerialPort for Ast10x0DirectSerial {
	fn configure(&mut self, baud_rate: u32) -> Result<(), SerialError> {
		let rate = match baud_rate {
			9_600 => Rate::Baud9600,
			19_200 => Rate::Baud19200,
			1_500_000 => Rate::MBaud1_5,
			_ => return Err(SerialError::Invalid),
		};

		// `set_rate` consumes and returns `Usart`; move out/in place.
		// SAFETY: we immediately write back a valid `Usart` before returning.
		let current = unsafe { core::ptr::read(&self.uart) };
		let updated = current.set_rate(rate);
		// SAFETY: `self.uart` is initialized again before any further use.
		unsafe { core::ptr::write(&mut self.uart, updated) };
		Ok(())
	}
}
