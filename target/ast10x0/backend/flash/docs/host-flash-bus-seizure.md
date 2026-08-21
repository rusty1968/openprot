# AST10x0 External Host Flash — Bus-Seizure Hazard and the Host-Hold Requirement

Companion to [host-flash-design.md](host-flash-design.md) §10. Explains, at the
SPI-bus level, why the RoT must hold the host before taking its flash bus, and
what goes wrong if it does not.

## The claim being unpacked

The external flash driver flips the SCU internal-master mux (`SCU0F0`) **per
operation** and does not assert host reset. That model is only safe if, at every
seize point, the host is **not** actively driving the flash. The design note
phrases this as an assumption that "the host tolerates losing the bus
mid-transaction" — an assumption that generally does **not** hold.

## What a SPI transaction is

A SPI flash transaction is a bounded sequence owned by exactly one master while
chip-select is asserted:

1. Master pulls `CS#` low.
2. Master clocks out a command byte (e.g. READ `0x03`, PAGE PROGRAM `0x02`,
   SECTOR ERASE `0x20`), usually followed by an address and/or data.
3. Master releases `CS#`.

While `CS#` is low, that master owns the bus. There is no protocol handshake to
politely yield the bus mid-burst.

## What the mux flip does

Normally the passthrough path is active: host pins → SPI monitor (SPIM) →
flash. Flipping the mux to insert the RoT internal master **severs the
host↔flash connection at whatever instant the flip lands**. It is unilateral and
immediate. If the host happens to be mid-transaction at that instant, its
transaction is cut off partway.

## Failure modes if the host is not held

- **Host mid-read.** The host was clocking data out of the flash (code fetch,
  boot data, config). After the flip it clocks against a disconnected or
  re-owned bus and receives truncated or garbage bytes. If that data was an
  instruction fetch, the host can execute corrupt code, hang, or fault.
- **Host mid-program/erase.** Worse: the flash may be left in a partially
  written page or a mid-erase state, and its internal write-in-progress (WIP)
  is still running. The RoT then takes over a chip that is busy or in an
  indeterminate state, so the RoT's own commands can fail or compound the
  corruption.
- **CS# / signal contention.** During the brief overlap, two would-be masters
  and an abruptly toggled `CS#` leave the bus in an undefined electrical
  state — CS glitches, dropped or extra clock edges.

## Why holding the host removes the hazard

The aspeed-zephyr reference asserts **host reset first** in `BMCBootHold()` /
`PCHBootHold()`, *then* switches the mux. A host held in reset is not a bus
master at all, so:

- there is no in-flight transaction to sever, and
- there is no two-master contention window.

The hold eliminates the assumption entirely rather than relying on timing.

## Consequence for the OpenPRoT design

Safety is not achieved inside the flash driver; it comes from **holding the
host** for the duration of the access window. In OpenPRoT that hold is decided
by the orchestrator state machine (`AssertReset` / `ReleaseReset` effects) and
carried out by the platform driver before it invokes flash access. The per-op
mux flips are harmless *while the host is held for the enclosing phase*; any
host-flash access without a preceding hold has no reset backstop and relies on
mux preemption alone, which is unsafe. Host-flash access outside a hold window
should be treated as unsupported until that case is explicitly designed.
