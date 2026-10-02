# Firmware-update throughput: what we found, in plain language

This summarizes a study of how fast OpenPRoT applies a firmware update over the
I3C bus, where the time goes, what we sped up, and what's left. It's written for
readers who aren't in the code.

## What we were measuring

A firmware update is sent to the device in **chunks** over the I3C bus. The
device asks the host "send me the next chunk," the host sends it, the device
stores it, and it repeats until the whole image has arrived. We built a benchmark
that runs a full update on the chip **emulator** and reports the download speed in
bytes per second — the same way the related upstream project (caliptra-mcu-sw)
measures its own update speed, so the method lines up.

**Important caveat up front:** these numbers come from an emulator, not real
silicon. The emulator models a slow (1 MHz) CPU and adds its own overhead, so the
absolute bytes/sec is *not* a real-hardware figure. The numbers are only
meaningful as **before/after comparisons** — "this change made it 2x faster" —
not as "the device does X KB/s."

## Where the time actually goes

The single most important finding: **the device spends almost all of its time
waiting, not working.** We measured a download and found ~**94%** of the time is
the device sitting idle waiting for a chunk to make the round trip from the host,
and only ~6% is actual work between chunks. The firmware's own computation is
negligible.

In other words, the bottleneck isn't the code doing too much — it's the
**number of back-and-forth round trips**, each of which has a large fixed
waiting cost (bus + the messaging handoffs between internal components).

## What we changed, and what it bought

1. **Bigger chunks (the big win).** Fewer, larger chunks mean fewer round trips
   for the same image. This roughly **tripled** throughput (about 475 → ~1300
   bytes/sec). It had been blocked by a placeholder size limit buried in a
   library dependency (a hard-coded "512" with a literal comment saying "define
   an appropriate size"); raising it unlocked the gain.

2. **Reliability fixes.** Along the way the update was only succeeding about 1 in
   5 times. Two fixes made it reliable (now passes every run): a small **buffer
   ring** so incoming data fragments can't overwrite each other, and a fix for a
   case where the device could get **stuck waiting** for an acknowledgment that
   never came. These matter more than the speed numbers — an update that usually
   fails isn't useful at any speed.

## The isolation tradeoff

For security, OpenPRoT keeps the **I3C driver** and the **messaging layer (MCTP)**
in separate, protected compartments — so a bug in one can't corrupt the other.
That separation has a cost: every chunk has to be handed across that boundary,
which isn't free.

We measured it by building a variant that **merges** those two compartments into
one. Result: about **28–35% faster**. But merging them removes exactly the
protection the separation provides — a bug in the bus driver could then reach the
messaging state directly. For a security chip that's generally the wrong trade, so
we kept the merged version as a **measured experiment, not a shipped change**. The
related upstream project does merge them (and runs them at higher privilege); that
's part of why its path is a bit cheaper, and it's a deliberate design difference.

## How we compare to the upstream project

Same order of magnitude, measured the same way — and that's all we can honestly
say. It is **not** a clean head-to-head: different emulators, a different test
driver, and a different firmware-update implementation. Our numbers landing near
theirs is reassuring (we measure the right thing) but doesn't mean one is "faster"
than the other.

## The biggest remaining opportunity

Because ~94% of the time is *waiting*, the highest-leverage improvement is
**pipelining**: let the device ask for the next chunk (or several) before the
previous one has arrived, so the waiting overlaps instead of happening one chunk
at a time. Our measurement says this could be worth roughly **3–4x** more at a
modest overlap depth.

Two honest qualifiers:
- It's the most involved change — it touches several layers and needs the
  messaging to handle more than one outstanding request at a time (today it's
  strictly one-at-a-time).
- The *realized* gain depends on whether the host and bus can actually keep
  several requests in flight; our measurement proves the device side has the
  headroom, but not that the other side can fill it.

## Recommendation

- **Ship** the reliability fixes and the bigger-chunk change — clear wins, no
  downside.
- **Treat merging the driver and messaging as a security decision**, not a
  performance one; the ~28–35% is real but costs isolation.
- **Pipelining is the next big lever** and is now justified by data (~3–4x
  potential), but it's a project, not a tweak — worth scoping deliberately.
- Everything else we looked at (the firmware device's own logic, the I3C driver)
  is already near-optimal; the remaining cost is structural (round trips and the
  by-design isolation), not wasteful code.

See `ipc_isolation_cost.md`, `pldm_throughput_colo/PR_DISCUSSION.md`, and the
per-test READMEs for the detailed numbers and commands.
