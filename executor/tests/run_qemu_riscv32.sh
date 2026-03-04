#!/usr/bin/env bash
# Licensed under the Apache-2.0 license
# SPDX-License-Identifier: Apache-2.0

# Run the executor integration test on QEMU RISC-V 32 virt machine.
#
# Usage:
#   ./executor/tests/run_qemu_riscv32.sh <path-to-elf-binary>
#
# Prerequisites:
#   - qemu-system-riscv32 installed (apt: qemu-system-misc, brew: qemu)
#
# QEMU virt machine flags:
#   -machine virt          RISC-V virt platform
#   -cpu rv32              RV32 CPU
#   -m 16M                 16 MiB RAM (matches linker script)
#   -nographic             No graphical output
#   -semihosting-config    Enable semihosting for hprintln! and exit
#   -bios none             No firmware, direct ELF load
#   -kernel <elf>          The test binary

set -euo pipefail

BINARY="${1:?Usage: $0 <path-to-elf-binary>}"

if ! command -v qemu-system-riscv32 &>/dev/null; then
    echo "ERROR: qemu-system-riscv32 not found. Install with:"
    echo "  sudo apt install qemu-system-misc"
    exit 1
fi

echo "=== Running executor test on QEMU riscv32 virt ==="
echo "Binary: ${BINARY}"
echo ""

qemu-system-riscv32 \
    -machine virt \
    -cpu rv32 \
    -m 16M \
    -nographic \
    -semihosting-config enable=on,target=native \
    -bios none \
    -kernel "${BINARY}"

EXIT_CODE=$?

if [ ${EXIT_CODE} -eq 0 ]; then
    echo ""
    echo "=== QEMU exited successfully (code 0) ==="
else
    echo ""
    echo "=== QEMU exited with code ${EXIT_CODE} ==="
fi

exit ${EXIT_CODE}
