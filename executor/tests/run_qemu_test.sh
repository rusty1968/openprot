#!/usr/bin/env bash
# Licensed under the Apache-2.0 license
# SPDX-License-Identifier: Apache-2.0

# Build and run the executor QEMU RISC-V 32 integration test.
#
# Usage:
#   ./executor/tests/run_qemu_test.sh          # build + run
#   ./executor/tests/run_qemu_test.sh --run     # run only (skip build)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
cd "${REPO_ROOT}"

SKIP_BUILD=false
if [[ "${1:-}" == "--run" ]]; then
    SKIP_BUILD=true
fi

# --- Build ---
if [[ "${SKIP_BUILD}" == false ]]; then
    echo "==> Building executor test for riscv32..."
    bazel build --config=qemu_riscv32 //executor/tests:qemu_riscv32_test
fi

# --- Locate binary ---
ELF="$(find bazel-out/ -path '*/executor/tests/qemu_riscv32_test' -not -path '*/runfiles/*' -not -name '*.d' -not -name '*.params' -type f 2>/dev/null | head -1)"
if [[ -z "${ELF}" || ! -f "${ELF}" ]]; then
    echo "ERROR: Binary not found. Build first (omit --run)." >&2
    exit 1
fi

# --- Check QEMU ---
QEMU_BIN="qemu-system-riscv32"
if ! command -v "${QEMU_BIN}" &>/dev/null; then
    echo "ERROR: ${QEMU_BIN} not found. Install with: sudo apt install qemu-system-misc" >&2
    exit 1
fi

# --- Run ---
echo "==> Running on QEMU virt (riscv32, 640K firmware, semihosting)..."
echo ""
# --foreground is required so QEMU's semihosting exit propagates
# correctly when invoked from a script (timeout creates a new process
# group otherwise, which prevents clean termination).
EXIT=0
timeout --foreground 30 "${QEMU_BIN}" \
    -machine virt \
    -cpu rv32 \
    -m 128M \
    -nographic \
    -semihosting-config enable=on,target=native \
    -bios none \
    -kernel "${ELF}" || EXIT=$?
echo ""

if [[ ${EXIT} -eq 0 ]]; then
    echo "==> TEST PASSED"
else
    echo "==> TEST FAILED (exit code ${EXIT})"
fi

exit ${EXIT}
