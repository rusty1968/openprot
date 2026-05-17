# ecdsa / evb — HARDWARE-ONLY NIST KAT (goal.md §4.B)

Runs on the **AST1060 EVB** (physical board) only.

**Excluded from QEMU/CI. The user runs this on real AST1060 silicon.**

NIST CAVP "186-4 ECDSA" signature-verification vectors, curve **P-384 /
SHA-384** (`SigVer.rsp`, ECDSA2VS) — the correctness authority (goal.md
§2.3.2) and verdict-parity gate (§4.B). Cannot run on QEMU: the SBC model
has no ECC engine, so no accept/reject verdict exists there (parent README /
goal.md ADR-4). Running this on hardware also discharges P5-OPEN-A
behaviorally (SRAM base actually consumed) and the poll-budget tuning
obligation (goal.md §2.1 residual).

## Why it cannot be auto-run here

`.bazelrc:86` ends `--test_tag_filters=…,-hardware`, so a target tagged
`["hardware"]` is dropped from `bazel test --config=virt_ast10x0` and CI.
The real test target (when written) MUST carry both guards, exactly as
`tests/peripherals/i2c/i2c_init/BUILD.bazel` does:

```starlark
system_image_test(
    name = "ecdsa_evb_kat_test",
    image = ":ecdsa_evb_kat",
    tags = ["hardware"],                       # ← excluded by virt_ast10x0
    target_compatible_with = select({
        "//target/ast10x0:qemu_enabled": ["@platforms//:incompatible"],
        "//conditions:default": [],            # ← incompatible when QEMU on
    }),
)
```

## To implement (later, by the user)

1. Vendor the NIST CAVP P-384/SHA-384 `SigVer` vectors (record source +
   revision, same pinning discipline as `plans/zephyr-reference/`).
2. Mirror `i2c_init/` pw_kernel `system_image` firmware; the target.rs body
   drives `EcdsaOp::verify_raw` per vector and asserts the expected
   accept/reject.
3. Replace the `filegroup` placeholder in `BUILD.bazel` with the
   `system_image` + `system_image_test` wiring above. Keep `tags =
   ["hardware"]` and the `qemu_enabled` select.
4. Run on AST1060 hardware via `--config=k_ast1060_evb` (or the board
   runner). Record results back into goal.md §4.B.
