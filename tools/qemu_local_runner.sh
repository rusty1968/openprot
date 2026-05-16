#!/usr/bin/env bash
# Run a kernel ELF under the locally-built qemu-system-arm with the AST1030
# EVB machine and semihosting. Used as Bazel --run_under for ad-hoc execution
# of system_image_test targets against an out-of-tree QEMU build.
#
# Usage: qemu_local_runner.sh <test-elf>
set -euo pipefail

QEMU="${QEMU_BINARY:-/home/ferro/work/qemu-ast10x0-i2c/build/qemu-system-arm}"
ELF="$1"

exec "$QEMU" \
    -machine ast1030-evb \
    -cpu cortex-m4 \
    -bios none \
    -nographic \
    -semihosting-config enable=on,target=native \
    -kernel "$ELF"
