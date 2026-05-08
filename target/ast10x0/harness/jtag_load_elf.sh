#!/usr/bin/env bash
# Licensed under the Apache-2.0 license
#
# Load an ELF onto AST10x0 over JTAG using GDB + debugger backend.
# Supports both OpenOCD (generic adapters) and J-Link (Segger hardware).
#
# Examples:
#   OpenOCD (CMSIS-DAP, ST-Link, etc):
#   ./target/ast10x0/harness/jtag_load_elf.sh \
#     --elf bazel-bin/target/ast10x0/tests/threads/kernel/threads.elf \
#     --backend openocd \
#     --interface interface/cmsis-dap.cfg \
#     --target target/ast1030.cfg
#
#   J-Link (Segger):
#   ./target/ast10x0/harness/jtag_load_elf.sh \
#     --elf bazel-bin/target/ast10x0/tests/threads/kernel/threads.elf \
#     --backend jlink \
#     --device cortex-m4 \
#     --ifspeed 1000
#
# Defaults can be overridden with env vars:
#   GDB, OPENOCD, JLINKGDBSERVER, OPENOCD_INTERFACE_CFG, OPENOCD_TARGET_CFG

set -euo pipefail

GDB_BIN="${GDB:-gdb-multiarch}"
OPENOCD_BIN="${OPENOCD:-openocd}"
JLINKGDBSERVER_BIN="${JLINKGDBSERVER:-JLinkGDBServer}"

# OpenOCD defaults
OPENOCD_INTERFACE_CFG="${OPENOCD_INTERFACE_CFG:-interface/cmsis-dap.cfg}"
OPENOCD_TARGET_CFG="${OPENOCD_TARGET_CFG:-target/ast1030.cfg}"
OPENOCD_TELNET_PORT="${OPENOCD_TELNET_PORT:-4444}"
OPENOCD_GDB_PORT="${OPENOCD_GDB_PORT:-3333}"

# J-Link defaults
JLINK_DEVICE="${JLINK_DEVICE:-cortex-m4}"
JLINK_INTERFACE="${JLINK_INTERFACE:-swd}"
JLINK_SPEED="${JLINK_SPEED:-1000}"
JLINK_GDB_PORT="${JLINK_GDB_PORT:-2331}"

ELF=""
BACKEND="openocd"
RESET_AND_RUN=0
KEEP_SERVER=0
ENABLE_SEMIHOSTING=1

usage() {
  cat <<EOF
Usage: $0 --elf <path-to-elf> [options]

Required:
  --elf <path>                 ELF image to load

Common options:
  --backend <openocd|jlink>    Debugger backend (default: ${BACKEND})
  --gdb <path>                 GDB binary (default: ${GDB_BIN})
  --reset-and-run              After load, reset and continue execution
  --keep-server                Keep debugger server running after GDB exits
  --no-semihosting             Disable semihosting (J-Link only)
  -h, --help                   Show help

OpenOCD options (use with --backend openocd):
  --interface <cfg>            OpenOCD interface cfg (default: ${OPENOCD_INTERFACE_CFG})
  --target <cfg>               OpenOCD target cfg (default: ${OPENOCD_TARGET_CFG})
  --openocd <path>             OpenOCD binary (default: ${OPENOCD_BIN})
  --gdb-port <port>            GDB server port (default: ${OPENOCD_GDB_PORT})
  --telnet-port <port>         Telnet port (default: ${OPENOCD_TELNET_PORT})

J-Link options (use with --backend jlink):
  --jlinkgdbserver <path>      JLinkGDBServer binary (default: ${JLINKGDBSERVER_BIN})
  --device <name>              Device name (default: ${JLINK_DEVICE})
  --interface <swd|jtag>       Interface type (default: ${JLINK_INTERFACE})
  --ifspeed <kHz>              Interface speed in kHz (default: ${JLINK_SPEED})
  --gdb-port <port>            GDB server port (default: ${JLINK_GDB_PORT})

Notes:
  - Thbackend)
      BACKEND="$2"
      shift 2
      ;;
    --gdb)
      GDB_BIN="$2"
      shift 2
      ;;
    --openocd)
      OPENOCD_BIN="$2"
      shift 2
      ;;
    --jlinkgdbserver)
      JLINKGDBSERVER_BIN="$2"
      shift 2
      ;;
    --interface)
      OPENOCD_INTERFACE_CFG="$2"
      JLINK_INTERFACE="$2"
      shift 2
      ;;
    --target)
      OPENOCD_TARGET_CFG="$2"
      shift 2
      ;;
    --device)
      JLINK_DEVICE="$2"
      shift 2
      ;;
    --ifspeed)
      JLINK_SPEED="$2"
      shift 2
      ;;
    --gdb-port)
      OPENOCD_GDB_PORT="$2"
      JLINK_GDB_PORT="$2"
      shift 2
      ;;
    --telnet-port)
      OPENOCD_TELNET_PORT="$2"
      shift 2
      ;;
    --reset-and-run)
      RESET_AND_RUN=1
      shift
      ;;
    --keep-server)
      KEEP_SERVER=1
      shift
      ;;
    --no-semihosting)
      ENABLE_SEMIHOSTING=0
      shift 2
      ;;
    --telnet-port)
      TELNET_PORT="$2"
      shift 2
      ;;
    --reset-and-run)
      RESET_AND_RUN=1
      shift
      ;;
    --keep-openocd)
      KEEP_OPENOCD=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 2
      ;;
  esac
done

if [[ -z "$ELF" ]]; then
  echo "--elf is required" >&2
  usage
  exit 2
fi

if [[ ! -f "$ELF" ]]; then
  echo "ELF not found: $ELF" >&2
  exit 2
fi

if ! command -v "$GDB_BIN" >/dev/null 2>&1; then
  echo "GDB not found: $GDB_BIN" >&2
  exit 2
fi

if [[ "$BACKEND" != "openocd" && "$BACKEND" != "jlink" ]]; then
  echo "Unknown backend: $BACKEND (must be openocd or jlink)" >&2
  exit 2
fi

GDB_CMDS="$(mktemp /tmp/ast10x0-gdb.XXXXXX.cmds)"
SERVER_LOG="$(mktemp /tmp/ast10x0-server.XXXXXX.log)"

cleanup() {
  rm -f "$GDB_CMDS"
  if [[ ${KEEP_SERVER} -eq 0 ]]; then
    if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" >/dev/null 2>&1; then
      kill "$SERVER_PID" >/dev/null 2>&1 || true
      wait "$SERVER_PID" 2>/dev/null || true
    fi
  fi
  echo "Server log: $SERVER_LOG"
}
trap cleanup EXIT

if [[ "$BACKEND" == "openocd" ]]; then
  if ! command -v "$OPENOCD_BIN" >/dev/null 2>&1; then
    echo "OpenOCD not found: $OPENOCD_BIN" >&2
    exit 2
  fi
  
  "$OPENOCD_BIN" \
    -f "$OPENOCD_INTERFACE_CFG" \
    -f "$OPENOCD_TARGET_CFG" \
    -c "gdb_port $OPENOCD_GDB_PORT" \
    -c "telnet_port $OPENOCD_TELNET_PORT" \
    -c "adapter speed 1000" \
    -c "init" \
    -c "reset init" \
    >"$SERVER_LOG" 2>&1 &
  SERVER_PID=$!
  GDB_PORT=$OPENOCD_GDB_PORT
  
  for _ in $(seq 1 50); do
    if grep -q "Listening on port $GDB_PORT for gdb connections" "$SERVER_LOG"; then
      break
    fi
    sleep 0.1
  done
  
  if ! grep -q "Listening on port $GDB_PORT for gdb connections" "$SERVER_LOG"; then
    echo "OpenOCD did not start correctly" >&2
    tail -n 80 "$SERVER_LOG" >&2 || true
    exit 1
  fi
  
elif [[ "$BACKEND" == "jlink" ]]; then
  if ! command -v "$JLINKGDBSERVER_BIN" >/dev/null 2>&1; then
    echo "JLinkGDBServer not found: $JLINKGDBSERVER_BIN" >&2
    exit 2
  fi
  
  "$JLINKGDBSERVER_BIN" \
    -device "$JLINK_DEVICE" \
    -if "$JLINK_INTERFACE" \
    -speed "$JLINK_SPEED" \
    -port "$JLINK_GDB_PORT" \
    >"$SERVER_LOG" 2>&1 &
  SERVER_PID=$!
  GDB_PORT=$JLINK_GDB_PORT
  
  for _ in $(seq 1 50); do
    if grep -q "Connected to target" "$SERVER_LOG" || \
       grep -q "GDBServer listening on TCP port" "$SERVER_LOG"; then
      break
    fi
    sleep 0.1
  done
  
  sleep 0.5  # Give J-Link more time to settle
fi

{
  echo "set confirm off"
  echo "set pagination off"
  echo "target extended-remote :$GDB_PORT"
  
  if [[ "$BACKEND" == "jlink" && $ENABLE_SEMIHOSTING -eq 1 ]]; then
    echo "monitor semihosting IOClient 2"
  fi
  
  echo "monitor halt"
  echo "monitor reset init"
  echo "file $ELF"
  echo "load"
  
  if [[ $RESET_AND_RUN -eq 1 ]]; then
    echo "monitor reset run"
    echo "continue"
  else
    echo "monitor reset halt"
  fi
  
  echo "quit"
} > "$GDB_CMDS"

"$GDB_BIN" -q -x "$GDB_CMDS"

echo "ELF successfully loaded: $ELF"
