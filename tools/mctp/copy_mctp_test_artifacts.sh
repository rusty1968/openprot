#!/usr/bin/env bash
set -euo pipefail

# Copies the artifacts needed to run MCTP QEMU/host-side testing.
#
# Defaults:
#   DEST_HOST=pso_rot@PSO-ROT-SER
#   DEST_DIR=~/anthony/mctp
#
# Optional env vars:
#   DEST_HOST, DEST_DIR, SSH_OPTS, SCP_OPTS, DRY_RUN=1
#   INCLUDE_HOST_TOOLS=1, INCLUDE_QEMU_BIN=1, QEMU_BIN=/path/to/qemu-system-arm

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "${SCRIPT_DIR}/../.." && pwd)"

DEST_HOST="${DEST_HOST:-pso_rot@PSO-ROT-SER}"
DEST_DIR="${DEST_DIR:-~/anthony/mctp}"
SSH_OPTS="${SSH_OPTS:-}"
SCP_OPTS="${SCP_OPTS:-}"
DRY_RUN="${DRY_RUN:-0}"
INCLUDE_HOST_TOOLS="${INCLUDE_HOST_TOOLS:-1}"
INCLUDE_QEMU_BIN="${INCLUDE_QEMU_BIN:-1}"

ELF_PATH="${REPO_ROOT}/bazel-bin/target/ast10x0/tests/mctp_echo/mctp_echo_image.elf"
BIN_PATH="${REPO_ROOT}/bazel-bin/target/ast10x0/tests/mctp_echo/mctp_echo_image.bin"
README_PATH="${REPO_ROOT}/tools/mctp/README.md"
HOST_TOOL_BIN="${REPO_ROOT}/tools/mctp/echo_linux/target/release/test-mctp-request"
HOST_TOOL_MANIFEST="${REPO_ROOT}/tools/mctp/echo_linux/Cargo.toml"
HOST_TOOL_README="${REPO_ROOT}/tools/mctp/echo_linux/README.md"
QEMU_BIN="${QEMU_BIN:-${HOME}/tools/qemu/v8.2.4/build/qemu-system-arm}"

required=(
  "${ELF_PATH}"
  "${BIN_PATH}"
  "${README_PATH}"
)

missing=()
for f in "${required[@]}"; do
  if [[ ! -f "${f}" ]]; then
    missing+=("${f}")
  fi
done

if (( ${#missing[@]} > 0 )); then
  echo "Missing required artifacts:" >&2
  for f in "${missing[@]}"; do
    echo "  - ${f}" >&2
  done
  echo >&2
  echo "Build them first:" >&2
  echo "  cd ${REPO_ROOT}" >&2
  echo "  bazel build --config=virt_ast10x0 //target/ast10x0/tests/mctp_echo:mctp_echo_image" >&2
  exit 1
fi

copy_list=(
  "${ELF_PATH}"
  "${BIN_PATH}"
  "${README_PATH}"
)

if [[ "${INCLUDE_HOST_TOOLS}" == "1" ]]; then
  host_missing=()
  for f in "${HOST_TOOL_BIN}" "${HOST_TOOL_MANIFEST}" "${HOST_TOOL_README}"; do
    if [[ ! -f "${f}" ]]; then
      host_missing+=("${f}")
    fi
  done

  if (( ${#host_missing[@]} > 0 )); then
    echo "Missing host-side echo_linux artifacts:" >&2
    for f in "${host_missing[@]}"; do
      echo "  - ${f}" >&2
    done
    echo >&2
    echo "Build host tool first:" >&2
    echo "  cd ${REPO_ROOT}" >&2
    echo "  cargo build --release --manifest-path tools/mctp/echo_linux/Cargo.toml" >&2
    exit 1
  fi

  copy_list+=(
    "${HOST_TOOL_BIN}"
    "${HOST_TOOL_MANIFEST}"
    "${HOST_TOOL_README}"
  )
fi

if [[ "${INCLUDE_QEMU_BIN}" == "1" ]]; then
  if [[ ! -f "${QEMU_BIN}" ]]; then
    echo "Missing QEMU binary: ${QEMU_BIN}" >&2
    echo "Set QEMU_BIN to your local qemu-system-arm path, or use INCLUDE_QEMU_BIN=0." >&2
    exit 1
  fi
  copy_list+=("${QEMU_BIN}")
fi

echo "Destination: ${DEST_HOST}:${DEST_DIR}"
echo "Artifacts to copy:"
for f in "${copy_list[@]}"; do
  echo "  - ${f}"
done

if [[ "${DRY_RUN}" == "1" ]]; then
  echo "DRY_RUN=1 set, skipping SSH/SCP."
  exit 0
fi

ssh ${SSH_OPTS} "${DEST_HOST}" "mkdir -p ${DEST_DIR}"
scp ${SCP_OPTS} "${copy_list[@]}" "${DEST_HOST}:${DEST_DIR}/"

echo "Copy complete."
