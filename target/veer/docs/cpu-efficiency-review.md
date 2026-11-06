# CPU-efficiency review: PLDM over MCTP over I3C (OpenPRoT, VeeR 17.91 MHz)

Review of the working tree on branch `mctp-i3c-fpga-bench` (with uncommitted trace instrumentation). Read-only: nothing was edited, built or run. Cycle figures are estimates and suspects to check against the trace, unless marked as taken from `trace-run.log`. The ms printed by the trace script are 100x too large and were ignored; the corrected per-chunk numbers from the task brief were used.

## Trace observations (from trace-run.log, core cycles)

| Segment | Cycles (median) |
|---|---|
| ack -> frame decoded, 68 B frame | ~365k (very consistent, 18 samples) |
| 13 B frame, warm | ~53k |
| 13 B frame, cold | ~279k |
| ack -> callback exit, IBI-done callback, nothing decoded | ~457k |
| final `recv` returning NoData in drain loop | ~27k |
| `on_interrupt` (1 MMIO read + `debug_clock_now` syscall) | ~7.8k |
| `interrupt_ack` syscall | ~29.5k |
| MCTP inbound, 68 B middle fragment | ~31k |
| MCTP inbound, last 13 B fragment completing the message | ~257k **[WRONG, corrected 2026-10-25: measured about 30k cycles in the traces (n=21 to 63) and 23k in an isolated board benchmark, the same as a middle fragment]** |

Conclusions:
- 68 B costs 365k and a warm 13 B costs 53k: a clean ~5.4k cycles/byte with no fixed term.
- The same recv path with nothing to decode costs 27k in the drain loop, so the 457k in the IBI-done callback is not recv work.
- A NoData recv is ~500 instructions in 27k cycles, i.e. an effective CPI of ~50 (inferred).

## Ranked findings

### 1. Bitwise CRC-8 on every byte (confirmed by trace linearity)
- Where: `services/mctp/transport-i3c/src/encap.rs:45-57` (`crc8_step`), `:76-82` (`transfer_pec`), `:142` (RX check in `decode_write`), `:199` (TX).
- Mechanism: 8-iteration branchy loop per byte, ~60-90 instructions at opt-s. At CPI ~50 that gives the measured ~5.4k cycles/byte.
- Cost: 217 RX bytes per chunk, ~1.1M cycles, ~60 ms (~15% of a chunk). TX PEC on ~17 B is ~90k cycles inside the 85.7 ms send segment.
- Explains: the 136 ms receive phase; ~20 ms per 68 B fragment.
- Fix: const 256-entry table (~8x cheaper; 16-entry nibble table as an alternative). Saves ~50 ms/chunk on RX, ~4 ms on TX. Also check whether the I3C core offers hardware PEC check.

### 2. Effective CPI of ~50 (inferred; probably the largest multiplier)
- Evidence: syscall + MMIO read = 7.8k cycles; `interrupt_ack` = 29.5k; NoData recv ~500 instr = 27k.
- Linker layout: `target/veer/target.ld.jinja:16-21`, `target/veer/tooling/render_system_config.rs:36-46` put kernel code, data, bss and stack in MCU SRAM (0xa8c... window, over AXI). Nothing uses ICCM/DCCM except the vector table (`config.rs:79`).
- Open question (not checked): whether I/D-cache covers that region depends on MRAC CSR (0x7c0), set by the MCU ROM (`third_party/caliptra/caliptra-mcu-sw/src/generate_rom_ld.rs` has `MRAC_VALUE`). Also unchecked: whether the native stack runs from ICCM/DCCM.
- Check: time a register-only loop with `rdcycle` (e.g. 1M `add`s); read MRAC at kernel entry.
- Fix if CPI is high: link hot text into ICCM and stacks into DCCM, or make the SRAM window cacheable. Scales every segment.

### 3. Unexplained 230-450k-cycle windows after `interrupt_ack` (speculation)
- Seen: IBI-done callback (457k, nothing decoded) and some 13 B frame callbacks (279k vs 52k warm). Values are very regular (279.2k, 457.3k), so likely deterministic, not jitter.
- Candidates: (a) FD thread running after the Send reply (`channel_respond`) and being scheduled around the ack; (b) kernel IRQ/scheduler work around the ack; (c) cold-code penalty.
- Cost: 2-3 such windows per chunk, ~0.7-1.0M cycles (~40-55 ms).
- Check: trace tag right after `object_wait` returns in the callback; record which thread owns the CPU; count tag 1 (irq entry) per chunk (currently 5: 1 IBI-done + 4 inbound, so no IRQ storm).

### 4. IPC round trips and per-call 1 KB zero-fill (confirmed by reading)
- FD side, per chunk: (1) `stack.req` (`services/mctp/api/src/stack.rs:68`), (2) `send`, (3) `recv` (blocking, deferred), (4) `drop_handle`/unbind (`stack.rs:141`), (5) responder `recv` poll.
- Server side: ~5 requests, 5 IRQ wakes and 1 deadline wake = ~11 passes of `mctp_server_runtime::run`; at least 22 context switches, plus `wait_group_remove`/`wait_group_add` on every deferred `Recv` (`util/service/lib.rs` `defer`/`complete`).
- Zero-fill: `services/mctp/client-ipc/src/lib.rs:91-96` builds a fresh `[0u8; 1032]` and copies `request_buf` into it on every IPC call (~5 KB memset + memcpy per chunk, ~25-40 ms at observed CPI).
- Fix: borrow the two fields of `inner` separately and pass `&inner.request_buf[..req_len]` directly.
- Fix: reuse one request handle across chunks. `StackReqChannel` already allows a second send/recv after `recv` resets `sent_tag`, but `MctpPldmTransport::send_request` (`services/pldm/src/transport.rs:111-126`) creates and drops a channel per call. Saves 2 of 5 IPCs, including the unbind inside the 68.9 ms "last fragment -> FD gets data" segment.
- Explains: the 85.7, 68.9 and 56.1 ms segments.

### 5. 1 ms responder poll after every chunk (confirmed by reading)
- Where: `services/pldm/src/firmware_device.rs:64`, `:217-222`. While the initiator is active, every step ends with `listener.set_timeout(1)` and `respond_once`.
- Mechanism: deferred `Recv` with effective ~2 ms wait (`instant_at` adds 1 ms, `mctp/server-runtime/src/lib.rs:178-182`): ~36k cycles of idle wait per chunk plus the defer, deadline wake, timeout encode and `complete` path, run at the slow rate.
- Explains: part of the 56.1 ms "FD between download_fw_data and next request".
- Fix: poll the responder every N chunks (e.g. 16), or skip it for the bench. `timeout 0` means infinite, so a truly non-blocking recv is not available; rate-limiting is simplest.

### 6. Copies and zero-fills (confirmed by reading; small each, add up)
Copies of a 196 B chunk, end to end (8):
1. HW FIFO -> ring slot (`target/veer/peripherals/i3c/lib.rs:148-156`)
2. ring -> `resp` in `dispatch` Recv (`services/i3c/server/src/lib.rs:205`)
3. `resp` -> `i3c_rx_buf` (`services/i3c/client/src/lib.rs:133-135`)
4. fragment -> reassembly `Vec` (`mctp-estack` `reassemble.rs:208`, `extend_from_slice`)
5. `Vec` -> `recv_buf` (`services/mctp/server/src/server.rs:131` or `:272`)
6. `recv_buf` -> `response_buf` (`wire::encode_recv_response`)
7. kernel copy into FD `response_buf`
8. `response_buf` -> FD `fw_buf` (`client-ipc` `recv`, `:170`)

TX side has ~3 copies. Many are at odd offsets (1 B header, 4 B MCTP header); on rv32 this may drop `compiler_builtins` memcpy to a byte loop (speculation).

Per-call zero-fills (~10 KB per chunk total):
- `I3cClient::transact` zeros `req[251]`; `recv`/`send`/`dynamic_address` zero `resp[251]` (`services/i3c/client/src/lib.rs:94, 108, 128, 148`).
- `I3cSender::send_vectored` zeros `pkt[249]` and `out[250]` on every loop iteration, including the final `Complete` one (`services/mctp/transport-i3c/src/sender.rs:95, 98`).
- `Server::latch_inbound` has a 250 B scratch array on the full-ring path only.

Fixes:
- Colo-only: add `Server::pop_frame() -> Option<&[u8]>` and decode straight from the ring slot. Removes copies 2-3, both zero-fills and the dispatch encode.
- Hoist `pkt`/`out` out of the loop; use `MaybeUninit` or a persistent buffer in the client.
- Cache the dynamic address in the sender (read once per message today).
- Estimated saving: ~1M+ cycles at CPI 50 (guess).

### 7. Server loop overhead per wake (confirmed by reading)
- Where: `services/mctp/server-runtime/src/lib.rs:83-120`, `services/mctp/server/src/server.rs:254-297`.
- Mechanism: up to 4 `SystemClock::now()` + `as_millis()` per pass (u64 division on rv32, `__udivdi3`); two `server.advance()` calls; `Server::update` returns a `heapless::Vec<(Handle, RecvResult), 16>` by value each pass and clears/iterates `outstanding`; `drive_pending` runs on every event including IRQs.
- Cost: ~1-3k instructions per pass x ~11 passes; at CPI 50 up to several million cycles per chunk.
- Fix: read the clock once per pass; skip `drive_pending` when `outstanding` is empty and the event was an IRQ that fed no packet; return a count or small fixed array instead of a 16-entry `Vec`.

### 8. I3C driver MMIO (confirmed; minor)
- Where: `target/veer/peripherals/i3c/lib.rs:130-138`. Per frame it reads `tti_interrupt_status`, `tti_desc_queue_depth`, the descriptor, 17 data words, and the latch loop calls once more per burst returning None. The byte loop `for &b in &word.to_le_bytes()` does `get_mut(i)` per byte.
- Measured: RX drain 19k cycles median per frame (up to 300k); ~1-2k cycles per MMIO read, ~20 reads per frame.
- Fix: skip the `status_set` read when the depth register is implemented; write whole words (`buf[i..i+4].copy_from_slice`). ~1% saving.
- PEC and delays: the driver does no PEC (it is in `encap.rs`) and has no delays.

### 9. Send 11-20 B ~15 ms vs 36-100 B ~2 ms (speculation; not explained by reading)
- `send` is `tx_write` (descriptor + 3-5 data words) then `ibi_raise` (descriptor + one word). Cost does not scale with length, so loops cannot explain 15 vs 2 ms.
- Likely cause: an MMIO write stalling on hardware state (TX queue or IBI port busy). The small slow sends are steady-state RequestFirmwareData right after the host's multi-fragment write; the larger fast ones are negotiation messages on an idle bus.
- Check: trace points between descriptor write, data words, and IBI descriptor/payload writes; compare steady-state vs negotiation. If the IBI write is slow, check whether the previous IBI's IbiDone had been serviced.

### 10. Trace instrumentation overhead (confirmed by reading)
- Each `rec()` is a `debug_clock_now` syscall (several thousand cycles); ~25 events per chunk, so measured segments include 100k+ cycles of it.
- `dump_some` prints 4 UART lines per wake after the buffer fills (`mctp_i3c_colo.rs:78-91`, `pldm_fd.rs:115-132`), skewing chunks after ~25; matches the 148 s vs 122 s seen.

## Not the problem
- FD callbacks and message buffers: `fw_buf` and `buf` (`[0u8; 1024]`) are allocated once in `run_until`, not re-zeroed per step.
- Timeout bookkeeping: `Stack::update` skips work inside its 100 ms interval.
- PLDM parsing: light in the part read; `pldm_interface` (outside the repo) was not read.
- MCTP inbound for middle fragments: ~31k cycles each.

## Suggested order of work
1. Table-driven CRC (item 1): minimal change, ~50 ms+ per chunk.
2. CPI calibration loop and MRAC / ICCM-DCCM check (item 2). If CPI is high this multiplies everything else.
3. Remove the 1032 B `req_copy` and reuse the request handle (item 4); rate-limit the 1 ms poll (item 5).
4. Add trace tags at the start of each callback (item 3) and per-MMIO tags on the send path (item 9); then `pop_frame` and buffer hoisting (item 6).
