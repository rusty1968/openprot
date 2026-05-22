# Writing Peripheral Drivers with ast1060-pac

A practical guide based on the UART peripheral driver implementation.

## Overview

The AST1060 PAC (Peripheral Access Crate) provides type-safe register access to hardware peripherals. This guide shows how to write a peripheral driver that:
- Wraps PAC register access in safe abstractions
- Implements standard embedded-io traits
- Handles interrupts safely
- Provides non-blocking APIs

See [target/ast10x0/peripherals/uart/mod.rs](target/ast10x0/peripherals/uart/mod.rs) for the working UART example.

## Project Structure

```rust
pub struct Usart {
    usart: *const device::uart::RegisterBlock,
    _not_sync: PhantomData<UnsafeCell<()>>,
}
```

**Key elements:**
- Store a raw pointer to the PAC register block (`*const device::uart::RegisterBlock`)
- Add `PhantomData<UnsafeCell<()>>` to prevent `Sync` (and thus prevent `Send` across thread boundaries safely)
- Keep the struct small; PAC provides all register definitions

## Error Handling

Define a simple, flat error enum:

```rust
#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub enum Error {
    Frame,
    Parity,
    Noise,
    BufFull,
}
```

Implement traits so errors work with both blocking and non-blocking APIs:

```rust
impl embedded_io::Error for Error {
    fn kind(&self) -> embedded_io::ErrorKind {
        embedded_io::ErrorKind::Other
    }
}

impl serial_nb::Error for Error {
    fn kind(&self) -> serial_nb::ErrorKind {
        match self {
            Error::Frame => serial_nb::ErrorKind::FrameFormat,
            Error::Parity => serial_nb::ErrorKind::Parity,
            Error::Noise => serial_nb::ErrorKind::Noise,
            Error::BufFull => serial_nb::ErrorKind::Overrun,
        }
    }
}
```

## Register Access Pattern

**Always access registers through a single inline method:**

```rust
#[inline]
fn regs(&self) -> &device::uart::RegisterBlock {
    // SAFETY: Usart construction is unsafe, so caller upholds
    // pointer validity, non-nullness, and ownership requirements.
    unsafe { &*self.usart }
}
```

This single unsafe block is the entire safety perimeter. All other methods call `regs()` and are safe.

## Critical Pattern: Read-Once Status Registers

When reading a hardware status register that is cleared on read or whose bits must correspond to the same snapshot:

```rust
#[inline]
fn try_read_byte(&self) -> nb::Result<u8, Error> {
    // Read LSR exactly once per byte so error bits and DR correspond
    // to the same FIFO state.
    let lsr = self.regs().uartlsr().read();
    
    if !lsr.dr().bit() {
        return Err(nb::Error::WouldBlock);
    }

    let byte = self.regs().uartrbr().read().bits() as u8;
    
    if lsr.fe().bit_is_set() {
        Err(nb::Error::Other(Error::Frame))
    } else if lsr.pe().bit_is_set() {
        Err(nb::Error::Other(Error::Parity))
    } else {
        Ok(byte)
    }
}
```

**Why this matters:**
- Read LSR once and bind to local variable
- Check data-ready bit from that snapshot
- Only then read the data register
- Prevents TOCTOU (time-of-check-time-of-use) bugs where error bits disappear between checks

## Non-Blocking API: `nb::Result`

Use `nb::Result<T, E>` to distinguish between:
- `WouldBlock` — retryable (e.g., hardware not ready)
- `Other(E)` — terminal error (e.g., parity error, frame error)

```rust
impl serial_nb::Read<u8> for Usart {
    fn read(&mut self) -> nb::Result<u8, Self::Error> {
        self.try_read_byte()
    }
}

impl serial_nb::Write<u8> for Usart {
    fn write(&mut self, word: u8) -> nb::Result<(), Self::Error> {
        if !self.is_tx_full() {
            self.regs().uartthr().write(|w| unsafe { w.bits(word as u32) });
            Ok(())
        } else {
            Err(nb::Error::WouldBlock)
        }
    }
}
```

## Blocking API: `embedded_io` Traits

Wrap non-blocking operations for blocking behavior:

```rust
impl Read for Usart {
    fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        if out.is_empty() {
            return Ok(0);
        }

        let mut count = 0;
        
        // Block until at least one byte is available
        while count == 0 {
            match self.try_read_byte() {
                Ok(byte) => {
                    out[count] = byte;
                    count += 1;
                }
                Err(nb::Error::WouldBlock) => continue,
                Err(nb::Error::Other(e)) => return Err(e),
            }
        }

        // Then drain what is immediately available
        while count < out.len() {
            match self.try_read_byte() {
                Ok(byte) => {
                    out[count] = byte;
                    count += 1;
                }
                Err(nb::Error::WouldBlock) => break,
                Err(nb::Error::Other(e)) => return Err(e),
            }
        }

        Ok(count)
    }
}
```

**Pattern:** Guarantee at least one byte (blocking loop), then drain the rest (non-blocking).

## Construction: Unsafe + Safe Builder

Make the constructor `unsafe` to force callers to think about ownership:

```rust
pub unsafe fn new(usart: *const device::uart::RegisterBlock) -> Self {
    let this = Self {
        usart,
        _not_sync: PhantomData,
    };

    // Configure hardware immediately
    unsafe {
        this.regs().uartfcr().write(|w| {
            w.enbl_uartfifo().set_bit();
            w.rx_fiforst().set_bit();
            w.tx_fiforst().set_bit();
            w.define_the_rxr_fifointtrigger_level().bits(0b10)
        });
    }

    // Chain safe builder methods
    this.set_rate(Rate::MBaud1_5)
        .set_8n1()
        .interrupt_enable()
}
```

**Safety comment should document:**
- What the pointer represents
- Lifetime requirements
- Caller's responsibility for exclusive access

## Builder Methods for Configuration

Chain builder methods to configure the device:

```rust
pub fn set_rate(self, rate: Rate) -> Self {
    // Enable DLAB to access divisor latch
    self.regs().uartlcr().modify(|_, w| w.dlab().set_bit());

    match rate {
        Rate::Baud9600 => {
            self.regs().uartdlh().write(|w| unsafe { w.bits(0) });
            self.regs().uartdll().write(|w| unsafe { w.bits(12) });
        }
        // ... other rates
    }

    // Disable DLAB to access other registers
    self.regs().uartlcr().modify(|_, w| w.dlab().clear_bit());

    self
}

pub fn interrupt_enable(self) -> Self {
    self.regs().uartier().write(|w| {
        w.erbfi().set_bit();  // RX available
        w.etbei().set_bit();  // TX empty
        w.elsi().set_bit();   // RX line status (errors)
        w.edssi().set_bit();  // Modem status
        w
    });
    self
}
```

## Interrupt Handling

Provide enums and methods for decoding hardware interrupts:

```rust
#[derive(Debug)]
pub enum InterruptDecoding {
    ModemStatusChange = 0,
    TxEmpty = 1,
    RxDataAvailable = 2,
    LineStatusChange = 3,
    CharacterTimeout = 6,
    Unknown = -1,
}

impl TryFrom<u8> for InterruptDecoding {
    type Error = ();
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value & 0x07 {
            0 => Ok(InterruptDecoding::ModemStatusChange),
            1 => Ok(InterruptDecoding::TxEmpty),
            // ... etc
            _ => Err(()),
        }
    }
}

impl Usart {
    pub fn read_interrupt_status(&self) -> InterruptDecoding {
        InterruptDecoding::try_from(
            self.regs().uartiir().read().intdecoding_table().bits() & 0x07,
        )
        .unwrap_or(InterruptDecoding::Unknown)
    }
}
```

## Status Queries

Provide convenient status methods:

```rust
pub fn is_tx_full(&self) -> bool {
    !self.regs().uartlsr().read().thre().bit()
}

pub fn is_tx_idle(&self) -> bool {
    self.regs().uartlsr().read().txter_empty().bit_is_set()
}

pub fn is_rx_empty(&self) -> bool {
    !self.regs().uartlsr().read().dr().bit()
}

pub fn read_line_status(&self) -> LineStatus {
    let status = self.regs().uartlsr().read().bits() as u8;
    LineStatus::from_bits_truncate(status)
}
```

Use bitflags! for complex status registers:

```rust
bitflags! {
    #[derive(Debug)]
    pub struct LineStatus: u8 {
        const DataReady = 0x01;
        const OverrunError = 0x02;
        const ParityError = 0x04;
        const FramingError = 0x08;
        const BreakInterrupt = 0x10;
        const TransmitterHoldingRegisterEmpty = 0x20;
        const TransmitterEmpty = 0x40;
        const ErrorInReceiverFifo = 0x80;
    }
}
```

## Common Pitfalls

### 1. **Multiple Status Register Reads**
❌ **Bad:** Reading status register multiple times can give inconsistent snapshots
```rust
if self.regs().uartlsr().read().dr().bit() {
    // Between these two reads, status might have changed
    let byte = self.regs().uartrbr().read().bits() as u8;
}
```

✅ **Good:** Read once, use the snapshot
```rust
let lsr = self.regs().uartlsr().read();
if lsr.dr().bit() {
    let byte = self.regs().uartrbr().read().bits() as u8;
}
```

### 2. **Forgetting to Clear/Single Register Flags**
Some status bits are cleared on read (CSR — clear on status read). Don't re-read if you need the same bits.

### 3. **Mixing Blocking and Non-Blocking in Unsafe Code**
Don't spin-wait in unsafe blocks:
```rust
// DON'T: Unsafe busy-wait
unsafe {
    while !self.regs().uartlsr().read().thre().bit() { }
}
```

Instead, use the safe wrapper:
```rust
// DO: Safe method that can be tested
pub fn flush(&mut self) -> Result<(), Error> {
    while !self.is_tx_idle() {}
    Ok(())
}
```

### 4. **PAC `write()` vs `modify()`**
- `write()` — overwrites entire register; unsafe for shared fields
- `modify()` — read-modify-write; use unless you control all fields

✅ Correct:
```rust
self.regs().uartier().modify(|_, w| w.erbfi().set_bit());
```

### 5. **Unchecked unsafe { w.bits(...) } Calls**
When the PAC forces `unsafe { w.bits(...) }`, document why it's safe:

```rust
// Safe: UART is configured for 8-bit mode; byte value always fits
self.regs().uartthr().write(|w| unsafe { w.bits(byte as u32) });
```

### 6. **DLAB Side Effects**
If a peripheral uses DLAB (divisor latch access bit) to multiplex registers, track it:

```rust
pub fn set_rate(self, rate: Rate) -> Self {
    self.regs().uartlcr().modify(|_, w| w.dlab().set_bit());
    // ... configure divisor ...
    self.regs().uartlcr().modify(|_, w| w.dlab().clear_bit());
    self
}
```

## Testing Without Hardware

Use embedded_io traits to implement a mock:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    struct MockUart {
        rx_data: &'static [u8],
        rx_index: usize,
    }

    impl MockUart {
        fn new(data: &'static [u8]) -> Self {
            Self { rx_data: data, rx_index: 0 }
        }
    }

    impl embedded_io::ErrorType for MockUart {
        type Error = Error;
    }

    impl embedded_io::Read for MockUart {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
            if self.rx_index >= self.rx_data.len() {
                return Ok(0);
            }
            let to_copy = (self.rx_data.len() - self.rx_index).min(buf.len());
            buf[..to_copy].copy_from_slice(&self.rx_data[self.rx_index..self.rx_index + to_copy]);
            self.rx_index += to_copy;
            Ok(to_copy)
        }
    }
}
```

## Safety Checklist

- [ ] Constructor is `unsafe` with documented safety requirements
- [ ] All unsafe blocks have safety comments explaining why the operation is safe
- [ ] Register access goes through `regs()` method (single unsafe point)
- [ ] `PhantomData<UnsafeCell<()>>` prevents `Sync` implies of shared references
- [ ] Status register bits read once and stored locally before checking related bits
- [ ] Non-blocking methods use `nb::Result` to distinguish `WouldBlock` from terminal errors
- [ ] No `panic!`, `unwrap()`, or unchecked indexing
- [ ] Blocking operations that loop (busy-wait) are safe methods, not inline `unsafe`
- [ ] PAC wrapper documentations explain which bits are cleared-on-read
- [ ] FIFO/buffer saturation returns error, doesn't panic

## Integration Points

1. **Trait Implementations**
   - `embedded_io::{Read, Write}` for blocking I/O
   - `embedded_hal_nb::serial::{Read, Write}` for non-blocking I/O
   - `ErrorType` and `Error` trait impls for the error enum

2. **Interrupt Dispatch**
   - Call `read_interrupt_status()` in IRQ handler to determine which interrupt occurred
   - Non-blocking APIs (`try_read_byte()`, etc.) can be called from IRQ context
   - Use `nb::Result` to distinguish which errors are retryable from ISR

3. **FIFO Draining**
   - Provide `try_read_available()` for non-blocking drain during interrupt service
   - Distinguish terminal errors (parity, frame) from stalling (no data available)

4. **Queueing Integration**
   - For interrupt-driven services (like the USART server), arm interrupts with `set_rx_data_available_interrupt()`
   - Disable when queue is full with `clear_rx_data_available_interrupt()`
   - Re-enable once queue has space

## Key Takeaways

1. **Minimize unsafe surface:** Use unsafe sparingly; consolidate in `regs()` method
2. **Status atomicity:** Always read volatile registers once and reuse the snapshot
3. **Non-blocking first:** Implement `nb::Result` API; wrap for blocking behavior
4. **Classify errors:** Distinguish terminal failures from retry-able conditions
5. **Builder pattern:** Chain configuration methods; apply hardware config in `new()`
6. **Trait coverage:** Implement `embedded_io` and `embedded_hal_nb` for compatibility
7. **Hardware knowledge:** Understand FIFO behavior, interrupt semantics, register side effects
