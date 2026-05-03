# SMC IRQ Enablement Plan

Date: 2026-05-02

## Error interrupt flows

### Write-protect error (FMC008 bit 9)

1. A CS has its write-protect signal asserted — hardware pin or `CONF_ENABLE_Wx` cleared in FMC000.
2. A write or program command is issued — via user-mode PIO (`transceive_user`) or DMA write.
3. Hardware detects WP condition, rejects the command, sets FMC008 bit 9.
4. If `dmaintenbl` (FMC008 bit 3) is set, FMC fires IRQ to the CPU.
5. `handle_dma_irq()` decodes bit 9 → `WriteProtected` → `Err(SmcError::WriteProtected)`, state → `Error`.

### Command-abort error (FMC008 bit 10, non-DMA)

1. Controller is idle (no DMA in flight).
2. A command is issued that hardware aborts — incompatible mode on a CS, timing violation, etc.
3. Hardware sets bit 10, fires IRQ.
4. `handle_dma_irq()` decodes bit 10 with `dma_in_flight = false` → `CommandAbort` → `Err(SmcError::HardwareError)`, state → `Error`.

### DMA-abort error (FMC008 bit 10, DMA in flight)

1. `dma_read()` is called, DMA launched, `enable_dma_irq()` sets `dmaintenbl`, state = `DmaInFlight`.
2. Hardware encounters an abort during transfer (e.g., address out of segment bounds, alignment fault).
3. Hardware sets bit 10 (not bit 11), fires IRQ — no clean DMA completion.
4. `handle_dma_irq()` decodes bit 10 with `dma_in_flight = true` → `DmaError` → `Err(SmcError::DmaAborted)`, state → `Ready`.

### DMA complete (FMC008 bit 11, success path for reference)

1. Same DMA launch as above.
2. Hardware completes transfer cleanly, sets bit 11.
3. `handle_dma_irq()` decodes bit 11 → `DmaComplete` → `Ok(SmcInterrupt::DmaComplete)`, state → `Ready`.

---

## Current enablement gap

`dmaintenbl` is only set inside `dma_read()`. Consequence:

- **Write-protect errors on PIO/user-mode writes will never fire an IRQ.** `transceive_user()` does not call `enable_dma_irq()` / `disable_dma_irq()`.
- **Command-abort errors outside DMA will similarly never fire.** Bit 10 status will accumulate but no interrupt is delivered.

### Decision required

Option A — **DMA-only interrupt scope (current)**
Accept that error IRQs only fire during DMA operations. PIO/user-mode errors are detected synchronously by callers inspecting return values. This matches the aspeed-rust baseline exactly.

Option B — **Bracket all operations**
Call `enable_dma_irq()` at the start of `transceive_user()` and disable it on return. Gives write-protect and command-abort IRQs for PIO paths too, but adds IRQ-enable/disable overhead around every SPI byte transfer.

Recommendation: start with Option A. Add user-mode error IRQ bracketing only when a concrete requirement to catch PIO write-protect asynchronously exists.

---

## Tests needed before error IRQ path is complete

| Test | Location | Status |
|---|---|---|
| `handle_dma_irq` DmaInFlight → Ready on bit 11 | controller.rs #[cfg(test)] | missing |
| `handle_dma_irq` DmaInFlight → Error on bit 10 (DmaError) | controller.rs #[cfg(test)] | missing |
| `handle_dma_irq` Ready + bit 10 → CommandAbort | controller.rs #[cfg(test)] | missing |
| `handle_dma_irq` any state + bit 9 → WriteProtected | controller.rs #[cfg(test)] | missing |
| `handle_dma_irq` no bits set → ControllerNotReady (spurious fire) | controller.rs #[cfg(test)] | missing |
| `handle_dma_irq` bit 11 + bit 10 simultaneously → DmaComplete wins | interrupts.rs | covered |

These are all pure state-machine tests — no hardware needed. Extract a `process_dma_irq_status(status: u32, dma_in_flight: bool) -> (SmcState, Result<SmcInterrupt, SmcError>)` pure function from `handle_dma_irq()` and unit-test that directly.

---

## Implementation sequence

1. Extract `process_dma_irq_status` pure helper in `controller.rs`.
2. Add the 5 missing unit tests against that helper.
3. Decide on Option A vs B for PIO error IRQ scope.
4. Wire NVIC unmask in the caller (test harness or kernel task), not the driver.
