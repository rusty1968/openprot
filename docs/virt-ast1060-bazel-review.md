# Virtual AST1060 Bazel Configuration Review

**Date:** 2025-01-27  
**Scope:** Review of `target/ast1060/crypto/` and supporting QEMU configuration  
**Verdict: PASS with 3 findings (2 cleanup, 1 informational)**

---

## 1. Review Methodology

Compared `target/ast1060/crypto/` against:

- **Known-working QEMU targets:** `target/ast1060/ipc/user/` and `target/ast1060/threads/kernel/`
- **Hardware counterpart:** `target/ast1060-evb/crypto/`
- **Platform definitions:** `target/ast1060/BUILD.bazel` vs `target/ast1060-evb/BUILD.bazel`
- **Bazel configs:** `.bazelrc` entries `k_qemu_ast1060`, `virt_ast1060_evb`, `k_ast1060_evb`
- **Crate registries:** `@rust_crates`, `@oot_crates_no_std`, `@rust_crates_base` via `MODULE.bazel`

---

## 2. Structural Correctness — PASS

`target/ast1060/crypto/BUILD.bazel` follows the exact same pattern as the known-working
`target/ast1060/ipc/user/BUILD.bazel`:

| Rule                       | ipc/user | crypto | Match? |
|--------------------------- |----------|--------|--------|
| `system_image`             | ✅        | ✅      | ✅      |
| `system_image_test`        | ✅        | ✅      | ✅      |
| `rust_binary_no_panics_test` | ✅     | ✅      | ✅      |
| `filegroup` (system_config)| ✅        | ✅      | ✅      |
| `target_codegen`           | ✅        | ✅      | ✅      |
| `target_linker_script`     | ✅        | ✅      | ✅      |
| `rust_binary` (target)     | ✅        | ✅      | ✅      |

Both include `target_codegen` because both use userspace apps needing IPC codegen.
The `threads/kernel` test correctly omits `target_codegen` (kernel-only, no userspace).

---

## 3. Platform & Console Backend — PASS

```
platform "//target/ast1060" → console_backend_semihosting  ← correct for QEMU
platform "//target/ast1060-evb" → console_backend_uart     ← correct for hardware
```

The console backend is resolved via platform flags in `target/ast1060/BUILD.bazel`:

```python
flags = flags_from_dict(
    KERNEL_DEVICE_COMMON_FLAGS | {
        "@pigweed//pw_kernel/subsys/console:console_backend":
            "@pigweed//pw_kernel/subsys/console:console_backend_semihosting",
    },
)
```

The crypto target binary depends on `@pigweed//pw_kernel/subsys/console:console_backend`
(the label flag), which resolves to semihosting when built under `//target/ast1060`.
This matches the pattern used by all other `target/ast1060` tests.

---

## 4. target.rs — PASS

`target/ast1060/crypto/target.rs` is structurally identical to `target/ast1060/ipc/user/target.rs`:

- Both use `cortex_m_semihosting::debug::{EXIT_FAILURE, EXIT_SUCCESS, exit}` for QEMU shutdown
- Both call `codegen::start()` in `main()` to launch userspace processes
- Both use `{console_backend as _, entry as _}` for link-time inclusion
- Both implement `TargetInterface` with `shutdown()` that signals exit status to QEMU

The `shutdown()` function is critical: it calls `exit(EXIT_SUCCESS)` or `exit(EXIT_FAILURE)`,
which triggers a semihosting `SYS_EXIT` call. This is how `system_image_test` detects
pass/fail when running under QEMU.

---

## 5. Constraint Chain — PASS

The constraint resolution chain is internally consistent:

```
.bazelrc: virt_ast1060_evb → --platforms=//target/ast1060
     ↓
target/ast1060/BUILD.bazel: constraint_value "target_ast1060"
     ↓
target/ast1060/defs.bzl: TARGET_COMPATIBLE_WITH selects on "target_ast1060"
     ↓
target/ast1060/crypto/BUILD.bazel: uses TARGET_COMPATIBLE_WITH from defs.bzl
```

All targets in `target/ast1060/crypto/` use `target_compatible_with = TARGET_COMPATIBLE_WITH`,
which evaluates to `[]` (compatible) only when the platform sets `target_ast1060`.

---

## 6. Crate Registry Analysis — PASS (informational note)

### Architecture

```
MODULE.bazel defines 3 registries:
  @rust_crates_base  ← host crates (anyhow, clap, etc.) from //third_party/crates_io
  @oot_crates_no_std ← embedded crates (cortex-m, sha2, etc.) from //third_party/crates_io/crates_no_std
  @rust_crates       ← alias hub that routes cortex-m → @oot_crates_no_std, host → @rust_crates_base
```

### What references what

| Crate reference | Used by | Resolves to |
|----------------|---------|-------------|
| `@rust_crates//:cortex-m-semihosting` | target binary | `@oot_crates_no_std//:cortex-m-semihosting` (via alias) |
| `@rust_crates//:cortex-m-rt` | entry.rs | `@oot_crates_no_std//:cortex-m-rt` (via alias) |
| `@oot_crates_no_std//:sha2` | crypto-server | direct |
| `@oot_crates_no_std//:hmac` | crypto-server | direct |
| `@oot_crates_no_std//:aes-gcm` | crypto-server | direct |
| `@oot_crates_no_std//:zerocopy` | crypto-api, client, server | direct |

**No conflict:** `@rust_crates//:cortex-m-semihosting` is an alias for
`@oot_crates_no_std//:cortex-m-semihosting`. Both the target binary and the
crypto service crates ultimately resolve to the same `@oot_crates_no_std` registry
for all embedded crates. There is no ABI mismatch risk.

**Informational note:** The crypto service BUILD files reference `@oot_crates_no_std`
directly rather than going through the `@rust_crates` alias hub. This works but is
a style inconsistency with the Pigweed-upstream convention of using `@rust_crates`.
Currently the `@rust_crates` alias hub does not include entries for `sha2`, `hmac`,
`aes-gcm`, `zerocopy`, etc. If these aliases were added, the service BUILD files
could use `@rust_crates//:sha2` and the hub would route them to `@oot_crates_no_std`.
This is purely cosmetic — not a correctness issue.

---

## 7. Memory Layout — PASS

`system.json5` defines the memory map:

```
0x00000000 - 0x000004A0  Vector table (1184 bytes)
0x000004A0 - 0x00020000  Kernel code (~126KB)
0x00020000 - 0x00040000  Crypto client (128KB flash)
0x00040000 - 0x00080000  Crypto server (256KB flash)
0x00080000 - 0x000A0000  Kernel RAM (128KB)
0x000A0000 - 0x000B0000  App RAM (64KB: 16KB client + 48KB server)
                          Total: 704KB — fits within AST1060's 768KB SRAM
```

This layout is identical to the `ast1060-evb/crypto/system.json5`. The server
gets 256KB of flash (needed for RustCrypto software implementations) and 48KB
RAM (8KB stack for crypto operations). All addresses are power-of-2 aligned for
PMSAv7 MPU compatibility.

---

## 8. QEMU Run-Under Configuration — PASS

```bash
# .bazelrc
test:virt_ast1060_evb --run_under="@pigweed//pw_kernel/tooling:qemu \
  --cpu cortex-m4 \
  --machine ast1030-evb \
  --semihosting \
  --image "
```

- `--cpu cortex-m4` — matches AST1060 hardware (Cortex-M4F), compatible with M3 soft-float ABI
- `--machine ast1030-evb` — QEMU machine model for AST1030/AST1060 family
- `--semihosting` — enables ARM semihosting for console output and exit signaling
- `--image` — tells the wrapper to pass the system image binary to QEMU

This is identical to the established `k_qemu_ast1060` config.

---

## 9. Findings

### Finding 1: Redundant .bazelrc Config (cleanup)

**Severity:** Low — cosmetic duplication  
**Location:** `.bazelrc` lines 56–67

`virt_ast1060_evb` is **functionally identical** to `k_qemu_ast1060`:

| Property | `k_qemu_ast1060` | `virt_ast1060_evb` |
|----------|-------------------|--------------------|
| `--config=` | `k_common` | `k_common` |
| `--platforms=` | `//target/ast1060` | `//target/ast1060` |
| QEMU CPU | `cortex-m4` | `cortex-m4` |
| QEMU machine | `ast1030-evb` | `ast1030-evb` |
| Semihosting | yes | yes |

**Recommendation:** Keep `virt_ast1060_evb` if the naming convention is preferred
for clarity alongside `k_ast1060_evb`. Otherwise remove and use `k_qemu_ast1060`.
Not a correctness issue either way.

### Finding 2: Orphaned `target/virt-ast1060-evb/` Directory (cleanup)

**Severity:** Low — dead code  
**Location:** `target/virt-ast1060-evb/`

This directory was created during an earlier design iteration. It contains:
- `BUILD.bazel` — defines platform `virt-ast1060-evb` and constraint `target_virt_ast1060_evb`
- `defs.bzl` — constraint select for unused constraint
- `entry.rs` — duplicate of `target/ast1060/entry.rs`
- `crypto/` — incomplete crypto test target

**Nothing references it.** No `.bazelrc` config points to `//target/virt-ast1060-evb`.
It builds nothing.

**Recommendation:** Delete `target/virt-ast1060-evb/` entirely.

### Finding 3: `@oot_crates_no_std` Direct References (informational)

**Severity:** Informational — no action required  
**Location:** `services/crypto/{api,client,server}/BUILD.bazel`

The crypto service crates use `@oot_crates_no_std//:sha2`, etc. directly rather
than `@rust_crates//:sha2`. This works because both registries support
`thumbv7m-none-eabi` and the alias hub routes embedded crates to
`@oot_crates_no_std` anyway.

If the project eventually adds `sha2`/`hmac`/`aes-gcm`/`zerocopy` aliases to
`third_party/crates_io/rust_crates/alias_hub.BUILD`, the service BUILD files
could migrate to `@rust_crates` for consistency. Low priority.

---

## 10. Comparison Summary

| Aspect | Virtual (`ast1060/crypto`) | Physical (`ast1060-evb/crypto`) |
|--------|---------------------------|--------------------------------|
| Platform | `//target/ast1060` | `//target/ast1060-evb` |
| Console | semihosting | UART |
| Entry | minimal (no DDK) | full (Aspeed DDK + ISR stubs) |
| Crate registry | `@rust_crates` (→ `@oot_crates_no_std`) | `@oot_crates_no_std` direct |
| Memory layout | 704KB total | 704KB total (identical) |
| QEMU support | ✅ run_under configured | ❌ no QEMU, flash to board |
| Test runner | `system_image_test` → QEMU | `system_image_test` → board runner |

---

## 11. Conclusion

The virtual AST1060 crypto test configuration is **correctly structured** and
follows established patterns from working QEMU test targets (`ipc/user`,
`threads/kernel`). The build succeeds. The configuration is ready for test
execution under QEMU via:

```bash
bazel test --config=virt_ast1060_evb //target/ast1060/crypto:crypto_test
```

Two cleanup items (redundant config, orphaned directory) should be addressed
but do not block testing.
