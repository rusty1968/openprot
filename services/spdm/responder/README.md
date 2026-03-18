# openprot-spdm-responder

SPDM responder (server) implementation for OpenPRoT.

## Overview

This crate provides the SPDM responder role, which handles attestation requests from SPDM requesters. The responder processes requests and provides:
- Version and capability negotiation
- Certificate chain provisioning
- Device measurements
- Challenge-response attestation
- CSR generation and certificate updates

## Status

This crate is in early development. Basic infrastructure is in place, but SPDM protocol operations are not yet implemented.

## Dependencies

- `spdm-lib` — SPDM protocol library from 9elements
- `heapless` — `no_std` collections

## Future Work

- Implement SPDM 1.2+ response handlers
- Add GET_VERSION, GET_CAPABILITIES handlers
- Implement CHALLENGE response with signature
- Add GET_MEASUREMENTS handler
- Integrate with certificate storage
- Integrate with MCTP transport layer
- Add crypto service integration for signing operations
