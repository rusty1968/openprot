# MCTP Testing with tmux

This guide shows how to run the full MCTP test workflow in one tmux session.

## Goal

Run these components side-by-side in tmux panes:

1. QEMU running the firmware image
2. socat bridging QEMU socket to a PTY
3. mctp-dev on the bridged PTY
4. echo_linux host test client

## Prerequisites

From repo root, build firmware and host tool:

    bazel build --config=virt_ast10x0 //target/ast10x0/tests/mctp_echo:mctp_echo_image
    cargo build --release --manifest-path tools/mctp/echo_linux/Cargo.toml

Install required host packages and tool:

    sudo apt install tmux socat qemu-system-arm

If your packaged QEMU is missing ast1030-evb support, follow the source-build section in tools/mctp/README.md and use that qemu-system-arm binary in the commands below.

mctp-dev should be installed and available on PATH.

## Start a tmux session

Create a named session:

    tmux new -s mctp-test

Inside tmux, create a 2x2 layout:

    Ctrl-b %
    Ctrl-b "
    Ctrl-b o
    Ctrl-b "

Pane mapping used below:

1. Top-left: QEMU
2. Top-right: socat
3. Bottom-left: mctp-dev
4. Bottom-right: echo_linux

## Commands per pane

Set a firmware variable first in any pane (adjust path if needed):

    FW=bazel-bin/target/ast10x0/tests/mctp_echo/mctp_echo_image.elf

Pane 1 (QEMU):

    qemu-system-arm \
      -machine ast1030-evb -nographic \
      -kernel "$FW" \
      -serial mon:stdio \
      -chardev socket,id=mctp0,path=/tmp/mctp.sock,server=on,wait=off \
      -serial chardev:mctp0

Pane 2 (socat bridge):

    socat -d -d PTY,raw,echo=0,link=/tmp/mctp-pty UNIX-CONNECT:/tmp/mctp.sock

Pane 3 (mctp-dev):

    mctp-dev serial /tmp/mctp-pty

Pane 4 (echo_linux):

    REMOTE_EID=8 MSG_TYPE=1 cargo run --release --manifest-path tools/mctp/echo_linux/Cargo.toml

## Optional: one-shot tmux bootstrap

You can launch the full layout from outside tmux:

    FW="$(pwd)/bazel-bin/target/ast10x0/tests/mctp_echo/mctp_echo_image.elf"
    tmux new-session -d -s mctp-test
    tmux send-keys -t mctp-test:0.0 "qemu-system-arm -machine ast1030-evb -nographic -kernel $FW -serial mon:stdio -chardev socket,id=mctp0,path=/tmp/mctp.sock,server=on,wait=off -serial chardev:mctp0" C-m
    tmux split-window -h -t mctp-test:0.0
    tmux send-keys -t mctp-test:0.1 "socat -d -d PTY,raw,echo=0,link=/tmp/mctp-pty UNIX-CONNECT:/tmp/mctp.sock" C-m
    tmux split-window -v -t mctp-test:0.0
    tmux send-keys -t mctp-test:0.2 "mctp-dev serial /tmp/mctp-pty" C-m
    tmux split-window -v -t mctp-test:0.1
    tmux send-keys -t mctp-test:0.3 "REMOTE_EID=8 MSG_TYPE=1 cargo run --release --manifest-path tools/mctp/echo_linux/Cargo.toml" C-m
    tmux select-layout -t mctp-test:0 tiled
    tmux attach -t mctp-test

## tmux quality-of-life keys

1. Switch pane: Ctrl-b o
2. Select pane by arrows: Ctrl-b ArrowKey
3. Zoom pane: Ctrl-b z
4. Detach session: Ctrl-b d
5. Re-attach later: tmux attach -t mctp-test

## Teardown order

Stop in this order to avoid noisy socket errors:

1. echo_linux
2. mctp-dev
3. socat
4. QEMU

Then exit tmux session if desired:

    tmux kill-session -t mctp-test

## Troubleshooting

If mctp-dev sees no traffic:

1. Ensure QEMU is running and /tmp/mctp.sock exists.
2. Ensure socat pane shows a connected PTY at /tmp/mctp-pty.
3. Confirm mctp-dev is pointed to /tmp/mctp-pty.
4. Check firmware boot logs in QEMU pane for MCTP init.

If the echo client fails:

1. Verify REMOTE_EID and MSG_TYPE values.
2. Ensure mctp-dev is running before echo command.
3. Rebuild host tool in release mode if binary is stale.
