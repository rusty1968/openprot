# Device Layer Parity Gaps (aspeed-rust vs reference SMC)

Date: 2026-05-02

## Scope

This note captures remaining parity work for the device layer under:
- `target/ast10x0/peripherals/smc/device/flash.rs`

Transport-layer multi-CS routing is complete; these items are specific to
`SpiNorFlash` behavior and semantics.

## Remaining Gaps

1. Per-device capacity accounting (CS-relative)
- Current constructors validate `FlashConfig` against controller-level capacity,
  not selected-CS capacity.
- Needed: validate against the specific chip-select capacity (`Cs0` or `Cs1`).

2. CS-relative read offsets
- `read()` delegates to controller mapped read without applying per-CS base.
- Needed: treat `SpiNorFlash` offsets as device-local and translate to
  controller-window offsets by CS base.

3. CS-relative command addressing
- Program/erase command address bytes are currently built from unshifted `offset`.
- Needed: device-local offset model with explicit CS base handling so command
  addressing is consistent with read path semantics.

4. Transfer mode policy at device layer
- Device command paths hardcode `TransferMode::Mode111`.
- Needed: policy for exposing dual/quad at device layer (or explicit scope note
  if intentionally fixed to 111 for now).

5. Polling/timeout policy fidelity
- `wait_write_complete()` uses fixed poll budget and checks WIP bit only.
- Needed: document parity intent and, if required, add richer timeout/status
  handling to match target behavior expectations.

6. Verify API range behavior
- `verify()` currently uses a 256-byte scratch buffer and rejects larger slices.
- Needed: chunked verify for larger buffers or explicit API limitation docs.

## Completed Foundations (Already Green)

1. Raw transport path integrated through device ops (WREN/RDSR/program/erase).
2. `AddressWidth` command byte encoding is covered by host tests.
3. Multi-CS transport routing and `InvalidChipSelect` guard are covered by
   QEMU integration tests.
4. Not-ready error granularity is covered by QEMU integration tests.
5. Normal-read restore invariant is covered by QEMU integration tests.

## Suggested Next Implementation Order

1. CS-relative capacity + offset model in `SpiNorFlash`.
2. CS-relative command address generation for program/erase.
3. Add QEMU test asserting per-CS device-local offset semantics.
4. Expand `verify()` to chunk over arbitrary lengths.
5. Decide/document device-level transfer-mode policy (fixed 111 vs configurable).
