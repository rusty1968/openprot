# HACE Behavioral Parity Goal (AST1060)

## Objective

The `ast10x0/peripherals/hace` port must reach **total behavioral parity** with the
**authoritative model**: the upstream Zephyr `aspeed_hace` driver — `hace_aspeed.c`
+ `hace_aspeed.h` + `hash_aspeed_priv.h` — from the AspeedTech-BMC Zephyr fork,
pinned at the exact `aspeed-zephyr-project/west.yml` revision
`zephyr @ cfe94dc149ffa0af7e1af668a27f57eecf0cd1e9`. A frozen verbatim copy of those
three files is the normative artifact at
[zephyr-reference/](zephyr-reference/) (see `zephyr-reference/PINNED_COMMIT.txt`);
all behavior claims below are grounded in that copy, read directly — not inferred
from `aspeed-rust`. This is the normative reference because it is the model the
deployed AST1060 firmware actually runs. See [zephyr-behavior.md](zephyr-behavior.md)
for the consumer/streaming analysis.

`aspeed-rust/src` (`hace_controller.rs`, `hash.rs`, `hash_owned.rs`, `hmac.rs`) is
**informative, not normative** — a second port of the same hardware, useful for
cross-checking register sequencing, but *not* the spec. Where `aspeed-rust` diverges
from the pinned Zephyr driver, the Zephyr driver wins and `aspeed-rust` is treated as
buggy.

Parity standard (decided): **observable byte-for-byte parity on every reachable
input** — the port reproduces the pinned Zephyr driver's output exactly for every
input any real consumer can produce (see [zephyr-behavior.md](zephyr-behavior.md)
consumer analysis), explicitly including the production-dominant 4 KB-aligned
streaming path. Where the Zephyr driver has a latent defect that no reachable input
triggers, the port keeps the safer/correct behavior; that single divergence is
recorded as the lone *intentional delta* (D2). Everything else is strict identity.

Scope is AST1060 only (little-endian Cortex-M). No other target is considered.

---

## 1. Reference behavior to replicate

### 1.1 Hardware contract

A hash pass is driven through six registers in this exact order:

1. `hace1c` — write `hash_intflag = 1` to clear the completion latch.
2. `hace20` — source address: SG-list pointer when `method & HACE_SG_EN`, else the
   linear `buffer` pointer.
3. `hace24` — digest in/out address (`ctx.digest`).
4. `hace28` — written with the **same digest address** as `hace24`.
5. `hace2c` — byte length of this pass.
6. `hace30` — command word (`method`); this write starts the engine.

Completion = `hace1c.hash_intflag` re-asserts. Cleanup/idle = write `hace30 = 0`.

### 1.2 Command word

`method = HACE_CMD_ACC_MODE(1<<8) | HACE_SHA_BE_EN(1<<3) | HACE_SG_EN(1<<18) | algo_bits`.

- ACC_MODE always set: digest buffer is live chaining state (engine reads it in,
  updates, writes back). This is what makes multi-call streaming work.
- SHA_BE_EN: digest emitted big-endian.
- SG_EN: source is the 2-entry SG descriptor list; `HACE_SG_LAST(1<<31)` OR'd into the
  final descriptor's length terminates it. **HMAC clears SG_EN** and uses the linear
  buffer — the `hace20` source-select branch is mandatory.
- Algorithm bit patterns (port the full table from `hace_controller.rs`):
  SHA1, SHA224, SHA256, SHA384, SHA512, SHA512_224, SHA512_256. Only SHA-256/384/512
  are reachable through aspeed-rust's public API and are the parity requirement;
  the others are defined but unwired in aspeed-rust and are out of scope.

### 1.3 Memory model

Single global `HashContext`, `#[repr(C, align(64))]`, in `.ram_nc`. SG descriptors,
`buffer`, and `digest` are DMA targets. Exactly one operation in flight at any time
across all API surfaces.

### 1.4 IV / endianness (AST1060)

- `copy_iv_to_digest` / `load_iv`: each IV `u32` written in **native (little-endian)**
  byte order into `digest`, length `iv.len() * 4` bytes. SHA-256 → 32 bytes;
  SHA-384/512 → 64 bytes (full digest buffer). Parity already holds
  (`to_ne_bytes` == aspeed-rust's pointer-cast on LE).
- Digest readback: the canonical observable output is the **standard digest byte
  string** as produced by aspeed-rust's scoped API and asserted by `hash_test.rs`.
  Port must yield this. Use `from_ne_bytes` packing so `Digest::as_bytes()` round-trips
  to the standard digest (see Delta D4).

### 1.5 Digest (hash) state machine

- **init**: set `method`; load IV into `digest`; set `block_size`; zero `bufcnt`,
  `digcnt`. Does **not** clear `buffer`/`digest` (scoped semantics).
- **update**:
  - Add `input.len()` into 128-bit counter (`digcnt[0]`, carry → `digcnt[1]`).
  - If `bufcnt + len < block_size`: append to `buffer`, return (no engine pass).
  - Else: build SG list of whole blocks only (sg[0] = buffered partial, sg[1] =
    block-aligned input span, last entry `HACE_SG_LAST`); one engine pass over the
    block-aligned `total_len`; copy tail (`len % block_size`) back to `buffer[0..]`.
- **finalize**: `fill_padding(0)`; single `HACE_SG_LAST` descriptor over the padded
  buffer; final pass; extract `digest_size` bytes; cleanup (zero `bufcnt`, `digcnt`,
  `buffer`, `digest`; `hace30 = 0`).

### 1.6 Padding (`fill_padding`)

Merkle–Damgård: write `0x80`, zero-fill to the 56-byte (block 64) / 112-byte
(block 128) boundary, then the message **bit-length** big-endian as an 8-byte field
(block 64) or 16-byte field (block 128, `high = (digcnt[1]<<3)|(digcnt[0]>>61)`,
`low = digcnt[0]<<3`). Already correct in `helpers.rs`; reused unchanged.

### 1.7 HMAC — NOT streaming  *(authoritative: `aspeed_hash_setkey` / `aspeed_hash_digest_hmac`)*

Grounded in the pinned driver, **not** `aspeed-rust/hmac.rs`. HMAC parity is
governed by a separate declared authority (§2.1) — this is the behavioral record.
**The port does not implement this engine-native path**: §3 item 2 implements HMAC
in software (RFC-2104 over the port's hasher). This subsection is a cross-reference
to confirm the chosen threshold/XOR match the driver's RFC-2104-correct behavior,
not an implementation target.

- **setkey** (`aspeed_hash_setkey`): if `key_len > block_size` (**RFC-2104-correct,
  algorithm-dependent: 64 for SHA-256, 128 for SHA-384/512** — *not* a flat 128)
  reduce the key via `aspeed_hash_digest` (one-shot H(K)), else copy the raw key
  into `key_buff`. Then zero-pad `key_buff` to `block_size`; set
  `ipad = opad = key_buff`; XOR the **full `block_size`** of `ipad` with `0x36`,
  `opad` with `0x5c`.
- **key reduction** (`aspeed_hash_digest`): clear `HACE_SG_EN` (Direct Access /
  linear `buffer`); load IV; single padded linear pass over the key;
  `key_buff = digest`; `key_len = digest_size`.
- **update** (one-shot, computes full HMAC of *this* input; last call wins):
  1. `digcnt[0] = block_size`; `bufcnt = block_size`; `buffer = ipad ‖ input`;
     `digcnt[0] += input.len()`; `bufcnt += input.len()`; clear `HACE_SG_EN`.
  2. `fill_padding(0)`; `copy_iv_to_digest`; engine pass → inner digest.
  3. `digcnt[0] = block_size + digest_size`; `bufcnt = block_size + digest_size`;
     `buffer = opad ‖ inner`; `fill_padding(0)`; `copy_iv_to_digest`; engine pass.
- **finalize**: read `digest_size` bytes; cleanup.

### 1.8 Owned move-based API (`hash_owned.rs`)

Same digest behavior as scoped, plus: `init` consumes the controller and returns an
owned context; `finalize` returns `(digest, controller)`; `cancel` cleans up and
returns the controller. On an exact block boundary the owned path sets `bufcnt = 0`
(see Delta D2 — the port adopts this for the scoped path too).

---

## 2. Deltas vs. the authoritative driver (source-verified @ cfe94dc)

All four rows are now grounded in the frozen [zephyr-reference/](zephyr-reference/)
copy, read directly. **D2 is the only intentional delta.** D1 and D4 turned out to
be *conformance*, not divergence (the earlier "vs. `aspeed-rust`" framing was
measuring against the wrong reference); they remain listed so the record is
unambiguous.

| ID | Pinned Zephyr driver (verbatim) | Port behavior | Classification |
|----|----------------------------------|---------------|----------------|
| D1 | **Bounded** wait: `aspeed_hash_wait_completion(dev, 3000)` → `reg_read_poll_timeout(…, 1, 3000)`, returns error on timeout (`hace_aspeed.c:380–393`) | Bounded `poll_budget` → `HaceError::Timeout`, `yield_fn` between polls | **Conformance, not a delta.** Authority already bounds the wait; port matches the observable failure semantics (error return, no partial output). Only the budget unit / yield differ — output-identical. |
| D2 | **Has the stale-`bufcnt` latent bug**: `aspeed_hash_update` updates `bufcnt` only when `remainder != 0`; on an exact block boundary `bufcnt` keeps its prior value (`hace_aspeed.c:489–492`) | Always `bufcnt = remaining` (correct SHA on every input) | **The lone intentional delta (decided: keep the fix).** Unreachable by any consumer: PFR streams 4 KB pages from `bufcnt==0` (boundary branch leaves it `0`, correct); DICE uses sub-block updates (never reaches the branch). Byte-identical to the authority on every reachable input; safer on the pathological one nothing produces. |
| D3 | Authority **does** implement HMAC (`aspeed_hash_setkey`/`aspeed_hash_digest_hmac`); key-reduction threshold is RFC-2104-correct `key_len > block_size` (`hace_aspeed.c:619–659`) | HMAC governed by a *separate* authority by deliberate choice (§2.1) | **Out of this goal's parity scope by decision** — not because the authority lacks HMAC (it doesn't). `aspeed-rust`'s flat `>128` is wrong and is *not* adopted. |
| D4 | Emits the canonical big-endian digest — raw `memcpy(pkt->out_buf, data->digest, digest_size)` of the `SHA_BE_EN` engine output (`hace_aspeed.c:513,583,614`) | `from_ne_bytes` everywhere → same canonical digest | **Conformance, not a delta.** Port output = authority output. `aspeed-rust` *owned*'s `from_be_bytes` byte-swap is an `aspeed-rust` bug the port must not adopt. |

The D2 verification obligation is **discharged**: the exact-block-boundary branch
was read directly from the pinned source and the consumer traces are in
[zephyr-behavior.md](zephyr-behavior.md) §5.

### 2.1 HMAC authority (separate by deliberate choice)

**Correction of record:** the authoritative Zephyr `aspeed_hace` driver *does*
implement HMAC (`aspeed_hash_setkey` + `aspeed_hash_digest_hmac`); the earlier
"Zephyr HACE has no HMAC" claim was wrong — it was based on the thin
`aspeed-zephyr-project` HAL wrapper, which simply never calls those driver ops, not
on the driver itself. Its key-reduction threshold is the **RFC-2104-correct,
algorithm-dependent** `key_len > block_size` (64 / 128), documented in §1.7.

By **explicit decision**, HMAC is governed by a *separate declared authority* rather
than byte-parity with the driver. That authority has two distinct, non-overlapping
parts — do not conflate them:

- **Algorithm correctness authority = the RFCs/KATs only.** RFC 2104 (construction)
  validated by published HMAC-SHA known-answer vectors (RFC 4231 / RFC 2202), with
  the pinned Zephyr driver (§1.7) as the *behavioral model* for the implementation
  shape (key reduction, full-`block_size` ipad/opad XOR, one-shot semantics). This
  is what fixes the `> block_size` key threshold (the driver's / RFC-2104's, **not**
  `aspeed-rust`'s flat `>128`).
- **API-surface authority = the openprot MAC trait** ([hal/blocking/src/mac.rs](../../../../../hal/blocking/src/mac.rs)).
  **Note (verified):** that trait is algorithm-agnostic — `MacInit`/`MacOp`/
  `MacAlgorithm`/`KeyHandle` constrain only the *interface* (init/update/finalize,
  opaque key handle, output type). It contains **no** RFC reference and mandates
  **no** HMAC semantics; no openprot spec document does either (only a glossary
  acronym in `docs/.../terminology.md`). So openprot imposes the *shape*, never the
  *algorithm*. Do not read "openprot requires RFC-2104" — it does not; RFC-2104 is
  *our* declared choice, sourced here.

The port's HMAC must therefore (a) pass the RFC-4231/-2202 KATs, (b) use the
RFC-2104-correct `> block_size` threshold, and (c) satisfy the openprot MAC trait
interface — three independent obligations from two authorities. HMAC
parity-vs-driver is *not* a gate of this goal; digest parity vs. the pinned driver
(§2 D1/D2/D4, §3–§4) is.

### 2.2 OPEN ISSUE — HMAC-SHA512 long-key (RFC-4231 #6/#7)

Status as of this investigation (QEMU `ast1030-evb`, KAT harness):

- **Green:** all SHA-256/384/512 digests (NIST one-shot, 4 KB-aligned streaming,
  D2 boundary); HMAC-SHA256 all RFC-4231 cases incl. streamed; HMAC-SHA384 all
  cases incl. the 131-byte > block-size key reduction; HMAC-SHA512 cases 1–4 and
  with a pre-reduced 64-byte key.
- **Failing:** HMAC-SHA512 with `key_len > 128` (RFC-4231 #6/#7) — a wrong but
  valid-looking SHA-512 tag (deterministic for a fixed binary; no crash).

Isolation (all verified by sub-tests in the KAT harness):

- `SHA-512(131-byte key)` standalone is **correct**.
- The full HMAC-SHA512 #6 math reconstructed via the public digest API is
  **correct** (`dbg sha512 hmac6 manual`).
- `hmac.rs` with a *pre-reduced* 64-byte key (non-reduce branch) is **correct**.
- Only `hmac.rs`'s reduce branch (3 chained SHA-512 sub-hashes, first = the
  131-byte key reduction) is wrong — **only** for the SHA-512 monomorphization
  (SHA-256/384 identical structure pass).
- The defect is **memory-layout-sensitive**: forcing `HmacKey` alignment moved
  the failing case (#6→#4); moving HMAC buffers into a `static` made a
  previously-green SHA streaming case HardFault. ⇒ this is a memory/DMA/aliasing
  fault whose victim shifts with layout, **not** a SHA-512 algorithm error.

Conclusion: black-box rebuild iteration is not converging and risks regressions.
Next investigation must use **instrumented tooling** — QEMU memory/DMA tracing or
GDB watchpoints on the corrupted region, and/or reading the upstream QEMU
`aspeed-hace` model for SG-DMA addressing/length constraints on 128-byte-block
(SHA-384/512) operations — not further speculative edits.

---

## 3. Implementation plan

1. **Algorithm spec generalization** (`digest.rs`, `constants.rs`, `context.rs`)
   - Add `HaceDigestSpec` impls for `Sha2_384`, `Sha2_512`: `HASH_CMD` (ACC|BE|SG|algo
     bits), `BLOCK_SIZE = 128`, IVs (`SHA384_IV`, `SHA512_IV` as `[u32;16]`),
     `digest_from_context` (48 / 64 bytes, `from_ne_bytes`).
   - Add the SHA-384/512 IV constants and digest-size constants.
   - Confirm `fill_padding` block-128 path (already present) is selected via
     `ctx.block_size`.
   - Acceptance: SHA-256/384/512 of `b"hello_world"` match the `hash_test.rs`
     expected vectors byte-for-byte.

2. **HMAC module** (new `hmac.rs` + `mod.rs` export) — **governed by §2.1.**
   **Decided implementation strategy: software RFC-2104 over the port's own HACE
   hasher**, *not* the engine's native HMAC ops. The openprot MAC trait is
   construction-agnostic (verified — [hal/blocking/src/mac.rs](../../../../../hal/blocking/src/mac.rs):
   no provided HMAC, no required hasher; only `MacOutput = Digest<N>` couples the
   output type to the digest module), so HMAC is implemented *on top of* the
   already-parity-verified `DigestInit`/`DigestOp` (§1.5). This is RFC-2104 by
   construction, trivially KAT-gated, and adds **no** engine-HMAC register code.
   The driver's `aspeed_hash_*_hmac` (§1.7) is a *behavioral cross-reference only*,
   not the implementation; `aspeed-rust`'s flat `>128` is rejected.
   - Implement `MacInit::init`: derive `K0` — if `key.len() > block_size`
     (**RFC-2104-correct**: 64 for SHA-256, 128 for SHA-384/512) reduce via one
     `DigestOp` pass `H(key)`, else use the raw key; zero-pad to `block_size`;
     form `ipad = K0 ⊕ 0x36`, `opad = K0 ⊕ 0x5c` over the full `block_size`;
     start the inner digest and absorb `ipad`.
   - `MacOp::update` → feed the inner `DigestOp`. `MacOp::finalize` →
     `inner = H(ipad ‖ msg)`, then `tag = H(opad ‖ inner)` via a fresh `DigestOp`;
     return `Digest<N>`. No `HACE_SG_EN` source-select / linear-buffer plumbing is
     needed (it was only for the engine-native HMAC path, now not taken).
   - `key`/`ipad`/`opad` may reuse `context.rs` scratch but are driven by the MAC
     module, not the engine HMAC mode.
   - Acceptance: HMAC-SHA256/384/512 match published RFC-4231/-2202 KATs; long-key
     (`> block_size`) reduces via `H(K)`; a 65–128 B SHA-256 key **is reduced**
     (not used raw — the `aspeed-rust` deviation we reject).

3. **Owned API module** (new `digest_owned.rs` + export)
   - Move-based init/update/finalize/cancel reusing the scoped state machine and
     `from_ne_bytes` packing (D4); `bufcnt = 0` on block boundary (D2).
   - Acceptance: chunked streaming across boundaries equals one-shot digest;
     `cancel` leaves a clean reusable context.

4. ~~**Register source-select** (`registers.rs`)~~ — **dropped.** The
   `method & HACE_SG_EN` linear-buffer source-select was only needed for the
   engine-native HMAC/key-hash path. With software HMAC (item 2) the port never
   clears `HACE_SG_EN`, so SG is always on and `src_addr` is unconditionally the
   SG-list pointer. No change to `program_hash_operation` is required.

5. **Parity test harness** — keyed to the authoritative Zephyr workload
   - SHA-256/384/512 KAT vectors (= pinned-Zephyr output; standard digests).
   - **Gating test**: the production path — `start → update(4096)×N → update(tail)
     → finish` — asserted byte-equal to standard SHA. (On this path the authority's
     D2 branch is dormant, so port == authority == standard SHA.)
   - **D2 delta test**: the pathological pattern (non-empty buffer then an update
     that exactly completes to a block multiple) — assert the port produces correct
     SHA and document that the authority would here produce a different (stale-
     `bufcnt`) value. This is the one place port ≠ authority, by decision; no
     consumer reaches it.
   - D1/D4 require no divergence assertions — they are conformance (§2).
   - HMAC KATs live in the HMAC component's harness (§2.1), not here.

## 4. Done criteria

- SHA-256/384/512 digest: byte-identical to the **pinned Zephyr driver**
  ([zephyr-reference/](zephyr-reference/) @ `cfe94dc`) for every reachable input,
  explicitly the production-dominant `start → update(4096)×N → update(tail) →
  finish` path — verified against the frozen `hace_aspeed.c` directly.
- Owned API: emits the same canonical digest as the scoped API (= Zephyr output);
  the `aspeed-rust` owned-API `from_be_bytes` byte-swap is **not** reproduced (D4
  conformance).
- D2 is the **only** intentional delta — covered by a positive test (port = correct
  SHA) plus a documented note that the authority diverges on the unreachable
  pathological pattern. D1/D4 are conformance, no divergence tests needed. No other
  behavioral difference vs. the pinned driver on any reachable input.
- HMAC: gated by published RFC-4231/-2202 KATs and the RFC-2104-correct
  `> block_size` threshold (§2.1) — not by byte-parity with the driver, by decision.
