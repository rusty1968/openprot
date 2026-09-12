# Task: Complete the Caliptra dependency uprev in `openprot`

## Goal

Finish bumping the pinned Caliptra source revisions in `/home/antrocha/work/otp/openprot`
so that the whole build graph resolves and compiles, then land it as a single commit.

**Definition of done:**
1. `cd /home/antrocha/work/otp/openprot && bazelisk build //target/veer/tooling:caliptra_runner` succeeds.
2. `bazelisk test //target/veer/tests/otp:emulator_test --test_output=all` passes.
3. Changes committed (terse message, e.g. `uprev caliptra pins to mcu-sw@476be194 / sw@85981e1b / dpe@cf3224c4`).

Work **only** on the uprev. Do **not** stage/commit the in-progress OTP-service files
(untracked: `services/otp/`, `target/veer/peripehrals/`, `target/veer/tests/otp/`,
`OTP_SERVICE_BACKEND_PLAN.md`, `hal/blocking/otp/README.md`, this prompt file) — they are a
separate feature. The uprev must be its own commit. The one exception: the OTP emulator test
(`//target/veer/tests/otp:emulator_test`) is the final validation target, so it must build/run,
but its *source* belongs to the other feature — don't fold it into the uprev commit.

## The target pins (already applied)

| repo             | old rev      | NEW rev (target)                             |
|------------------|--------------|----------------------------------------------|
| `caliptra_mcu_sw`| b7e45fc1…    | `476be194a8664e69cc111280ed8f40b355b25c91`   |
| `caliptra_sw`    | 2fe38a09…    | `85981e1bc28662a9a9fa5040cbb38ac46e548217`   |
| `caliptra_dpe`   | f56f66ef…    | `cf3224c41b64789aa556e4f0c78e7395c6528a43`   |
| `caliptra_cfi`   | a98e499d…    | **KEEP `a98e499d279e81ae85881991b1e9eee354151189`** (see note) |
| `ureg`           | 412ca401…    | unchanged (bazel_dep git_override in MODULE.bazel)|

**cfi note:** the `uprev` tool mis-derived `caliptra_cfi` → `72c75dc7…`, but openprot uses the
`-git`-suffixed crates (`caliptra-cfi-lib-git` / `caliptra-cfi-derive-git`) which only exist at
`a98e499d…`. It must stay `a98e499d…`. (dpe@cf3224c4 separately pulls the *non*-suffixed
`caliptra-cfi-lib` / `caliptra-cfi-derive` from `caliptra-cfi.git@72c75dc7`, but those are
redirected by a `[patch]` — see below — so they don't matter for the pin.)

## Environment facts

- Always `cd /home/antrocha/work/otp/openprot` before any `bazelisk` command (cwd sometimes resets).
- `bazelisk` is the wrapper. Repin with `CARGO_BAZEL_REPIN=1 bazelisk build …` **only when a
  `crates_io/*/Cargo.toml` changed**; otherwise a plain `bazelisk build` is enough (and much faster).
- Bazel output_base: `/home/antrocha/.cache/bazel/_bazel_antrocha/120c8194565e0190bb5a6690c0259b98`
  - fetched caliptra-sw source: `<output_base>/external/+caliptra_repos+caliptra_sw`
  - generated embedded hub BUILD: `<output_base>/external/rules_rust++crate+rust_caliptra_crates/BUILD.bazel`
  - generated host hub BUILD: `<output_base>/external/rules_rust++crate+rust_caliptra_crates_host/BUILD.bazel`
- Local mcu-sw checkout (HEAD already at 476be194): `/home/antrocha/work/otp/caliptra-mcu-sw`
- An **ssh passphrase prompt** for `~/.ssh/id_ed25519` may appear intermittently when bazel does a
  git fetch. It usually resolves on its own / via ssh-agent. **Never type or route secrets through
  the agent.** If it blocks, press Enter (empty line) to let it fall through, or just re-run.
- Build log pattern used so far: `... >/tmp/crbuild.log 2>&1; echo "EXIT=$?"; tail -6 /tmp/crbuild.log`
  then grep the log for errors. `| tail` on a live bazel build buffers until completion — prefer
  redirecting to a file.

## Why this is big: upstream did coordinated renames

Between the old and new revs, **every** `caliptra-mcu-*` crate got a `caliptra-mcu-` package-name
prefix, `caliptra-sw` renamed its vendored `ureg`→`caliptra-ureg`, and `caliptra-dpe` renamed its
members (`dpe`→`caliptra-dpe`, `crypto`→`caliptra-dpe-crypto`, `platform`→`caliptra-dpe-platform`,
added `caliptra-dpe-response-buffer`) and reworked its features.

### The remap technique (already used throughout)
In `crates_io/{host,embedded}/Cargo.toml`, keep the **dependency key** stable and add
`package = "<new-name>"`. This preserves:
- Rust `use <key_underscored>` in first-party code, and
- the crate_universe **hub alias**, which is derived from the *dependency key*:
  - key with a dash → hub alias uses **underscores** when key ≠ package (a rename),
    e.g. key `mcu-error` + `package="caliptra-mcu-error"` ⇒ hub target `@…//:mcu_error`.
  - key == package ⇒ hub alias is the **dashed** package name, e.g. `@…//:caliptra-dpe`.
  - `crate.annotation(crate = …)` matches the **package name**, not the key.
  - a crate with an explicit `[lib] name` (e.g. `pldm-fw-pkg`) only gets the **package-name**
    hub alias (`@…//:caliptra-mcu-pldm-fw-pkg`), no underscore-key alias.

## Edits already applied (verify they're present)

**`third_party/caliptra/versions.bzl`**: `caliptra_mcu_sw`→476be194, `caliptra_sw`→85981e1b,
`caliptra_dpe`→cf3224c4, `caliptra_cfi` reverted to a98e499d.

**`third_party/caliptra/crates_io/host/Cargo.toml`** — `package=` remaps:
`registers-generated`→caliptra-mcu-registers-generated, `mcu-config[-emulator|-fpga]`→caliptra-mcu-…,
`flash-image`→caliptra-mcu-flash-image, `mctp-vdm-common`→caliptra-mcu-mctp-vdm-common,
`pldm-common`→caliptra-mcu-pldm-common, `pldm-fw-pkg`→caliptra-mcu-pldm-fw-pkg,
`pldm-ua`→caliptra-mcu-pldm-ua, `mcu-firmware-bundler`→caliptra-mcu-firmware-bundler,
`caliptra-util-host-mailbox-test-config`→caliptra-mcu-core-util-host-mailbox-test-config,
and added `emulator-state`→caliptra-mcu-emulator-state.

**`third_party/caliptra/crates_io/embedded/Cargo.toml`**:
- `package=` remaps: `mcu-error`, `romtime`, `registers-generated`, `mcu-config[-emulator]`,
  `mcu-image-header`, `flash-image` → `caliptra-mcu-*`.
- `ureg` → `package="caliptra-ureg"` (still git=caliptra-sw@85981e1b).
- dpe block → rev cf3224c4, `package="caliptra-dpe[-crypto|-platform]"`, added
  `caliptra-dpe-response-buffer`, added `caliptra-ocp-eat` (git=caliptra-sw). **dpe features are
  the current open question — see blocker.**
- Added `[patch."https://github.com/chipsalliance/caliptra-cfi.git"]` redirecting `caliptra-cfi-lib`
  and `caliptra-cfi-derive` to `git=caliptra-sw@85981e1b` (mirrors caliptra-sw's own `[patch]`; this
  is what fixes the dpe-vs-vendored cfi spoke collision — do not remove).

**`third_party/caliptra/MODULE.bazel`**: `crate.annotation(crate = "pldm-common")` → `"caliptra-mcu-pldm-common"`.

**`third_party/caliptra/BUILD.bazel`**: in the `crate_<name>` alias comprehension,
`"registers-generated"` → `"registers_generated"` (and its one referrer in caliptra-mcu-sw/BUILD.bazel:227
→ `crate_registers_generated`).

**`third_party/caliptra/caliptra-sw/BUILD.bazel`**:
- `caliptra_image_gen` deps: added `@rust_caliptra_crates_host//:p384`.
- `caliptra_drivers_runtime` deps: added `@rust_caliptra_crates//:caliptra-dpe` and
  `…//:caliptra-dpe-crypto` (dash form; the crates' Rust lib names are now natively
  `caliptra_dpe` / `caliptra_dpe_crypto`, so no `aliases` map is needed).
- `caliptra_runtime_lib` deps: dpe deps switched to dash form and added
  `…//:caliptra-dpe-response-buffer` and `…//:caliptra-ocp-eat`.

**`third_party/caliptra/caliptra-mcu-sw/BUILD.bazel`**:
- All renamed-crate hub refs converted dash→underscore (e.g. `:mcu_error`, `:registers_generated`,
  `:pldm_common`, `:mctp_vdm_common`, …), except `pldm-fw-pkg` → `@…//:caliptra-mcu-pldm-fw-pkg`.
- `emulator_caliptra` deps: added `//third_party/caliptra/caliptra-sw:caliptra_hw_model_types`.
- `mcu_testing_common` deps: added `@rust_caliptra_crates_host//:emulator_state`.

## CURRENT BLOCKER (start here)

`//third_party/caliptra/caliptra-sw:caliptra_runtime_lib` fails to compile with:
```
error[E0046]: not all trait items implemented, missing: `__cfi_derive_cdi`,
   `__cfi_derive_exported_cdi`, `__cfi_derive_key_pair_exported`, `__cfi_derive_key_pair`
```
The `Crypto` trait comes from `caliptra-dpe-crypto`; the impl is in
`caliptra-sw/runtime/src/dpe_crypto.rs`. The `__cfi_derive_*` methods only exist when dpe's `cfi`
feature is on.

**Leading hypothesis (try first):** the overlay `caliptra_runtime_lib` sets
`crate_features = ["emu", "fips_self_test"]` (no default-features), so the runtime crate's own
`cfi` feature is **off**. In upstream `runtime/Cargo.toml`, `caliptra-dpe/cfi` is only enabled via
the runtime `cfi` feature. Therefore dpe should be built **without** `cfi` here, so the trait has no
`__cfi_derive_*` methods and the impl (also no-cfi) matches.

Action: in `crates_io/embedded/Cargo.toml`, set the `caliptra-dpe` features back to
`["hybrid", "arbitrary_max_handles"]` (remove `"cfi"`). Then
`CARGO_BAZEL_REPIN=1 bazelisk build //target/veer/tooling:caliptra_runner`.

If instead upstream expects cfi **on** everywhere, the alternative is to enable the runtime `cfi`
feature on the overlay target(s) so the impl generates the methods — but the no-cfi route above is
the smaller change and matches the current overlay feature set. Confirm empirically.

## Error → fix playbook (iterate the build)

Run the build; for each failure class:

- **`error: no matching package named X`** — crate `X` was renamed. Find the new package name:
  `grep -rn '^name = ' $(grep -rl "\"<lib-or-path>\"" --include=Cargo.toml <repo>)`. Add
  `package = "<new>"` to the dep in the right `crates_io/*/Cargo.toml`, keeping the key. Repin.
- **`no such target '@…//:X' (did you mean Y?)`** — hub alias changed. Update the overlay
  `third_party/caliptra/**/BUILD.bazel` reference from `X` to `Y` (usually dash↔underscore, or to
  the `caliptra-mcu-<pkg>` package-name form for crates with an explicit `[lib] name`).
- **`Error: Unused annotations … name: "X"`** — a `crate.annotation(crate="X")` in
  `third_party/caliptra/MODULE.bazel` must use the new **package** name.
- **`unresolved import` / `unlinked crate <c>` while Compiling `<target>`** — the overlay
  `rust_library <target>` is missing a dep. Add the matching `@rust_caliptra_crates[_host]//:<c>`
  (dash or underscore per the alias rules). If `<c>` isn't declared anywhere, add it as a direct
  dep in `crates_io/{host,embedded}/Cargo.toml` first (pick host vs embedded by matching sibling
  deps in that target), then reference it.
- **`strip_prefix at <p> does not exist` / spoke collision** — two sources provide the same
  package name+version. Mirror caliptra-sw's `[patch."<source-url>"]` in the corresponding
  `crates_io/*/Cargo.toml` to unify them (already done for cfi).
- **trait/API mismatch (E0046/E0407/E0412/E0425/E0599, "cannot find … in crate")** — a
  version/feature mismatch. Re-check the pin and the enabled features against what the *consuming*
  caliptra-sw crate expects (read its `Cargo.toml` `[features]` and the upstream source under
  `<output_base>/external/+caliptra_repos+caliptra_sw`).

Known-good source-of-truth for cross-checking: the fetched caliptra-sw at
`<output_base>/external/+caliptra_repos+caliptra_sw` (its `Cargo.toml`, `Cargo.lock`, and crate
sources), and the local mcu-sw checkout at `/home/antrocha/work/otp/caliptra-mcu-sw`.

## After it builds

1. `bazelisk test //target/veer/tests/otp:emulator_test --test_output=all` — the OTP emulator test
   now runs the newer OTP model. Expect the `ReadBytes` and `ProgramBytes` sections to pass. The
   `CommitSvnFloor` section may still be under development (part of the *other* feature); if it
   fails, that's the OTP-service work, not the uprev — note it but don't block the uprev on it.
2. The uprev summary flagged manual follow-ups: verify no other caliptra-dependent openprot targets
   regressed from `caliptra_sw 2fe38a09→85981e1b`. A broad sanity check:
   `bazelisk build //third_party/caliptra/...` (and `//target/veer/...` if quick).
3. Commit **only** the uprev files (versions.bzl, the two `crates_io/*/Cargo.toml`, their
   `Cargo.lock`s if regenerated, `MODULE.bazel`, `MODULE.bazel.lock`, `BUILD.bazel`,
   `caliptra-sw/BUILD.bazel`, `caliptra-mcu-sw/BUILD.bazel`). Terse commit message. Do **not**
   stage markdown or the OTP-service files.

## Hard constraints

- Descriptive-but-terse commit messages. Never reference review "finding #N" in code/comments.
- Do not stage/commit markdown files (including this prompt) unless explicitly asked.
- Before any push, audit `git diff --name-only origin/main..HEAD | grep .md` for stray docs
  (compare against `origin/main`, not local `main`).
- Prefer local read-only `git -C <path> …` queries; avoid git-over-ssh where possible.
- Don't run multiple `bazelisk` commands in parallel; one at a time.
