# SMC Flash Command Transport Evidence Details

Date: 2026-05-02

## Purpose

This document dissects the evidence cited by
`SMC_FLASH_COMMAND_TRANSPORT_PLAN.md`.

The goal here is narrower than the plan itself:
- identify what the cited `aspeed-rust` code directly proves
- separate direct observations from design inferences
- call out where the evidence is strong, where it is only suggestive, and what
  remains unproven inside `reference`

## Scope Of The Evidence

The plan is built from implementation evidence in:
- `aspeed-rust/src/spi/fmccontroller.rs`
- `aspeed-rust/src/spi/util.rs`
- `aspeed-rust/src/spi/consts.rs`
- `aspeed-rust/src/spi/norflash.rs`

The strongest evidence is behavioral, not documentary: the `aspeed-rust`
controller code shows how an AST10x0 flash command transaction is actually
serialized in working software.

That means the evidence is useful for transport design, but it is not yet a
datasheet-backed register specification for the in-repo HAL.

## High-Level Conclusion

The cited implementation supports one central claim:

The AST10x0 command path is implemented by putting the controller into a
user-mode CS state and then treating the mapped flash aperture as the stream
port for opcode, address, dummy bytes, and payload.

This is the main reason the plan recommends a narrow raw transaction primitive
instead of introducing a second SPI engine abstraction.

## Evidence 1: Command Transport Uses The Flash Aperture

### Direct source

Primary code:
- `aspeed-rust/src/spi/fmccontroller.rs`, `spi_nor_transceive_user()`

Relevant behavior in that function:
- computes `start_ptr` from `self.spi_data.decode_addr[cs].start as *mut u32`
- enters user mode with `self.activate_user()`
- writes the opcode through `spi_write_data(start_ptr, &[opcode])`
- writes address bytes through `spi_write_data(start_ptr, bytes)`
- writes dummy bytes through `spi_write_data(start_ptr, &dummy[..dummy_len])`
- either reads data with `spi_read_data(start_ptr, op_info.rx_buf)` or writes
  data with `spi_write_data(start_ptr, op_info.tx_buf)`
- exits the transaction with `self.deactivate_user()`

### What this directly proves

This proves that, in the lifted implementation, the mapped flash window is used
as the byte-stream interface for explicit SPI NOR command transactions.

The transport is not modeled there as:
- a separate command FIFO API
- a dedicated packet descriptor structure at the register layer
- a firmware-emulated SPI shifter

Instead, the controller is switched into a user state and the aperture itself
is used for all byte phases.

### What this does not prove by itself

It does not prove that this is the only possible AST10x0 implementation.
It proves that this is a working implementation pattern in the nearest related
codebase in this workspace.

It also does not prove the exact hardware timing semantics beyond what the code
assumes. The hardware behavior is inferred from how the controller reacts to
volatile reads and writes while user mode is active.

### Why this matters to `reference`

This is the strongest evidence in the plan because it answers the core design
question: where do command bytes actually go?

The answer suggested by working code is: through the same mapped window already
used for normal flash reads, but under a different CS control state.

## Evidence 2: User Mode Is Entered And Exited By CS Control Register Values

### Direct source

Primary code:
- `aspeed-rust/src/spi/consts.rs`
- `ASPEED_SPI_USER = 0x3`
- `ASPEED_SPI_USER_INACTIVE = 0x4`
- `aspeed-rust/src/spi/fmccontroller.rs`, `activate_user()`
- `aspeed-rust/src/spi/fmccontroller.rs`, `deactivate_user()`

Observed register sequence:
- `activate_user()` writes `user_reg | ASPEED_SPI_USER_INACTIVE`
- then writes `user_reg`
- `deactivate_user()` writes `user_reg | ASPEED_SPI_USER_INACTIVE`
- then restores `self.spi_data.cmd_mode[cs].normal_read`

### What this directly proves

This proves that the lifted implementation treats user-mode transaction setup as
an ordered CS control sequence, not as a one-shot mode bit.

It also proves that the implementation distinguishes between:
- user-active state
- user-inactive state
- restored normal-read state

That distinction is important because the transaction path is not complete until
normal read mode is restored.

### What is inferred rather than proved

The plan infers that `reference` should preserve frequency and normal-read
configuration while changing only the mode bits needed for user transactions.

That inference is reasonable because `aspeed-rust` stores precomputed command
mode state in `self.spi_data.cmd_mode[cs]` and restores `normal_read` on exit,
but the exact field-level preservation logic is not shown as a minimal isolated
helper there.

In other words, the evidence proves the sequence shape more strongly than it
proves the exact mask/update recipe that `reference` should use.

### Why this matters to `reference`

The current `reference` implementation can adopt the state transition pattern
without importing the full `aspeed-rust` object model.

The minimum reusable lesson is:
- enter a user transaction with a CS control transition
- perform byte movement through the aperture
- leave user mode by restoring normal-read behavior

## Evidence 3: Byte Movement Is Plain Volatile Memory Access

### Direct source

Primary code:
- `aspeed-rust/src/spi/util.rs`, `spi_write_data()`
- `aspeed-rust/src/spi/util.rs`, `spi_read_data()`

Observed behavior:
- full aligned chunks are transferred as 32-bit little-endian words
- remaining tail bytes use byte-wide volatile access
- the helpers contain no flash-protocol policy
- the helpers contain no command interpretation logic

### What this directly proves

This proves that the transport data plane can stay extremely small.

At the transfer primitive level, the controller only needs:
- an aperture pointer
- a mode transition around the transaction
- volatile writes for command and TX phases
- volatile reads for RX phases

The helpers are generic byte movers. They do not know whether they are handling
opcode bytes, address bytes, dummy bytes, or payload bytes.

### What this does not prove

It does not prove that word-at-a-time optimization is required for correctness.
That is an implementation choice in the lifted code.

For `reference`, the direct lesson is not "copy the exact helper bodies". The
lesson is that the byte-stream transport does not require a richer protocol
object below the command-assembly layer.

### Why this matters to `reference`

This evidence supports keeping the controller API narrow. A controller helper
such as

```rust
fn transceive_user(&self, cmd: &[u8], tx_payload: &[u8], rx: &mut [u8])
    -> Result<(), SmcError>;
```

is consistent with the lifted transport model because the lower layer only has
to stream bytes in order.

## Evidence 4: SPI NOR Operations Already Match The Transport Shape

### Direct source

Primary code:
- `aspeed-rust/src/spi/norflash.rs`, `nor_write_enable()`
- `aspeed-rust/src/spi/norflash.rs`, `nor_sector_erase()`
- `aspeed-rust/src/spi/norflash.rs`, `nor_page_program()`
- `aspeed-rust/src/spi/norflash.rs`, `nor_wait_until_ready()`

Observed command construction:
- `nor_write_enable()` builds `opcode = WREN`, no address, no payload
- `nor_wait_until_ready()` builds `opcode = RDSR`, one-byte RX buffer,
  `DataDirection::DRead`
- `nor_sector_erase()` does `WREN`, then `SE` with a 3-byte address, then polls
  until ready
- `nor_page_program()` does `WREN`, then `PP` with a 3-byte address and TX data

### What this directly proves

This proves that the operations needed by the `reference` device layer already
fit the transport pattern observed in Evidence 1:
- command-only transaction
- command-plus-read transaction
- command-plus-address transaction
- command-plus-address-plus-write-payload transaction

It also proves that WIP polling via repeated `RDSR` is the control pattern used
in the lifted implementation.

### What this does not prove

It does not prove timeout behavior, error mapping, or policy boundaries for the
`reference` HAL. In `aspeed-rust`, `nor_wait_until_ready()` loops until ready
with a simple delay and no explicit timeout or richer error result.

So the plan is right to keep timeout and error semantics in the `reference`
device layer rather than cloning the lifted behavior exactly.

### Why this matters to `reference`

This evidence is what makes the proposed split defensible:
- device layer assembles flash commands and interprets status
- controller layer only performs the transport

The device API shape already matches the command shapes proven in the source
implementation.

## Evidence Strength Assessment

### Strong evidence

These claims are strongly supported by code:
- user-mode transactions in the lifted implementation stream bytes through the
  mapped flash aperture
- command, address, dummy, TX, and RX phases all use the same aperture pointer
- user mode involves an explicit active and inactive CS control sequence
- normal-read state is restored after the transaction
- the SPI NOR commands needed for status, erase, and page program already map
  onto the raw transport shape

### Moderate evidence

These claims are supported, but still include some design interpretation:
- `reference` should expose a narrow raw transaction helper instead of a larger
  SPI framework
- preserving existing normal-read configuration is the correct minimal restore
  behavior for the in-repo HAL
- the first implementation should stay CS0-only because current facades are
  currently oriented that way

### Weak or unproven areas

These points are not yet proven by the cited evidence alone:
- exact AST10x0 register field definitions for the `reference` HAL
- whether QEMU fully models command-side effects for all write and erase flows
- whether additional barriers, delays, or ordering constraints are needed in
  the in-repo environment
- whether FMC and SPI wrapper differences require any transport branching in
  `reference`

## Practical Implications For The In-Repo Design

The evidence supports the following implementation posture:

1. Put the transport primitive in the controller or wrapper layer, not in the
   device layer.
2. Treat command transport as ordered byte streaming, not as a second protocol
   framework.
3. Preserve and restore normal-read behavior after each transaction.
4. Keep device-specific policy above transport:
   status interpretation, WREN sequencing, WIP polling, timeout handling, and
   verify logic belong in `device/flash.rs`.

## What The Evidence Suggests But Does Not Yet Close

Before treating the transport design as fully settled, the remaining points to
confirm in `reference` are:
- the exact CS control register fields and masks used by the in-repo SMC HAL
- the cleanest place to store the restored normal-read control value
- whether an existing test can already exercise `status()` once the transport is
  wired
- whether QEMU is sufficient for write/erase validation or only for status-path
  smoke coverage

## Bottom Line

The plan's core recommendation is well supported by the cited `aspeed-rust`
code.

What is directly evidenced is not merely that flash commands exist, but that the
transport mechanism is aperture-based and user-mode-controlled. That is the key
fact that rules out adding a separate software-owned SPI engine for this phase.

What remains to be designed in `reference` is mostly about local register
representation, error semantics, and test coverage, not about the fundamental
shape of the transport.