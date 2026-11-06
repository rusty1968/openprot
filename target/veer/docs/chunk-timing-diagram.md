# One download chunk: where the time goes (OpenPRoT vs native)

Sources: `host-ts-openprot.log` / `host-ts-native.log` (host timeline, clean device images,
medians over 331 chunks) and `ipc-run.log` (device-internal trace, instrumented image,
medians over the first 18 chunks). The two clocks are not correlated; the device-internal
numbers come from the instrumented image, whose chunk is 263 ms against 204 ms (host
period) / 215 ms (benchmark) for the clean image, so treat them as proportions.

## Host-visible timeline (clean images, host clock)

```
                         native        OpenPRoT
 IBI received            |             |
   |  UA turnaround      |  51.9 ms    |  51.0 ms   (same host code: identical)
 first host write        |             |
   |  4 fragment writes  |  16.6 ms    |  16.4 ms   (identical)
 last host write end     |             |
   |  DEVICE WORK        |  36.6 ms    | 131.6 ms   <--- the whole gap (+95 ms)
 next IBI received       |             |
 chunk period            | 107.4 ms    | 204.2 ms
```

## Device-internal flow of one chunk (OpenPRoT, instrumented trace)

```
 host / UA          I3C bus              colo process                     pldm_fd process
 (rot-beast)                             (I3C driver + MCTP server)       (FD state machine)
 ----------         -------              ---------------------------      -------------------
                                                                           FD next request   ---+
                                                                           (callback; builds     |
                                                         <--- IPC ---      RequestFirmwareData)  | 26.4 ms  [A] FD + MCTP server
                                          MCTP server encodes, I3C                               |          + IPC + sender
                                          sender frames + PEC (table)  ------------------------+
                                          driver: TX write + raise IBI      1.8 ms  [B] driver send
                      IBI  <------------- (request staged)
   IBI seen
   read request  ----> (IBI-triggered read, len 17)
                                          ibi-done irq (44.6 ms after send end:      }
                                          device idle, waiting for the host)         } 98.2 ms [C] host takes the
                                          ibi-done callback (2.5 + 1.5 ms work)      }   IBI, UA builds the response,
   UA turnaround ~51 ms (host)                                                       }   host starts writing
   write fragment 1 (68 B)  ------------> irq, drain, decode (3.3), MCTP inbound
   write fragment 2 (68 B)  ------------> irq, drain, decode, inbound       } 73.3 ms [D] 4 fragments received:
   write fragment 3 (68 B)  ------------> irq, drain, decode, inbound       }   host write pacing (~5 ms sleep
   write fragment 4 (13 B)  ------------> irq, drain, decode,               }   per write) overlaps device work
                                          MCTP inbound completes message:   }
                                          reassembly + delivery              26.9 ms  [E] last interrupt callback
                                          callback exit
                                          MCTP server wakes FD  --- IPC --->                             29.9 ms  [F] last fragment ->
                                                                           FD recv returns chunk data         FD has the data (server,
                                                                           download_fw_data (trivial)         IPC, reassembled copy)
                                                                           FD between callbacks:           33.2 ms  [G] FD between callbacks
                                                                           responder poll (1 ms timeout,      (1 ms responder poll, request
                                                                           1 IPC round trip), process         handle reuse, next request)
                                                                           response, generate next request
                                                                           FD next request   --> (next chunk)
```

## Allocation of the 263 ms (instrumented) and the 131.6 ms device window (clean)

| id | segment | ms (instrumented) | share | notes |
|---|---|---|---|---|
| A | FD next request -> request staged (FD, MCTP server, IPC, sender) | 26.4 | 10% | was 66.3 before the IPC changes |
| B | I3C driver send (TX write + IBI) | 1.8 | 1% | was 14.9 (it included FD-thread work) |
| C | send end -> first response fragment irq (IBI, host UA, host write start) | 98.2 | 37% | host-bound; identical on native |
| D | 4 response fragments received | 73.3 | 28% | of which the last interrupt callback is E |
| E | last interrupt callback (usually the last TWO fragments: decode + MCTP inbound for each, plus one ack) | 26.9 | 10% | inside D. Corrected 2026-10-25: the EOM fragment is not more expensive (inbound about 30k cycles for every fragment, 13 B or 68 B); this segment is long because the callback handles two fragments |
| F | last callback exit -> FD has the data | 29.9 | 11% | was 72.1 before the IPC changes |
| G | FD between callbacks (poll, response handling, next request) | 33.2 | 13% | was 56.6 |
| | total | 263.0 | | clean benchmark: 215 ms per chunk |

Device work after the last host write (clean: 131.6 ms vs native 36.6 ms) is E + F + G + A + B
= 26.9 + 29.9 + 33.2 + 26.4 + 1.8 = 118.2 ms in the instrumented run, plus bus and IRQ latency
and the IBI. C and the first three fragments of D are host-bound (same on native).

## What the next change touches

The responder poll (1 ms timeout, one IPC round trip per chunk) sits in G. Polling every 16
chunks removes one IPC call, one server wake on the deadline and the timeout path from 15 of
every 16 chunks. G is 33.2 ms in total, so the saving is a part of it, not all of it.
