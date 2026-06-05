# AST10x0 PFR SMBus Protocol Crate

This crate implements the **proprietary SMBus mailbox protocol policy** for AST10x0 PFR.

## What This Crate Is

This crate is the policy layer between:

- transport/peripheral I/O (`target/ast10x0/peripherals/smb_mbox`)
- PFR protocol behavior (source-aware write filtering and notification-to-event mapping)

In short:

- `smbus_protocol.rs` = protocol rules
- `smb_mbox_adapter.rs` = bridge from protocol rules to mailbox register reads/writes

## Behavioral Mapping

The implementation maps directly to the protocol behavior defined in this crate:

- **Ownership filtering**
  - BMC mask `0x23`
  - PCH/CPU mask `0x03`
  - Implemented in `SmbusProtocol::filter_write`

- **Update intent handling**
  - BMC intent emits `UpdateRequested`
  - PCH intent is masked first, then conditionally emits `UpdateRequested`
  - Implemented in `SmbusProtocol::on_notification`

- **Intent2 handling**
  - BMC intent2 supports seamless-update ack bit behavior (clear/writeback path)
  - PCH intent2 emits `UpdateIntent2Requested`

- **Checkpoint handling**
  - ACM/BIOS/BMC checkpoint registers emit `WdtCheckpoint`

- **Reset communication handling**
  - BMC reset-communication value `1` emits event and requests writeback to `0`

- **Provision trigger handling**
  - UFM command trigger notifications emit `ProvisionCmd`

## Layering Rules

- Keep `target/ast10x0/peripherals` free of protocol policy.
- Keep this crate free of transport specifics and IRQ threading policy.
- Do not move source-access policy into peripheral register wrappers.

## API Overview

- `SmbusProtocol`
  - `filter_write(source, addr, value)`
  - `on_notification(addr, value)`

- `SmbMboxAdapter`
  - `write_from_source(mailbox, source, addr, value)`
  - `handle_notification(mailbox, addr)`

## Out of Scope

This crate does **not** implement:

- queue/dispatcher threading
- timer stop side effects outside mailbox writeback/event return
- full firmware state-machine transitions
- transport/IPC callback wiring

Those belong to higher integration layers.
