# Serial MCTP Echo Implementation Plan

## 1. Scope and success criteria

1. Build a serial-transport MCTP path in the AST10x0 runtime while keeping the echo app transport-agnostic.
2. Reuse the existing stack facade pattern so echo logic remains listener/response only.
3. Validate with a bootable system image test that exercises serial RX and echo response end-to-end.

## 2. Baseline alignment (read-only checkpoint)

1. Confirm current MCTP server runtime in `target/ast10x0/mctp/server/main.rs`.
2. Confirm serial transport trait surface in `services/mctp/transport-serial/src/common.rs`.
3. Use Hubris serial pattern as reference from `../hubris/task/mctp-server/src/serial.rs` and `../hubris/task/mctp-server/src/main.rs`.
4. Keep echo behavior equivalent to `../hubris/task/mctp-echo/src/main.rs`.
5. Confirm backend selection in `target/ast10x0/serial/BUILD.bazel`: alias `ast10x0_serial` defaults to `ast10x0_serial_direct` (not IPC).

## 3. Transport implementation tasks

1. Add a concrete `SerialSender` in userspace_runtime MCTP server package.
2. Implement `Sender::send_vectored` using a fragmentation loop and serial framing handler, mirroring Hubris flow.
3. Add a serial inbound handling path in the server loop:
   1. receive bytes from direct serial backend
   2. decode/frame into MCTP packet
   3. call `server.inbound(...)`
   4. call `server.update(...)`
4. Keep failure policy consistent with shared fail-stop helper in `target/ast10x0/userspace_runtime.rs`.

## 4. App wiring tasks

1. Add or update server target wiring in `target/ast10x0/mctp/server/BUILD.bazel` for serial transport dependencies.
2. Ensure the dependency uses `//target/ast10x0/serial:ast10x0_serial` (default direct backend), not `:ast10x0_serial_ipc`.
3. Update process wiring in `target/ast10x0/mctp/server/system.json5`:
   1. `mctp_server` gets serial driver/channel access
   2. `mctp_echo` connects only to `mctp_server` IPC endpoint
4. Keep echo app itself facade-based (no transport-specific logic).

## 5. Echo app tasks

1. Finalize echo app source in the target MCTP server package if not already present.
2. Keep logic minimal:
   1. set EID
   2. listener on configured msg type
   3. receive and send payload back
3. Ensure no direct serial operations in the echo app.

## 6. Test plan

1. Unit-level compile checks:
   1. `bazel build --config=virt_ast10x0 //target/ast10x0/mctp/server:app_mctp_server`
   2. `bazel build --config=virt_ast10x0 //target/ast10x0/tests/mctp_echo:mctp_echo_image`
2. Integration or boot test:
   1. create a serial-focused system image test under `target/ast10x0/tests/mctp`
   2. verify server startup, listener registration, serial packet ingress, and echo response
3. Regression check:
   1. existing MCTP boot test remains green
   2. no breakage for current non-serial test targets

## 7. Risks and mitigations

1. Risk: unknown serial framing API details in the pinned `mctp-lib` branch.
   Mitigation: start with Hubris-proven handler calls and adjust signatures to current crate API.
2. Risk: notification or wait-group mismatch for serial RX in userspace runtime.
   Mitigation: add explicit event mapping test and startup assertions.
3. Risk: payload sizing and MTU fragmentation edge cases.
   Mitigation: add oversized and multi-fragment echo test vectors.

## 8. Execution order

1. Implement `SerialSender` and inbound serial handling.
2. Wire BUILD and system configuration.
3. Finalize echo app source and dependency graph.
4. Add serial integration test target.
5. Run validation and adjust logs and error paths.
