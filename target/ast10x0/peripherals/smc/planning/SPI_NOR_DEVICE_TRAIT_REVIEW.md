# SPI NOR Device Trait Review

## Scope
This note reviews the current SpiNorDevice trait with focus on:
- naming
- functionality boundaries
- type design best practices
- migration direction

Trait under review:

```rust
pub trait SpiNorDevice {
    type Error;
    fn nor_read_init(&mut self, data: &SpiNorCommand) -> Result<(), Self::Error>;
    fn nor_write_init(&mut self, data: &SpiNorCommand) -> Result<(), Self::Error>;
    fn nor_write_enable(&mut self) -> Result<(), Self::Error>;
    fn nor_write_disable(&mut self) -> Result<(), Self::Error>;
    fn nor_read_jedec_id(&mut self) -> Result<[u8; 3], Self::Error>;
    fn nor_sector_erase(&mut self, address: u32) -> Result<(), Self::Error>;
    fn nor_page_program(&mut self, address: u32, data: &[u8]) -> Result<(), Self::Error>;
    fn nor_page_program_4b(&mut self, address: u32, data: &[u8]) -> Result<(), Self::Error>;
    fn nor_read_data(&mut self, address: u32, buf: &mut [u8]) -> Result<(), Self::Error>;
    fn nor_read_fast_4b_data(&mut self, address: u32, buf: &mut [u8]) -> Result<(), Self::Error>;
    fn nor_sector_aligned(&mut self, address: u32) -> bool;
    fn nor_wait_until_ready(&mut self);
    fn nor_reset(&mut self) -> Result<(), Self::Error>;
    fn nor_reset_enable(&mut self) -> Result<(), Self::Error>;
}
```

## Executive Summary
The trait is functional for the current implementation, but it is tightly coupled to one controller style and exposes low-level transfer details. The main improvements are:
- make fallibility consistent
- separate flash semantics from transport internals
- remove mode-specific method explosion (3B vs 4B variants)
- introduce domain types (JedecId, Address, Geometry, Capabilities)
- tighten naming consistency

## Findings

### 1) Error handling is inconsistent
- nor_wait_until_ready returns unit, so transfer/read failures cannot be propagated.
- In a hardware driver trait, all operations that touch hardware should be fallible.

Recommended direction:
- change to wait_until_ready with a fallible signature
- optionally include timeout policy in API

Example:

```rust
fn wait_until_ready(&mut self, timeout_us: u32) -> Result<(), Self::Error>;
```

### 2) API surface is mode-specific and will scale poorly
- Methods like nor_page_program and nor_page_program_4b duplicate semantics by address mode.
- Similar duplication appears in read methods.

Risk:
- as command/mode variants grow, trait becomes large and fragile.

Recommended direction:
- one semantic operation per behavior, addressing carried by type or device state.

### 3) Trait mixes semantic operations and transport plumbing
- nor_read_init and nor_write_init look like controller transaction setup, not flash-device behavior.
- This leaks implementation details into the public trait.

Recommended direction:
- keep semantic trait minimal (read/program/erase/reset/status)
- keep transfer-init internals private or in a lower-level bus trait

### 4) Naming is redundant and inconsistent
- In SpiNorDevice, the nor_ prefix is repetitive.
- A mix of names couples operation and mode in inconsistent ways.

Recommended naming style:
- read_jedec_id
- read
- program_page
- erase_sector
- write_enable
- write_disable
- wait_until_ready
- reset

### 5) Mutability is broader than needed
- nor_sector_aligned uses &mut self though it is logically pure.

Recommended direction:
- use &self for pure queries/helpers

### 6) Missing explicit geometry and capabilities contracts
- Callers cannot query page size, sector size, total size, supported address width, supported read/program modes.

Recommended direction:
- expose Geometry and Capabilities types

### 7) JEDEC ID is weakly modeled
- returning [u8; 3] is compact but semantically opaque.

Recommended direction:
- define a small JedecId struct for readability and validation

## Type Design Best Practices

### Domain types to add

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JedecId {
    pub manufacturer: u8,
    pub memory_type: u8,
    pub capacity: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressMode {
    ThreeByte,
    FourByte,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlashAddress {
    pub value: u32,
    pub mode: AddressMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlashGeometry {
    pub page_size: usize,
    pub sector_size: usize,
    pub total_size: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlashCapabilities {
    pub supports_4byte_addressing: bool,
    pub supports_fast_read: bool,
    pub supports_quad_read: bool,
    pub supports_quad_program: bool,
}
```

### Error design notes
- Keep associated Error type.
- Ensure all HW-touching methods return Result.
- Avoid hidden failure paths in helper methods.

## Proposed Trait Shape (V2)

```rust
pub trait SpiNorDevice {
    type Error;

    fn jedec_id(&mut self) -> Result<JedecId, Self::Error>;
    fn geometry(&self) -> FlashGeometry;
    fn capabilities(&self) -> FlashCapabilities;

    fn write_enable(&mut self) -> Result<(), Self::Error>;
    fn write_disable(&mut self) -> Result<(), Self::Error>;

    fn read(&mut self, address: FlashAddress, buf: &mut [u8]) -> Result<(), Self::Error>;
    fn program_page(&mut self, address: FlashAddress, data: &[u8]) -> Result<(), Self::Error>;
    fn erase_sector(&mut self, address: FlashAddress) -> Result<(), Self::Error>;

    fn is_sector_aligned(&self, address: FlashAddress) -> bool;

    fn wait_until_ready(&mut self, timeout_us: u32) -> Result<(), Self::Error>;

    fn reset(&mut self) -> Result<(), Self::Error>;
}
```

Notes:
- If reset-enable is mandatory for some devices, reset should do it internally.
- If explicit control is needed, expose an advanced extension trait instead of forcing all users to sequence two methods.

## Backward-Compatible Migration Plan

1. Add new V2 trait (or add default methods in current trait).
2. Keep existing method names as compatibility wrappers.
3. Internally route 3B and 4B methods through unified read/program/erase paths.
4. Deprecate low-level init methods from the high-level device trait.
5. Migrate call sites incrementally, then remove deprecated methods.

## Practical Improvements for Current Implementation
- Make wait function fallible and optionally timeout-aware.
- Convert sector alignment helper to shared reference.
- Replace [u8; 3] return with JedecId.
- Introduce geometry/capabilities getters.
- Keep transport setup details out of the top-level trait when possible.

## Conclusion
Current design works for bring-up and initial integration, but a semantic-first trait with richer domain types will improve portability, safety, and maintainability across multiple flash devices and controller backends.
