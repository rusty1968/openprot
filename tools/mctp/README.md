# MCTP Host Tools

Host-side utilities for testing and validating MCTP firmware endpoints over QEMU and real hardware.

## Overview

This directory contains **standalone host tools** that are separate from the main Bazel firmware build. They are intended for QEMU bring-up, hardware validation, and integration testing using serial MCTP framing (DSP0253).

---

## Quick Start: Echo Test Over QEMU Serial

Two paths are supported:

### Path 1: echo_linux (AF_MCTP socket, Linux kernel-based)
Use echo_linux if your kernel supports AF_MCTP and you just need to send/receive MCTP messages directly.

### Path 2: mctp-dev (full control protocol)
Use mctp-dev if you want a full MCTP control-protocol responder/initiator on the host side.

---

## Prerequisites (Both Paths)

### Bazelisk (required for firmware builds)

Do **not** install the `bazel` apt package (`sudo apt install bazel-bootstrap`) — it is an old version that does not support Bzlmod (`MODULE.bazel`) and will fail with `WORKSPACE file not found`.

This repo uses `bazelisk`, which reads `.bazelversion` and automatically downloads the correct Bazel version (8.5.1).

```bash
# Download bazelisk and install it as 'bazel' on PATH
curl -fsSL https://github.com/bazelbuild/bazelisk/releases/latest/download/bazelisk-linux-amd64 \
  -o /usr/local/bin/bazel && chmod +x /usr/local/bin/bazel
```

If you already installed `bazel-bootstrap` via apt, remove it first:

```bash
sudo apt remove bazel-bootstrap
```

Verify bazelisk is active:

```bash
bazel version
# Should print: Bazel version 8.5.1 (or similar, downloaded by bazelisk)
# Should NOT print: Build label: 5.x.x or earlier
```

### QEMU
- `qemu-system-arm` 8.0 or newer (must include `ast1030-evb` machine)

```bash
sudo apt install qemu-system-arm
qemu-system-arm --version  # must be 8.0 or newer
```

### Host Tools
- `socat` (for socket-to-PTY bridging): `sudo apt install socat`
- `mctp-dev` (if using Path 2): `git clone https://github.com/CodeConstruct/mctp-dev && cd mctp-dev && cargo build --release`
- `echo_linux` tool (this repo): built with `cargo run --manifest-path tools/mctp/echo_linux/Cargo.toml`

---

## Building the Firmware

All Bazel commands **must be run from the repo root** — the directory containing `MODULE.bazel`. Running from a subdirectory will fail with `not within a workspace`.

```bash
cd /path/to/userspace_runtime   # repo root
```

### Build the bootable MCTP firmware image

```bash
bazel build --config=virt_ast10x0 //target/ast10x0/tests/mctp_echo:mctp_echo_image
```

If you only want a compile check for the standalone userspace server app, build the generated app target instead:

```bash
bazel build --config=virt_ast10x0 //target/ast10x0/mctp/server:app_mctp_server
```

### Run the built-in MCTP echo test under QEMU

This is the fastest sanity check — no host tooling required:

```bash
bazel test --config=virt_ast10x0 //target/ast10x0/tests/mctp_echo:mctp_echo_test \
  --test_output=streamed --test_timeout=10
```

### Build all AST10x0 targets

```bash
bazel build //target/ast10x0/...
```

### Run all AST10x0 tests under QEMU

```bash
bazel test --config=virt_ast10x0 //target/ast10x0/...
```

> The `--config=virt_ast10x0` flag selects the Pigweed QEMU runner using the `ast1030-evb` machine with semihosting. Without it, firmware-backed tests are skipped.

The firmware ELF produced by the `mctp_echo_image` target is used as the `-kernel` argument in the QEMU commands below.

---

## Path 1: Firmware Echo via AF_MCTP (Linux Kernel-Based)

For simpler testing without a full protocol stack, use the `echo_linux` tool. This requires kernel MCTP support.

### Prerequisites: Linux Kernel MCTP Support

Check if your kernel has MCTP enabled:

```bash
grep CONFIG_MCTP /boot/config-$(uname -r)
```

Expected output (at least):
```
CONFIG_MCTP=y
CONFIG_MCTP_SERIAL=m
```

If `CONFIG_MCTP` is not present, rebuild your kernel with `CONFIG_MCTP=y`. If `CONFIG_MCTP_SERIAL` is not present, add it as well (for serial transport over UART).

Load the modules if built as modules:

```bash
sudo modprobe mctp
sudo modprobe mctp_serial  # Only if CONFIG_MCTP_SERIAL=m
```

Verify:

```bash
lsmod | grep mctp
ip mctp help  # Should show MCTP subcommands
```

### Using echo_linux

The `echo_linux` tool sends a "Hello, World!" MCTP message and verifies the echo response.

**Build & Run** (from repo root):

```bash
cargo run --manifest-path tools/mctp/echo_linux/Cargo.toml
```

**Environment Variables:**
- `REMOTE_EID` (default: `8`) — EID of the target firmware
- `MSG_TYPE` (default: `1`) — MCTP message type (0x7e for firmware echo app)

**Example:**

```bash
REMOTE_EID=8 MSG_TYPE=1 cargo run --manifest-path tools/mctp/echo_linux/Cargo.toml
```

**Requirements:**
- Kernel MCTP support enabled (as checked above)
- MCTP interface/route configured pointing to the target (usually via mctp-dev, kernel tools, or hardware setup)
- Target firmware running echo responder at the specified EID

---

## Path 2: Firmware Echo over mctp-dev + Serial Socket

This path uses `mctp-dev` as the host-side MCTP control protocol stack, communicating with firmware over a serial socket.

### Step 1: Boot Firmware in QEMU

Express: firmware ELF on UART1 or another UART (not the boot console UART5):

```bash
qemu-system-arm \
  -machine ast1030-evb -nographic \
  -kernel /path/to/firmware.elf \
  -serial mon:stdio \
  -chardev socket,id=mctp0,path=/tmp/mctp.sock,server=on,wait=off \
  -serial chardev:mctp0
```

**Explanation:**
- `-machine ast1030-evb` — Aspeed MiniBMC (Cortex-M4F)
- `-kernel firmware.elf` — Load firmware ELF
- First `-serial mon:stdio` → Boot console (UART5), prints to your terminal
- Second `-serial chardev:mctp0` → MCTP UART (second slot), exposed via Unix socket at `/tmp/mctp.sock`

**UART Slot Mapping:**
The AST1030 model maps `-serial` flags in order:
- 1st `-serial` → UART5 (boot console, default)
- 2nd `-serial` → second UART in firmware order (typically UART1 or UART0)

If your firmware uses a different UART order, add `-serial null` placeholders to shift the mapping, or use `-bmc-console=uartN` to relocate the boot console.

**Verify Firmware Boots:**
Watch the QEMU console output. The firmware should log that MCTP is up on its designated UART.

### Step 2: Bridge Socket to PTY

In a **second terminal**, wrap the Unix socket as a PTY:

```bash
socat -d -d PTY,raw,echo=0,link=/tmp/mctp-pty UNIX-CONNECT:/tmp/mctp.sock
```

This creates `/tmp/mctp-pty` pointing to the firmware's MCTP UART. Leave socat running.

### Step 3: Start mctp-dev

In a **third terminal**, launch mctp-dev:

```bash
mctp-dev serial /tmp/mctp-pty
```

mctp-dev will:
- Start the MCTP control protocol responder/initiator
- Log traffic to stdout
- Respond to control-protocol queries from the firmware (if firmware is bus owner)
- Allow you to assign EIDs and test upper-layer protocols (PLDM, NVMe-MI if built with those features)

Watch this window for MCTP traffic and status messages.

### Step 4: Verify Integration

In a **fourth terminal**, you can:
- Use additional mctp-dev client commands (if available)
- Browse MCTP endpoint info
- Send custom MCTP control messages

---

## Complete QEMU + echo_linux + mctp-dev Workflow

If you want to test both echo_linux AF_MCTP socket AND mctp-dev protocol handling:

### Terminal 1: QEMU
```bash
qemu-system-arm \
  -machine ast1030-evb -nographic \
  -kernel firmware.elf \
  -serial mon:stdio \
  -chardev socket,id=mctp0,path=/tmp/mctp.sock,server=on,wait=off \
  -serial chardev:mctp0
```
Watch this for firmware boot messages.

### Terminal 2: socat (bridge socket to PTY)
```bash
socat -d -d PTY,raw,echo=0,link=/tmp/mctp-pty UNIX-CONNECT:/tmp/mctp.sock
```

### Terminal 3: echo_linux test
```bash
REMOTE_EID=8 MSG_TYPE=1 cargo run --manifest-path tools/mctp/echo_linux/Cargo.toml
```

### Terminal 4: mctp-dev (optional, for protocol inspection)
```bash
mctp-dev serial /tmp/mctp-pty
```
Watches protocol traffic, assigns EIDs if needed.

---

## Troubleshooting

### mctp-dev sees nothing
**Cause:** UART slot mismatch — the socket is not connected to the right UART.

**Fix:**
- Add `-serial null` placeholders before `-serial chardev:mctp0` to shift its position
- Or use `-bmc-console=uartN` to relocate the boot console
- Check firmware logs to confirm which UART it actually uses for MCTP

### Firmware boots but never opens MCTP UART
**Cause:** Firmware configuration or missing transports.

**Fix:**
- Check firmware boot console logs for error messages
- Verify firmware was compiled with MCTP serial transport enabled
- Check system.json5 or config for transport wiring

### Garbled bytes or framing errors
**Cause:** Firmware using different DSP0253 framing or wrong baud rate.

**Fix:**
- Confirm firmware uses DSP0253 framing (start/escape 0x7e/0x7d, FCS)
- Verify socat is in raw mode: `PTY,raw,echo=0`
- Check mctp-dev is in `serial` mode (not USB)
- Check baud rate if applicable

### Connection refused on /tmp/mctp.sock
**Cause:** QEMU crashed or socket options wrong.

**Fix:**
- Verify QEMU is still running: `ps aux | grep qemu`
- Recheck QEMU `-chardev` line: must have `server=on,wait=off`
- Delete stale socket: `rm /tmp/mctp.sock`
- Restart QEMU

### echo_linux cannot find MCTP socket
**Cause:** Kernel MCTP support not enabled or interface not configured.

**Fix:**
- Run kernel check (above): `grep CONFIG_MCTP`
- Ensure mctp-dev or kernel tools have created the MCTP interface
- For kernel-only setup: `ip mctp link set dev <device> up`

---

## Teardown

Stop in this order to avoid socket errors:

1. mctp-dev (Ctrl-C in Terminal 3)
2. socat (Ctrl-C in Terminal 2)
3. QEMU (Ctrl-A X in Terminal 1, or Ctrl-C)

QEMU cleans up `/tmp/mctp.sock` on exit. socat cleans up `/tmp/mctp-pty`.

---

## Reference

- **Firmware MCTP server runtime:** [target/ast10x0/mctp/server](../../target/ast10x0/mctp/server)
- **Firmware test clients (internal echo):** [target/ast10x0/tests/mctp_echo](../../target/ast10x0/tests/mctp_echo)
- **MCTP service libraries:** [services/mctp](../../services/mctp)
- **mctp-dev repository:** https://github.com/CodeConstruct/mctp-dev
- **MCTP specification:** https://www.dmtf.org/standards/pmci
