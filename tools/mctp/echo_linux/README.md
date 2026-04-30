# MCTP Echo Host Tool (Linux)

Small host-side test tool for sending an MCTP "Hello, World!" payload and verifying echo response.

This is a **host-only tool**, separate from the Bazel firmware build.

## Prerequisites

- Linux with AF_MCTP support in kernel
- `mctp` and `mctp-linux` crates available from crates.io
- MCTP route/interface configuration for your MCTP link
- Target firmware reachable at the specified EID

## Build & Run

From the repo root:

```bash
cargo run --manifest-path tools/mctp/echo_linux/Cargo.toml
```

Optional environment variables:

- `REMOTE_EID` (default: `8`)
- `MSG_TYPE` (default: `1`)

Example:

```bash
REMOTE_EID=8 MSG_TYPE=1 cargo run --manifest-path tools/mctp/echo_linux/Cargo.toml
```

## Usage in QEMU Workflow

After setting up QEMU + socat + mctp-dev (as described in [test-mctp.md](../../test-mctp.md)):

1. Ensure kernel MCTP support is enabled.
2. Configure MCTP route for the device (socat PTY).
3. Run this tool to send a test message.
