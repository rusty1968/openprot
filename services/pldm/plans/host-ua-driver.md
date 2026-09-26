# Plan: host Update-Agent (UA) driver for PLDM over MCTP over I3C

The host side of an on-target PLDM firmware-update test. The firmware runs the
PLDM Firmware Device (FD) — `FirmwareDevice::run_terminus` + a bounded-slice
`MockFdOps` — behind the `mctp_server`/`i3c_server` we already have. This driver
plays the **Update Agent**, driving a one-component update to completion over
the emulator's I3C socket and asserting the FD reaches `ReadyXfer`.

Reference for the exact messages/APIs: `services/pldm/tests/firmware_update_host.rs`
(same UA logic, but over in-memory transports). The wire framing reuses the
MCTP-over-I3C helpers from `target/veer/tests/mctp_i3c` and
[[i3c-emulator-test-patterns]].

## The fundamental difference from the in-memory host test

In `firmware_update_host.rs` the FD runs in the *same thread*; `ua_transact`
sends a command and synchronously pumps the transfer queues, and a
`handle_fd_request` closure answers FD-initiated requests during the pump.

Over I3C the FD runs **autonomously in firmware** (its own process, in the
emulator). The host UA is a separate process talking over the socket, so it must
**interleave** two flows itself:

- **UA→FD commands** (host is requester): host does a private **write** with the
  PLDM request, then a private **read** for the FD's response.
- **FD→UA requests** (host is responder): when the FD is in update mode it
  autonomously issues `RequestFirmwareData` / `TransferComplete` /
  `VerifyComplete` / `ApplyComplete` via its requester transport. The FD (an I3C
  target) *stages* these; the host does a private **read** to collect one, then
  a private **write** with the response.

So the host UA is a small state machine, not a linear script.

## Message sequence (one small component)

Phase A — UA→FD setup (write request, read response), in order:

1. `QueryDeviceIdentifiers` → descriptors.
2. `GetFirmwareParameters` → parameters.
3. `RequestUpdate{max_transfer_size, num_comp=1, ...}` → Success (FD Idle → LearnComponents).
4. `PassComponentTable{StartAndEnd, comp}` → Success (→ ReadyXfer).
5. `UpdateComponent{comp, comp_image_size}` → Success (→ Download; FD begins RequestFirmwareData).

Phase B — FD→UA transfer loop (read FD request, write response) until `ApplyComplete`:

6. `RequestFirmwareData{offset, length}` → respond with `image[offset..offset+length]`.
   Repeat until the whole image is requested.
7. `TransferComplete{result}` → ack.
8. `VerifyComplete{result}` → ack.
9. `ApplyComplete{result}` → ack.

Phase C — UA→FD finish:

10. `ActivateFirmware` → Success.
11. `GetStatus` → assert `current_state == ReadyXfer`, completion Success.

## Reuse

- **Framing:** the `mctp_i3c` MCTP-over-I3C frame builder (`[dest<<1, 0x0f, bc,
  src<<1|1, mctp_hdr, msg_type, pldm..., PEC]`), sent raw
  (`send_private_write_raw_on_stream`); MCTP message type for PLDM is `0x01`.
- **PLDM messages:** `pldm_common` codecs, exactly as the in-memory test —
  `RequestUpdateRequest::new(..).encode()`, `RequestFirmwareDataRequest::decode()`,
  `RequestFirmwareDataResponse::new(..).encode()`, `GetStatusResponse::decode()`,
  etc. `pldm_common` is `no_std` but host-buildable (the existing host tests use
  it), so no hand-rolled PLDM bytes.
- **MCTP header:** EIDs `UA_EID`/`FD_EID`; the FD serves `remote_eid == UA_EID`.
  SOM|EOM single-fragment per message (see the size constraint below).

## Host UA state machine

```text
setup:   for cmd in [QueryDevId, GetFwParams, RequestUpdate,
                      PassComponentTable, UpdateComponent]:
             write(frame(cmd)); resp = read(); assert ok(resp)
transfer: loop:
             req = read_fd_request()            # private read; may retry until staged
             match pldm_cmd(req):
               RequestFirmwareData(off,len) -> write(frame(fw_data_resp(image[off..off+len])))
               TransferComplete             -> write(frame(ack))
               VerifyComplete               -> write(frame(ack))
               ApplyComplete                -> write(frame(ack)); break
finish:  write(frame(ActivateFirmware)); assert ok(read())
         write(frame(GetStatus));        assert state==ReadyXfer
```

Instance IDs: the UA increments its own for UA→FD requests; FD-initiated requests
carry the FD's instance id, which the UA **echoes** in its response (decode
`PldmMsgHeader` for it, as `firmware_update_host.rs` does).

## Hard constraints & risks

- **Transfer size must keep every message single-fragment.** A
  `RequestFirmwareData` *response* (the chunk the host writes **into** the FD) is
  an MCTP *inbound* to the FD. If a chunk exceeds the ~241-byte MTU it fragments,
  and **multi-fragment inbound is not yet supported** (single-frame latch — see
  [[i3c-emulator-test-patterns]] and the multi-frame-inbound plan). So pick
  `max_transfer_size`/`IMAGE_SIZE` small enough that each chunk response fits one
  fragment (payload ≲ ~200 B after PLDM+MCTP headers). A 512 B image in ~200 B
  chunks = ~3 rounds. This is the tightest coupling to the queue work: a
  realistic transfer size needs multi-frame inbound first.
- **Timing / requester timeout.** The FD's `send_request` for
  `RequestFirmwareData` waits `requester_timeout_millis` for the host's chunk. If
  the host isn't reading/responding promptly the FD times out. The host must be
  in its transfer loop (reading) as soon as `UpdateComponent` is acked.
- **Two FD transports over one mctp channel.** The FD holds responder +
  requester `MctpClient`s. Whether both share one mctp channel/EID or need two is
  an FD-app (firmware) wiring question that also decides whether the host talks to
  one endpoint or two. **Assumption:** one endpoint (`FD_EID`), both flows over
  it, distinguished by PLDM direction/tag — confirm when the FD app is built.
- **Single-frame latch pacing.** As in `mctp_i3c`, pace writes; read the FD's
  staged request before writing its response.
- **Exit rendezvous.** After `GetStatus` asserts, the FD firmware must exit
  cleanly (`Runner::wait`) — reuse the completion-write rendezvous pattern from
  `i3c_backpressure` (a marker the FD app waits for), or have the FD exit 0 after
  a defined stop condition via `run_until`.

## Incremental build (each step is a separate emulator run on your box)

1. **Phase A only** — issue the 5 setup commands, assert responses. Proves the
   UA↔FD request/response ping-pong over I3C and the PLDM codec framing. No
   transfer yet (stop after `UpdateComponent`, or use a `run_until` that stops
   before Download).
2. **Phase B** — the FD-initiated transfer loop with a tiny single-fragment
   image. This is where timing and the two-transport routing get exercised.
3. **Phase C** — `ActivateFirmware` + `GetStatus`, and the clean-exit rendezvous.

## Verification

- Host build of the driver crate is host-only (no emulator): `pldm_common`
  compiles for host, so the UA logic type-checks locally.
- Full run is `//target/veer/tests/pldm_i3c:pldm_i3c_test` (emulator, exclusive)
  — your box, per [[veer-emulator-oom]].
