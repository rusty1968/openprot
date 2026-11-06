# PLDM firmware device: polling vs event-driven (before / after)

Status 2026-10-23: implemented and tested on the host and emulator (12 of 12 emulator tests,
all `services/mctp` and `services/pldm` host suites). **Not yet run on the FPGA**; no speed number
exists for the event-driven version. OpenPRoT working tree, uncommitted.

Scope of "event driven": this change makes the FD's wait for a *response or a UA command* event
driven. The rest was already event driven (the MCTP server loop sleeps on a WaitGroup and wakes on
the I3C IRQ, a client request or a receive deadline) and is unchanged. The FD still makes two IPC
round trips per chunk (send, then wait).

## 1. Before: poll the responder after every request

```
 FD thread (pldm_fd)                       MCTP server (colo process)             UA (host)
 -------------------                       --------------------------             ---------
 step N
  Phase 1  generate RequestFirmwareData
           send     ------ IPC #1 ------>  encode, fragment, I3C send ----------> (sees IBI)
           recv     ------ IPC #2 ------>  defer reply (no deadline)  <---------- response fragments
                    <----- reply ---------  reassembled, complete
           process response
  Phase 2  poll responder, timeout 1 ms
           recv(listener, 1ms) -- IPC #3 ->  register deferred recv, deadline now+1ms
                                             ... sleep until the deadline ...
                                             wake on deadline, encode TimedOut
                    <----- TimedOut ------
           (timeout = "nothing there": ignored)
 step N+1 ...
```

- 3 IPC round trips per chunk, one of them a poll that almost always finds nothing.
- In the 64 KiB benchmark the UA sent no command during the download (host log: only response
  fragments were written), so every one of those polls was wasted. Measured cost of the poll: about
  29 ms per chunk (72.0 s -> 62.2 s when it ran 1 in 16 instead of every step; derived average).
- Why a poll at all: one thread, two jobs (ask the UA for chunks; answer UA commands such as
  CancelUpdate). The client API had one blocking receive per handle and no "wait for either", and
  `timeout 0` means "wait forever", so there was no real non-blocking check; 1 ms was the closest.

## 2. Interim: poll once every 16 requests (committed, `41280405`)

```
 step N      Phase 1 as above (IPC #1 send, IPC #2 recv)       Phase 2 skipped
 step N+1    Phase 1                                           Phase 2 skipped
 ...
 step N+15   Phase 1                                           Phase 2 skipped
 step N+16   Phase 1                                           Phase 2: poll (IPC #3)
```

- About 2.06 IPC round trips per chunk. Download 62.2 s.
- A UA command that arrives between polls waits up to 16 requests (about 3 s at the current speed)
  before it is answered. This is the trade-off the event-driven version removes.
- The timeout of the poll stayed 1 ms; only the frequency changed.

## 3. After: wait once for the response or a command

```
 FD thread (pldm_fd)                       MCTP server (colo process)             UA (host)
 -------------------                       --------------------------             ---------
 step N
  generate RequestFirmwareData
  send_request_serving:
    send        ------ IPC #1 ------>     encode, fragment, I3C send -----------> (sees IBI)
    recv_any    ------ IPC #2 ------>     register ONE deferred reply for
    [req handle, listener handle]         BOTH handles, shared deadline
                                          ... sleeps; wakes on whichever event comes first ...
      case A: response arrives     <------------------------------------------- response fragments
                    <----- Response -----  reply carries the req handle
      process response; next step                                              (no poll)

      case B: a UA command arrives while waiting
                                          <------------------------------------ CancelUpdate
                    <----- Command ------  reply carries the listener handle
      handler answers the command  ---- IPC (send response) ---->  --------->
      recv_any again (the request is still outstanding)  ------ IPC ------>  ...
      then case A
```

- 2 IPC round trips per chunk (send, recv_any), no polling, and a command is answered the moment
  it arrives, even in the middle of a request.
- Case A is the common path in the benchmark.

## 4. How the wait works

```
 FD                      client-ipc                    server (mctp_server_runtime::run)
 --                      ----------                    ---------------------------------
 recv_or_command(listener)
   recv_any([req, lst]) -> RecvAny request  ------->   try_recv_any: message already waiting?
                          (timeout, handles)             yes -> reply at once (earlier handle first)
                                                         no  -> register_recv_any:
                                                                  outstanding[req] = {deadline, group k, prio 0}
                                                                  outstanding[lst] = {deadline, group k, prio 1}
                                                                  defer reply token under key k = req
   (thread blocked)                                    events: I3C IRQ -> inbound(pkt) -> update():
                                                                  group k: first member (prio order)
                                                                  with a message wins; the other
                                                                  stays queued; whole group removed;
                                                                  one reply: header.handle = winner
                                                         deadline -> group k times out together
   <- reply (handle = winner)
   winner == req  -> ReqEvent::Response
   winner == lst  -> ReqEvent::Command(.., resp channel)
```

Rules: all-or-nothing registration (BadArgument for empty, repeated or more than 4 handles;
AddrInUse if a handle already has a receive; NoSpace if the table lacks room); a message beats a
simultaneous timeout; both messages waiting -> the earlier handle (the response) is returned and
the command stays queued for the next wait; `cancel_recv(key)` or `unbind(any member)` drops the
whole group.

## 5. FD step, before and after

```
 BEFORE                                          AFTER
 ------                                          -----
 step:                                           step:
   if initiator active and a request is ready:     if initiator active and a request is ready:
       send_request  (send + wait response)            send_request_serving:
       process response                                    send, then wait for (response | command)
   poll responder (1 ms, or blocking when idle)            serve each command that arrives
   -> serve a command if one arrived                       until the response arrives
                                                       process response; end of step
                                                     else (idle, or initiator with nothing to send):
                                                       wait for a command (blocking when idle,
                                                       a 1 ms look otherwise), as before
```

## 6. What did not change

- The server loop, the I3C driver and IRQ path, MCTP reassembly, the request-handle reuse and the
  CRC table.
- Per-IPC cost: the cost of one IPC round trip on the board is still unmeasured; this change does
  not reduce it. Merging send and receive into one call (one IPC per chunk) is a separate, not yet
  scoped-in step (see `event-driven-fd-scope.md`, option C).

## 7. Limitations and open points

- The request's client issues the `RecvAny` with a listener handle allocated by the responder's
  client. They must talk to the same server (true in all FD apps here: one `handle::MCTP` channel).
  One transport for both roles would be cleaner but changes `FirmwareDevice::init`.
- A `CancelUpdate` that arrives mid-request is answered, then the request's response is still
  awaited and processed as usual; whether a real UA expects the FD to abandon the in-flight request
  is a protocol question not covered by a test.
- Expected speed: about the poll-every-16 result (62.2 s); the poll was already 15/16 gone, so this
  buys correctness (immediate cancel), not throughput. To be confirmed on the board.
