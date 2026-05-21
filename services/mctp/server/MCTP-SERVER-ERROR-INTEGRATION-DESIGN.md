# MCTP Server Integration Design: Precise Errors

Author: Copilot (draft)
Date: 2026-05-21
Status: Proposed

## Scope

Integrate `openprot_error` (from `//util/error:error`) into the MCTP server path under `services/mctp/server` while preserving wire/API behavior expected by existing MCTP clients.

This design assumes your "mcrp" request refers to MCTP.

## Goals

1. Use globally unique, module-encoded 32-bit errors inside MCTP server logic.
2. Keep current MCTP wire semantics (`ResponseCode`) stable during migration.
3. Avoid large-scale breakage in `services/mctp/api`, `services/mctp/client`, and tests.
4. Provide a path to full precise-error adoption in the API layer later.

## Non-Goals

1. Changing MCTP protocol wire format in this phase.
2. Replacing all project errors in one CL.
3. Implementing cross-repo global uniqueness tooling in this CL.

## Current State

1. `services/mctp/server/src/server.rs` returns `Result<_, MctpError>`.
2. `MctpError` currently wraps `ResponseCode` only (`services/mctp/api/src/error.rs`).
3. Internal server errors are mapped from `mctp::Error` into `ResponseCode` values.
4. New crate `util/error` now exists with:
   - `openprot_error::Error`
   - `openprot_error::ErrorModule`
   - status constants and 32-bit encoding helpers.

## Design Summary

Adopt a **two-layer error model** for MCTP server migration:

1. **Internal layer**: `openprot_error::Error` (precise, module-specific).
2. **External/API layer**: existing `MctpError`/`ResponseCode` (compatibility boundary).

This keeps all public behavior stable while enabling precise errors internally.

## Proposed Error Model

### 1) New MCTP server module ID and constants

Add a dedicated module namespace in `services/mctp/server/src/error.rs`:

- `pub const ERR_MCTP_SERVER: ErrorModule = ErrorModule::new(0x0101);`

Module-local example codes:

- `ERR_REQ_ALLOC = 0x0001`
- `ERR_LISTENER_ALLOC = 0x0002`
- `ERR_SET_EID = 0x0003`
- `ERR_REGISTER_RECV_NO_SPACE = 0x0004`
- `ERR_SEND_TOO_LARGE = 0x0005`
- `ERR_SEND_FAIL = 0x0006`
- `ERR_INBOUND_FAIL = 0x0007`
- `ERR_UNBIND_FAIL = 0x0008` (if ever made fallible)

Expose constants as `openprot_error::Error` values:

- `pub const E_SEND_TOO_LARGE: Error = ERR_MCTP_SERVER.error(ERR_SEND_TOO_LARGE);`

### 2) Introduce server-internal error alias

In `services/mctp/server/src/error.rs`:

- `pub type ServerError = openprot_error::Error;`

### 3) Add compatibility mapping to existing API error

Provide deterministic mapping from precise error to `ResponseCode`:

- `ServerError -> ResponseCode` using module-local code table
- fallback to `ResponseCode::InternalError`

And helper:

- `fn to_mctp_error(e: ServerError) -> MctpError`

This allows current server API signatures to remain unchanged initially.

### 4) Refactor server internals to generate precise errors first

In `services/mctp/server/src/server.rs`:

1. Convert `mctp_error_to_server_error(e: mctp::Error) -> MctpError`
2. Into:
   - `mctp_error_to_precise_error(e: mctp::Error) -> ServerError`
3. At API boundary, map with `to_mctp_error(...)`.

Result: codebase starts producing precise errors internally, while callers still receive `MctpError`.

## API Evolution Plan

### Phase 0 (this design)

- Keep all public method signatures unchanged (`Result<_, MctpError>`).
- Add internal precise error generation + mapper.

### Phase 1 (opt-in API)

In `services/mctp/api`:

1. Extend `MctpError` with optional precise payload:
   - `pub precise: Option<openprot_error::Error>`
2. Keep `code: ResponseCode` for compatibility.
3. Add constructors:
   - `MctpError::from_code(...)`
   - `MctpError::from_precise(precise, mapped_code)`

This allows logging/telemetry to carry exact error identity without protocol change.

### Phase 2 (full precise-error-first API)

Introduce new trait/API surface (versioned):

- `Result<_, openprot_error::Error>` as canonical return type.
- Keep old API as compatibility shim.

## Wire/Protocol Behavior

No wire changes in this plan.

`ResponseCode` remains the on-wire response code. Precise error remains local/internal (or side-channel telemetry) until protocol extension RFC is approved.

## Bazel and Dependency Changes

### Required

1. `services/mctp/server/BUILD.bazel`
   - add `//util/error:error` to `mctp_server_lib` deps.

### Optional (Phase 1)

2. `services/mctp/api/BUILD.bazel`
   - add `//util/error:error` only if `MctpError` stores precise error.

## Suggested File Changes

1. Add `services/mctp/server/src/error.rs`
2. Update `services/mctp/server/src/lib.rs` exports:
   - export `ServerError` and key constants
3. Update `services/mctp/server/src/server.rs`:
   - use precise errors internally
   - map to `MctpError` at return boundary
4. Add tests in `services/mctp/server/tests/server_unit.rs`:
   - mapping from `mctp::Error` -> precise code
   - mapping from precise code -> `ResponseCode`

## Test Plan

1. Unit tests (server error module)
   - exact raw values for chosen module/code constants
   - stable mapping table assertions
2. Existing server tests
   - `mctp_server_unit_test`
   - `mctp_server_dispatch_test`
   - `mctp_server_integration_test`
3. Regression checks
   - verify no behavior changes in API-visible `ResponseCode`
4. Optional telemetry test (Phase 1)
   - ensure precise error is preserved when `MctpError` carries both fields

## Risks and Mitigations

1. Risk: accidental behavior change from new mappings.
   - Mitigation: explicit mapping table tests for every old `ResponseCode` path.
2. Risk: duplicate module IDs across crates.
   - Mitigation: reserve module ID range and add follow-up uniqueness scanner.
3. Risk: migration churn across call sites.
   - Mitigation: compatibility boundary preserves `MctpError` signatures initially.

## Rollout Plan

1. CL 1
   - add server-local precise error definitions + mappings
   - keep public signatures unchanged
2. CL 2
   - add optional precise field in `MctpError` (if desired)
3. CL 3
   - begin adopting precise-error-first APIs in selected clients (echo first)
4. CL 4
   - add uniqueness tooling and CI enforcement

## Open Questions

1. Resolved: use internal namespace (`ErrorModule::new`) for project-owned MCTP crates.
2. Do we want one module ID for all MCTP crates, or per crate (`api`, `client`, `server`, `transport-i2c`)?
3. Should precise raw error be included in protocol responses in a future wire version?
4. Which telemetry sink should receive precise raw values during Phase 0?
