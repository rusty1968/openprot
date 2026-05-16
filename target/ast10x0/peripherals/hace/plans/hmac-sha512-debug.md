# Debug Runbook — HMAC-SHA512 long-key wrong tag (unassisted-agent procedure)

Self-contained playbook for an autonomous agent to root-cause and fix the one
open HACE defect. Read [goal.md](goal.md) §2.1/§2.2 and
[zephyr-behavior.md](zephyr-behavior.md) first. Follow this **in order**; obey
the guardrails — the failure is memory/state-sensitive and prior blind
edit→rebuild loops *regressed* working cases.

---

## 0. Guardrails (read first, non-negotiable)

1. **One variable per iteration.** Change exactly one thing, rebuild, run the
   full KAT suite, record the result before the next change.
2. **Never thrash.** Max 6 instrumented iterations. If not converged, stop and
   write findings into goal.md §2.2 — do **not** keep guessing.
3. **Regression gate.** After *every* change the full suite must keep all
   currently-green cases green (§5). A change that regresses anything is
   reverted immediately.
4. **Known-good baseline exists** (§6). If the tree gets worse than baseline,
   restore it before continuing.
5. **Instrument, don't speculate.** Decisions come from QEMU trace output, not
   from reasoning about what "should" happen. The model is a heuristic shim,
   not faithful silicon — intuition has repeatedly been wrong here.
6. **Authority unchanged.** The pinned Zephyr driver
   ([zephyr-reference/](zephyr-reference/)) is normative for digest behavior;
   RFC-4231 KATs are normative for HMAC (goal.md §2.1). A "fix" that breaks
   parity with either is not a fix.

---

## 1. Symptom & what is already proven

`HMAC-SHA512` with `key_len > 128` (RFC-4231 #6/#7) yields a wrong but
valid-looking SHA-512 tag. Deterministic for a fixed binary; no crash in the
current (baseline) `hmac.rs`.

Proven by sub-tests already in the harness
(`target/ast10x0/tests/peripherals/hace/hace_sha256/target.rs`):

| Sub-test (in suite, runs before #6) | Result | Implies |
|---|---|---|
| `sha512 key6-2block` (SHA-512 of the 131 B key, public API) | PASS | SHA-512 of that exact input is correct |
| `dbg sha512 hmac6 manual` (reduction+inner+outer, **all via public digest API**) | PASS | The full HMAC-SHA512 #6 computation is correct on this engine when driven from the test |
| `hmac-sha512 prereduced6` (64 B pre-reduced key → `hmac.rs` non-reduce branch) | PASS | `hmac.rs` inner+outer composition is correct |
| `hmac-sha512 rfc4231-6` (`hmac.rs` reduce branch) | **FAIL** | Only the 3-chained-sub-hash path through `hmac.rs` is wrong, only for SHA-512 |
| `hmac-sha256/384 rfc4231-6/7` (same structure) | PASS | Not an HMAC-logic bug; algorithm-content-dependent |

Layout sensitivity (forcing `HmacKey` alignment moved the failing case #6→#4;
moving buffers to a `static` made an unrelated SHA case HardFault) ⇒ memory /
engine-state dependence, **not** a SHA-512 arithmetic error.

---

## 2. Leading hypothesis (from reading the QEMU model)

QEMU `hw/misc/aspeed_hace.c` (in `/home/ferro/work/qemu-ast10x0-i2c`, the
emulator the harness runs) does **not** emulate the engine faithfully. In ACC
mode (`HASH_DIGEST_ACCUM`, BIT(8)) it:

- Keeps **engine-global state** `s->hash_ctx` and `s->total_req_len`
  ([L373-417](../../../../../../qemu-ast10x0-i2c/hw/misc/aspeed_hace.c#L373)).
  `hash_ctx` is created only when `NULL` and freed/reset **only on a request
  detected as "final"**; `total_req_len` resets only then.
- Decides "final" via the heuristic `has_padding()`
  ([L153-185](../../../../../../qemu-ast10x0-i2c/hw/misc/aspeed_hace.c#L153)):
  reads the **last 8 bytes** of the request as a big-endian length `/8`, and if
  `total_msg_len <= s->total_req_len` **and** `padding[pad_offset] == 0x80`,
  strips the firmware padding and treats it as the final block.
- Engine-native HMAC is **unimplemented**
  ([L508-512](../../../../../../qemu-ast10x0-i2c/hw/misc/aspeed_hace.c#L508)) —
  confirming the software-HMAC-over-ACC-digest design (goal.md §2.1) was
  required, not optional.

**Hypothesis H1 (primary):** for the HMAC-SHA512 long-key sequence (three
chained ACC digests: 131 B key reduction → inner `H(ipad‖msg)` → outer
`H(opad‖inner)`), one sub-hash's finalize pass is **mis-classified by
`has_padding`** (false negative), so `hash_ctx`/`total_req_len` are **not
reset** and leak into the next sub-hash, which skips `qcrypto_hash_new` and
**continues the stale context** → valid-looking wrong digest. SHA-256/384
produce different padded-tail bytes that don't trip the heuristic.

**Hypothesis H2 (secondary):** a non-final prefix pass is mis-classified as
final (false positive), finalizing a sub-hash early on truncated input.

Both are directly observable in trace; do not assume which — measure.

---

## 3. Reproduction (exact)

```bash
cd /home/ferro/work/peripherals/openprot
bazelisk build --config=virt_ast10x0 \
  //target/ast10x0/tests/peripherals/hace/hace_sha256:hace_sha256
# image (ELF) is produced at:
IMG=bazel-bin/target/ast10x0/tests/peripherals/hace/hace_sha256/hace_sha256.elf
```

Run **directly under the local instrumentable QEMU** (owns the model source;
do NOT depend on the bazel/cipd qemu for debugging):

```bash
QEMU=/home/ferro/work/qemu-ast10x0-i2c/build/qemu-system-arm
"$QEMU" -machine ast1030-evb -cpu cortex-m4 -bios none -nographic \
  -serial mon:stdio -semihosting-config enable=on,target=native \
  -kernel "$IMG" 2>&1 | tee /tmp/hace_run.log
# Suite prints "case: <name>" then "<name>: PASS" / ": digest mismatch".
# It self-terminates on TEST_RESULT; if it hangs, kill after ~30 s.
```

The console interleaves with engine trace once instrumented (§4); correlate by
the `case:` markers (each HMAC case = its sub-hashes between two `case:` lines).

---

## 4. Instrumentation procedure (the core loop)

The model has trace points but the surest, fully-owned method is to add
`fprintf(stderr, …)` to the model and rebuild the local QEMU.

### 4.1 Add probes to `hw/misc/aspeed_hace.c`

In `/home/ferro/work/qemu-ast10x0-i2c/hw/misc/aspeed_hace.c` add prints
(guard with an env check, e.g. `getenv("HACE_DBG")`, so normal runs are quiet):

- In `has_padding()` just before each `return`: print
  `algo? n/a`, `req_len (plen)`, `s->total_req_len`, `*total_msg_len`,
  `padding_size`, `*pad_offset`, `padding[pad_offset]`, and the return value.
- In `hash_execute_acc_mode()`: print on entry `algo`, `iov_idx`, summed
  iov length, `s->hash_ctx == NULL?`, `final_request`; and after finalize the
  first 8 + last 4 bytes of `digest_buf`.
- In `do_hash_operation()`: print a monotonically increasing `op#`.

Keep prints one-line, prefixed `HACEDBG`, so they grep cleanly.

### 4.2 Rebuild only QEMU (fast, ~1–3 min)

```bash
cd /home/ferro/work/qemu-ast10x0-i2c
ninja -C build qemu-system-arm        # or: make -C build qemu-system-arm
```

### 4.3 Capture the #6 sequence

```bash
HACE_DBG=1 "$QEMU" -machine ast1030-evb -cpu cortex-m4 -bios none -nographic \
  -serial mon:stdio -semihosting-config enable=on,target=native \
  -kernel "$IMG" 2>&1 | tee /tmp/hace_dbg.log
```

Slice the log to the three sequences that bracket the bug and **diff their
trace**, since they run the *same* sub-hash shapes:

- `dbg sha512 hmac6 manual` — PASSES (reference "correct" trace).
- `hmac-sha384 rfc4231-6` — PASSES (same structure, different algo).
- `hmac-sha512 rfc4231-6` — FAILS.

### 4.4 What to determine (decision points)

For the **failing** `hmac-sha512 rfc4231-6`, for each of its 3 sub-hashes
(reduction over 131 B; inner over `128+54`; outer over `128+64`):

- D1: Is `final_request` set **exactly once**, on the padded finalize pass of
  each sub-hash? (Expected: yes.)
- D2: Between sub-hashes, is `hash_ctx` freed (i.e. next sub-hash's first pass
  sees `hash_ctx == NULL`)? If a sub-hash's first pass sees non-NULL →
  **H1 confirmed** (stale-context leak): identify which prior finalize failed
  `has_padding` and why (`total_msg_len`, `total_req_len`, `pad_offset`,
  `padding[pad_offset]`).
- D3: Is any **prefix** (non-final) pass classified final? → H2.
- D4: Compare the same fields against the PASSING `dbg sha512 hmac6 manual`
  trace. The first field that differs is the root cause locus.

---

## 5. Decision tree → candidate fixes

Pick the branch the trace proves; re-verify with §3 + full regression (§6).

- **A. `has_padding` false-negative on a finalize pass (H1).** The firmware's
  padded final block isn't recognized. Root cause is the model's heuristic vs.
  the port's padding/length encoding. Allowed fixes, in priority order:
  1. **Fix the QEMU model** (we own the source). If the heuristic is genuinely
     wrong for valid Merkle–Damgård SHA-512 padding the port emits (which
     matches the pinned Zephyr driver — verify against
     [zephyr-reference/hace_aspeed.c](zephyr-reference) `aspeed_ahash_fill_padding`),
     correct `has_padding`/state handling and **document the QEMU-model
     divergence + that real silicon/Zephyr is unaffected** in goal.md §2.2.
     This keeps the port faithful to the authority.
  2. Only if the port's padding actually deviates from the Zephyr reference:
     fix the port (`helpers.rs::fill_padding` / `digest.rs::finalize`) to match
     the reference exactly, then re-run all KATs.
  3. Do **not** "tune" the port to placate the heuristic at the cost of
     parity with `zephyr-reference`.
- **B. `has_padding` false-positive on a prefix pass (H2).** Same: prefer
  fixing the model; the port must keep emitting reference-faithful blocks.
- **C. Trace shows the port itself feeds wrong bytes/SG/length** (e.g. the
  reduction sub-hash hashes the wrong region). Then it *is* a port bug in
  `hmac.rs` or `digest.rs`; fix there and re-verify against
  `zephyr-reference` + RFC-4231.

After any model edit: `ninja -C build qemu-system-arm`, then run the suite via
the local QEMU **and** via bazel (`bazelisk test --config=virt_ast10x0 …`) —
note if the bazel/cipd QEMU differs from the local one (it may be a different
build; if so, the real fix must land where the harness's QEMU is sourced, or
the model fix must be upstreamed to that QEMU. Record this explicitly.)

---

## 6. Regression gate & known-good baseline

**Must stay green** (all currently pass on QEMU `ast1030-evb`):

- All SHA-256/384/512: `empty/abc/nist-448/nist-896`, `stream-9000` (256/384/512),
  `d2-boundary`, `key6-2block` (384/512), `dbg sha512 inner`,
  `dbg sha512 hmac6 manual`.
- HMAC-SHA256: RFC-4231 `1,2,3,4,6,7` + `streamed-7`.
- HMAC-SHA384: RFC-4231 `1,2,3,4,6,7`.
- HMAC-SHA512: `prereduced6`, RFC-4231 `1,2,3,4`.

**Target:** add `hmac-sha512 rfc4231-6` and `…-7` to the green set with **zero**
regressions and `TEST_RESULT:PASS`.

**Baseline:** the current `hmac.rs` is the deterministic known-good (one-shot,
`from_device`/`DigestInit`, no `static`, no forced alignment): suite is all
green except #6/#7 clean mismatch, **no crash**. `hmac.rs` is git-untracked, so
**before editing it, copy it**:
`cp target/ast10x0/peripherals/hace/hmac.rs /tmp/hmac.rs.baseline`. Restore with
`cp /tmp/hmac.rs.baseline …/hmac.rs` if a firmware change regresses. For QEMU
model edits, use `git -C /home/ferro/work/qemu-ast10x0-i2c diff` /
`git stash` to revert cleanly.

---

## 7. Success criteria & exit

Done when **all** hold:

1. Full KAT suite under QEMU prints `TEST_RESULT:PASS` (HMAC-SHA512 #6/#7 pass,
   nothing regressed).
2. Root cause written into goal.md §2.2 with the trace evidence (which pass,
   which field, why), and whether it was a QEMU-model defect or a port bug.
3. If a QEMU-model fix: state explicitly whether real silicon / the pinned
   Zephyr driver are affected (cross-checked against `zephyr-reference`), and
   whether the harness's QEMU (bazel/cipd) also needs the fix.
4. The temporary diagnostics in `target.rs` (`dbg sha512 …`, `prereduced6`,
   `key6-2block`) are either kept as permanent regression tests (preferred) or
   removed deliberately — not left half-in.
5. `hmac.rs` doc-comment "KNOWN ISSUE" note updated/removed to match reality.

If criteria 1 is unreachable within the iteration budget: leave baseline
restored, record the dead-ends and the narrowed locus in goal.md §2.2, and
stop. A precisely-narrowed open bug is an acceptable terminal state; a
regressed tree is not.

---

## 8. Key references

| What | Where |
|---|---|
| Open-issue status / authority rules | [goal.md](goal.md) §2.1, §2.2 |
| QEMU model: `has_padding` | `qemu-ast10x0-i2c/hw/misc/aspeed_hace.c:153-185` |
| QEMU model: ACC exec + state | `…/aspeed_hace.c:373-417` (`hash_execute_acc_mode`) |
| QEMU model: SG iov prep | `…/aspeed_hace.c:244-310` |
| QEMU model: cmd dispatch / HMAC unimpl | `…/aspeed_hace.c:504-538` |
| QEMU trace events | `qemu-ast10x0-i2c/hw/misc/trace-events:309-315` |
| Port HMAC (software RFC-2104) | `target/ast10x0/peripherals/hace/hmac.rs` |
| Port digest stream/finalize | `target/ast10x0/peripherals/hace/digest.rs` |
| Port padding (must match Zephyr) | `target/ast10x0/peripherals/hace/helpers.rs::fill_padding` |
| Authoritative padding reference | `plans/zephyr-reference/hace_aspeed.c::aspeed_ahash_fill_padding` |
| KAT harness + bracketing diagnostics | `target/ast10x0/tests/peripherals/hace/hace_sha256/target.rs`, `vectors.rs` |
| Local instrumentable QEMU | `/home/ferro/work/qemu-ast10x0-i2c/build/qemu-system-arm` (v10.2.94, machine `ast1030-evb`) |
| Harness QEMU runner (arg passthrough: `--qemu_args`) | `target/ast10x0/harness/qemu_runner.py` |
