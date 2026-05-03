# SMC Register Patterns (from aspeed-rust audit)

Date: 2026-05-02

## FMC008: DMA Status / Interrupt Enable

FMC008 is a dual-purpose register. The PAC exposes two named fields:

| Field         | Bit | Direction | Description                              |
|---------------|-----|-----------|------------------------------------------|
| `dmastatus`   | 11  | R/W1C     | DMA complete (write 1 to clear)          |
| `dmaintenbl`  | 3   | R/W       | DMA interrupt enable                     |

Bits 10 (command abort) and 9 (write protect) are also readable and W1C status bits, not separately named in the PAC — read/clear via raw `.bits()` mask.

### Interrupt enable pattern (from aspeed-rust `fmccontroller.rs`)

```rust
// Enable — called at end of dma_read() / program_dma(), not at init
self.regs.fmc008().modify(|_, w| w.dmaintenbl().set_bit());

// Disable — called at TOP of handle_interrupt(), before processing
self.regs.fmc008().modify(|_, w| w.dmaintenbl().clear_bit());
```

**Key constraint:** enable/disable is per-operation, not a one-time init toggle.

### Our deviation (bug)

We incorrectly set `DMA_EN` (bit 3) of **FMC06C** (spi_mode register) at init time instead of using `dmaintenbl` on FMC008. Fix: remove the `modify_spi_mode` call from `init()`, add `enable_dma_irq()` / `disable_dma_irq()` accessors in `registers.rs` using the PAC `dmaintenbl` field, and call them in `dma_read()` and `handle_dma_irq()` respectively.

## FMC06C: SPI I/O Mode Register

Bit 3 (`DMA_EN`) in FMC06C is **not** the interrupt enable. It controls whether DMA transfers are used at all (vs. PIO). It is separate from the interrupt enable in FMC008.

## IRQ lifecycle (aspeed-rust pattern)

1. `init()` — configure segments, timing, normal-read ctrl. Do **not** enable NVIC or FMC008 interrupt here.
2. `dma_read()` / `dma_write()` — set up DMA registers, write `DMA_CTRL_REQUEST`, then call `enable_dma_irq()`.
3. ISR / `handle_interrupt()` — call `disable_dma_irq()` first, then check `dmastatus.is_dma_finish()`, clear status, transition state.
4. NVIC unmask (`NVIC::unmask(Interrupt::fmc)`) — caller's responsibility, not the driver's.

## handle_interrupt scope

aspeed-rust `handle_interrupt` only checks `dmastatus` (bit 11). It returns an error if that bit is not set but does not decode write-protect (bit 9) or command-abort (bit 10).

Our `SmcInterruptDecoder::decode_with_context` is a superset — it handles all three error sources. We should keep this behavior.
