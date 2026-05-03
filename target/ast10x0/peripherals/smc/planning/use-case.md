# SMC Use Case: FMC Self-Update for a Mini-BMC

Date: 2026-05-02

## Goal

This use case captures the primary real-world scenario the AST10x0 SMC device layer must support: a mini-BMC updating its own boot flash through the FMC controller.

The intent is to anchor the SMC roadmap in an operational workflow rather than in isolated transport features.

## Why FMC Self-Update Is The Right Anchor

For a mini-BMC, the most important flash path is not an arbitrary external SPI peripheral. It is the primary boot flash attached to FMC.

That path matters because the system must be able to:

1. Identify the attached flash reliably.
2. Read an existing image or metadata from flash.
3. Erase the flash region that will receive a new image.
4. Program replacement image data into flash.
5. Verify the programmed contents.
6. Recover or fail safely if the write path does not complete.

This is the operational center of gravity for the driver.

## Proven Reference In aspeed-rust

The closest proven reference is the SPI NOR flow in aspeed-rust, specifically:

1. `src/spi/norflash.rs`
   - defines the NOR command vocabulary used in production-style flows
   - includes `RDID` (`0x9F`), `WREN` (`0x06`), `RDSR` (`0x05`), `PP` (`0x02`), and `SE` (`0x20`)
   - exposes `nor_read_jedec_id()`, `nor_write_enable()`, `nor_sector_erase()`, `nor_page_program()`, and `nor_wait_until_ready()`

2. `src/spi/spitest.rs`
   - contains block-device style erase/program/read-back validation
   - exercises a range-oriented usage pattern that is closer to firmware update than a single-page smoke test
   - the `test_block_device()` flow erases a block range, programs a larger buffer, reads it back, and compares the result

3. `src/spi/spidmairqtest.rs`
   - validates FMC DMA read/write completion behavior and chained DMA flows
   - shows the interrupt-driven side of the transport on real hardware

4. `tests-hw/src/main.rs`
   - routes these flows through the hardware test entrypoints, making them more than unit-level examples

The important point is not that reference and aspeed-rust must share identical APIs. The important point is that the update workflow is already proven there in silicon-facing code.

## Mini-BMC Self-Update Flow

At a high level, the mini-BMC self-update sequence looks like this:

1. Boot the updater logic from SRAM or another safe execution context.
2. Use FMC to read JEDEC ID and confirm the attached flash device matches expectations.
3. Read update metadata, image header, and target flash offsets.
4. For each destination region:
   - erase one or more sectors
   - wait for WIP to clear
   - program one or more pages
   - wait for WIP to clear after each page or write unit
5. Read back programmed data and verify it matches the staged image.
6. Mark the image valid or hand off to the next boot stage.

This is the use case the current SMC device layer should be judged against.

## What The Driver Must Provide

For that workflow, the SMC stack must provide the following capabilities.

### Identification

The updater must identify the flash before trusting any geometry or capability assumptions.

Required behavior:

1. JEDEC read over user-mode command transport.
2. Access to status register reads.
3. An explicit address-width policy where needed.

Current status:

- JEDEC read is now implemented in the reference SMC device layer.
- Status register read is implemented.

### Erase / Program Primitives

The updater needs reliable erase and page program operations on the primary flash.

Required behavior:

1. `WREN` sequencing before modifying operations.
2. Sector erase at device-local offsets.
3. Page program at device-local offsets.
4. WIP polling until completion.
5. Correct CS-local address handling.

Current status:

- Sector erase and page program are implemented.
- WIP polling is implemented.
- CS-local addressing semantics are implemented.

### Verification

Self-update is not complete when the last page write returns. It is complete when the written image has been verified.

Required behavior:

1. Deterministic read-back verification.
2. Support for buffers larger than a single page.
3. Bounded memory usage in verification code.

Current status:

- `verify()` is chunked and supports multi-page input.

### Optional Performance Path

PIO and command transport are sufficient for correctness. DMA matters when update size or latency starts to matter.

Required behavior for a future optimized updater:

1. DMA read for staging or verification acceleration.
2. IRQ-driven completion so the updater task can block efficiently.
3. Clear error reporting for DMA abort vs command abort vs write-protect.

Current status:

- DMA read launch and IRQ completion decode exist.
- End-to-end userspace completion routing is not yet in place.
- DMA write is not implemented.

## What Is Not Yet First-Class In The Device API

The self-update use case naturally wants range-oriented operations:

1. program an image chunk spanning many pages
2. erase a region spanning many sectors
3. verify a whole image region

The current device API is intentionally primitive-oriented:

- `erase_sector(offset)`
- `program_page(offset, data)`
- `verify(offset, expected)`

That is enough to implement self-update in a loop, but it means the updater must currently own pagination and sector iteration logic.

This is acceptable for now, but it should be considered a use-case-driven gap rather than a transport bug.

## Why This Use Case Matters For Planning

This use case sharpens roadmap decisions:

1. JEDEC support is not optional bring-up sugar; it is part of safe flash identification.
2. Device-local offset semantics are required because an updater must reason about the selected chip, not the controller window.
3. Verify must support multi-page buffers because firmware images are larger than one page.
4. IRQ work should be justified by updater task behavior and large-transfer efficiency, not by architecture purity alone.
5. Range-oriented helpers may be worth adding once the primitive layer is stable.

## Recommended Next Interpretation Of The Roadmap

If the roadmap is evaluated against mini-BMC FMC self-update, then the layers line up like this:

1. Device primitives: largely sufficient now.
2. JEDEC + status + erase + program + verify: sufficient to implement a correctness-first updater.
3. DMA + IRQ routing: next performance/concurrency layer, not a prerequisite for baseline self-update.
4. Multi-page/multi-sector convenience APIs: a use-case-driven ergonomic improvement, not a blocker.

## Bottom Line

The SMC device layer is now close to a correctness-first FMC self-update substrate.

What remains is not basic flash semantics. What remains is making the updater experience better:

1. range-oriented helper APIs
2. userspace service routing
3. optional DMA/IRQ acceleration for larger transfers

That is the right framing for the next phase.
