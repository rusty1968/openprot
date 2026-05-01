Testing an MCTP Firmware ELF on QEMU ast1030-evb
This guide assumes:

You have a firmware ELF for the Aspeed AST1030 (Cortex-M4F MiniBMC), typically a Zephyr build, that implements an MCTP endpoint over one of the UARTs (DSP0253 framing, e.g. via mctp-estack or equivalent).
You want to talk to that endpoint from the host using mctp-dev from https://github.com/CodeConstruct/mctp-dev — no real hardware, no Linux guest.


Prerequisites

qemu-system-arm 8.0 or newer (must include the ast1030-evb machine).
socat on the host.
mctp-dev built and on PATH:

bash  git clone https://github.com/CodeConstruct/mctp-dev
  cd mctp-dev && cargo build --release

Your firmware ELF, and knowledge of which UART it uses for MCTP. The boot/log console on the AST1030 model is UART5 by default (changeable with bmc-console=). Pick a different UART for MCTP — UART1 is a common choice.


1. Boot the firmware
The AST1030 model exposes serial slots in order (-serial #1 → boot console UART5, then additional -serial flags map to the remaining UARTs in firmware-defined order). The simplest pattern: console on stdio, MCTP UART on a Unix socket.
bashqemu-system-arm \
  -machine ast1030-evb -nographic \
  -kernel /path/to/firmware.elf \
  -serial mon:stdio \
  -chardev socket,id=mctp0,path=/tmp/mctp.sock,server=on,wait=off \
  -serial chardev:mctp0
What's happening:

-machine ast1030-evb — Aspeed MiniBMC (Cortex-M4F).
-kernel firmware.elf — Zephyr-style ELF load.
First -serial → boot console (UART5), printed to your terminal.
Second -serial → second UART, exposed as a Unix socket at /tmp/mctp.sock. This must be the UART your firmware uses for MCTP. If your firmware uses a different UART number, add additional -serial null slots before the chardev one to shift it into the right position, or pass bmc-console=uartN to relocate the boot console.

Watch the console output to confirm the firmware boots and reports its MCTP UART is up.

2. Bridge the socket to a PTY
mctp-dev wants a tty-style device, so wrap the socket:
bashsocat -d -d PTY,raw,echo=0,link=/tmp/mctp-pty UNIX-CONNECT:/tmp/mctp.sock
You now have /tmp/mctp-pty pointing at the firmware's MCTP UART. Leave socat running.

3. Run mctp-dev against the firmware
In a third terminal:
bashmctp-dev serial /tmp/mctp-pty
mctp-dev will start the MCTP control protocol responder/initiator on that link and log traffic. From here:

It will respond to control-protocol queries from the firmware (if your firmware acts as bus owner), or
You can use it to assign an EID to the firmware and exercise upper-layer protocols (NVMe-MI, PLDM) if mctp-dev was built with those features (--features nvme-mi, etc.).

Watch both windows: the QEMU console for firmware-side logs, and mctp-dev for host-side traffic.

Teardown
Stop in this order: mctp-dev (Ctrl-C), socat (Ctrl-C), then QEMU (Ctrl-A X from the monitor). The /tmp/mctp.sock file is cleaned up by QEMU on exit; /tmp/mctp-pty by socat.

Troubleshooting

mctp-dev sees nothing. Almost always a UART-slot mismatch. Add -serial null placeholders or use bmc-console=uartN so the chardev lands on the UART your firmware actually uses for MCTP.
Firmware boots but never opens the MCTP UART. Check the firmware's own log on the boot console — it usually prints which UART it bound MCTP to.
Garbled bytes. Confirm the firmware is using DSP0253 framing (start/escape 0x7e/0x7d, FCS) and that mctp-dev is in serial mode, not usb.
Connection refused on /tmp/mctp.sock. QEMU exited or never started the chardev. Recheck the -chardev line; server=on,wait=off is required so QEMU listens without blocking boot.
Need to re-run mctp-dev repeatedly. Leave QEMU and socat up; only restart mctp-dev. The socket persists for the life of the QEMU process.
