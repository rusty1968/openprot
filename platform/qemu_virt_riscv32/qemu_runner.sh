#!/usr/bin/env bash
# Licensed under the Apache-2.0 license
# SPDX-License-Identifier: Apache-2.0

# QEMU runner for RISC-V 32 embedded test binaries.
#
# Used by Bazel sh_test as the test runner. Expects the ELF binary
# path as the first argument.
#
# Exit codes:
#   0 = semihosting exit(SUCCESS)
#   1 = semihosting exit(FAILURE) or QEMU error
#
# Prerequisites: qemu-system-riscv32 on PATH
#   Install: sudo apt install qemu-system-misc

set -euo pipefail

BINARY="${1:?Usage: $0 <path-to-elf-binary>}"

# Resolve to absolute path (handles Bazel runfiles)
if [[ ! -f "${BINARY}" ]]; then
    # Try relative to RUNFILES_DIR
    if [[ -n "${RUNFILES_DIR:-}" && -f "${RUNFILES_DIR}/${BINARY}" ]]; then
        BINARY="${RUNFILES_DIR}/${BINARY}"
    elif [[ -n "${TEST_SRCDIR:-}" && -f "${TEST_SRCDIR}/${BINARY}" ]]; then
        BINARY="${TEST_SRCDIR}/${BINARY}"
    else
        echo "ERROR: Binary not found: ${BINARY}" >&2
        exit 1
    fi
fi

QEMU_BIN="qemu-system-riscv32"
if ! command -v "${QEMU_BIN}" &>/dev/null; then
    echo "ERROR: ${QEMU_BIN} not found. Install with: sudo apt install qemu-system-misc" >&2
    exit 1
fi

# Run the test binary on QEMU virt machine with semihosting.
# Timeout after 30 seconds to catch hangs.
#
# -m 128M: Must exceed the linker script's RAM LENGTH (16M) so QEMU has
# room to place the DTB above the firmware's memory region.
timeout 30 "${QEMU_BIN}" \
    -machine virt \
    -cpu rv32 \
    -m 128M \
    -nographic \
    -semihosting-config enable=on,target=native \
    -bios none \
    -kernel "${BINARY}" \
    2>&1

exit $?
