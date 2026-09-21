# PR #386 review — applying openprot-no-alloc / openprot-no-panic / openprot-secure-coding

PR: https://github.com/OpenPRoT/openprot/pull/386
Scope reviewed: `services/attest/api/src`, `services/attest/producer/src`, `services/attest/producer/tests`

## openprot-no-alloc — violation (crate-wide)

`openprot-attest-api` and `openprot-attest-producer` are the only two crates under `services/`
in this repo that aren't `#![no_std]`. Every sibling (`services/i2c/*`, `services/mctp/*`,
`services/orchestrator/*`, `services/pldm`, `services/spdm/*`) declares `#![no_std]` or
`#![cfg_attr(not(test), no_std)]`. This PR uses `std` freely in production (non-test) code:

- `services/attest/api/src/types.rs:112,129,137,142` — `OemId(pub Vec<u8>)`,
  `Measurement.digest: Vec<u8>`, `CertChain(pub Vec<Vec<u8>>)`, `hw_model: String`
- `services/attest/producer/src/signer.rs:729,741,743` — `std::sync::Arc<dyn CaliptraSigner>`,
  `Vec<Box<dyn MeasurementProvider>>`
- `services/attest/producer/src/builder.rs:234,241` — `Vec<(Value, Value)>`,
  `std::time::SystemTime`

None of it is gated behind `#[cfg(test)]`. If this is meant to run on-device it needs
`heapless` collections and `#![no_std]` like the rest of `services/`; if it's genuinely
host-only tooling, that should be stated explicitly since it breaks the established pattern.

## openprot-no-panic — clean

Every `.unwrap()`/`.expect()` in the diff is inside `#[cfg(test)]` modules or
`tests/producer_integration.rs`. No panics in production code paths.
`#![forbid(unsafe_code)]` is set in both crate roots (`api/src/lib.rs:52`,
`producer/src/lib.rs:553`).

## openprot-secure-coding — violation

`zeroize` is added as a dependency (`producer/Cargo.toml:19`, `producer/BUILD.bazel:16`) and
the README (`producer/README.md:81`) documents it as "Zero-on-drop for intermediate key
material and sensitive buffers" — but it's never imported or called anywhere in the source.
Dead dependency, and the security claim in the README is currently false.

No hardware register access in this PR, so that section of the skill doesn't apply.

## Verbosity / repetition (ponytail:ponytail-review)

```
producer/src/builder.rs:L236-316: shrink: nine near-identical claims.push((Value::Integer(CLAIM_X.into()), Value::...)) blocks. claim(key: i64, val: Value) -> (Value, Value) helper, 1 line per claim.
producer/src/builder.rs:L403-414 / producer/tests/producer_integration.rs:L860-875: delete: decode_payload/find_claim copy-pasted across two test files. One shared test helper, nothing duplicated.
producer/src/signer.rs:L825-833: delete: stub_leaf_cert()/stub_ca_cert() are identical one-line bodies. One function or a shared const.

net: -30 lines possible.
```
