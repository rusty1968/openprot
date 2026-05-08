# AST10x0 Pigweed Target

Pigweed kernel target for the AST10x0 platform.

## Building

Build all targets under the AST10x0 tree with:

```console
bazel build //target/ast10x0/...
```

Run the AST10x0 test targets with:

```console
bazel test //target/ast10x0/...
```

This builds the AST10x0 test targets and any required dependencies. Firmware-
backed tests are skipped unless a runner is configured.

## Running Tests Under QEMU

Run the full AST10x0 test suite under QEMU with:

```console
bazel test --config=virt_ast10x0 //target/ast10x0/...
```

The `virt_ast10x0` config launches images with Pigweed's QEMU runner using the
`ast1030-evb` machine and semihosting.

For more detailed failures:

```console
bazel test --config=virt_ast10x0 --verbose_failures //target/ast10x0/...
```

## Loading via JTAG (Physical Hardware)

Load firmware onto AST10x0 hardware via JTAG debugger probe.

### Prerequisites

Choose one debugger backend:

- **OpenOCD** (supports CMSIS-DAP, ST-Link, and other generic adapters)
  ```bash
  sudo apt-get install openocd
  ```

- **J-Link** (Segger hardware)
  - Download JLink tools from https://www.segger.com/downloads/jlink/
  - Install JLinkGDBServer

### Build ELF Image

```console
bazel build //target/ast10x0/tests/threads/kernel:threads
```

This creates `bazel-bin/target/ast10x0/tests/threads/kernel/threads.elf`

### Load via OpenOCD (Generic Adapters)

```console
bazel run //target/ast10x0/harness:jtag_load_elf -- \
  --backend openocd \
  --elf bazel-bin/target/ast10x0/tests/threads/kernel/threads.elf \
  --interface interface/cmsis-dap.cfg \
  --target target/ast1030.cfg \
  --reset-and-run
```

To customize interface adapter:
```console
bazel run //target/ast10x0/harness:jtag_load_elf -- \
  --backend openocd \
  --elf bazel-bin/target/ast10x0/tests/threads/kernel/threads.elf \
  --interface interface/ftdi.cfg \
  --target target/ast1030.cfg
```

### Load via J-Link (Segger Hardware)

```console
bazel run //target/ast10x0/harness:jtag_load_elf -- \
  --backend jlink \
  --elf bazel-bin/target/ast10x0/tests/threads/kernel/threads.elf \
  --device cortex-m4 \
  --interface swd \
  --ifspeed 1000 \
  --reset-and-run
```

For JTAG interface instead of SWD:
```console
bazel run //target/ast10x0/harness:jtag_load_elf -- \
  --backend jlink \
  --elf bazel-bin/target/ast10x0/tests/threads/kernel/threads.elf \
  --device cortex-m4 \
  --interface jtag \
  --reset-and-run
```

### Command Options

| Option | Example | Note |
|--------|---------|------|
| `--elf` | `path/to/image.elf` | **Required** - Path to ELF binary |
| `--backend` | `openocd` or `jlink` | Debugger type (default: `openocd`) |
| `--reset-and-run` | (no value) | Reset target and start execution; without this, halts at breakpoint |
| `--keep-server` | (no value) | Keep GDB server running after script exits |
| `--gdb-port` | `3333` | GDB server port (default: `3333` for OpenOCD, `2331` for J-Link) |

### Semihosting Output

When using J-Link, semihosting output is automatically captured. Check the terminal for output from kernel logging and test results.

For more options, see the script help:
```console
bazel run //target/ast10x0/harness:jtag_load_elf -- --help
```

## Notes

- `bazel build //target/ast10x0/...` builds all targets under the AST10x0 tree.
- `bazel test //target/ast10x0/...` builds the AST10x0 test targets and any
  required dependencies, but skips bare-metal test execution.
- `bazel test --config=virt_ast10x0 //target/ast10x0/...` executes the AST10x0
  system-image tests under QEMU.
- JTAG loading requires a physical debugger probe (CMSIS-DAP or J-Link) connected to the board's JTAG interface.