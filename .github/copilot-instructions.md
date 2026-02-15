# OpenPRoT — AI Agent Instructions

## Architecture

Layered `no_std` embedded firmware platform for Root-of-Trust operations (OpenTitan Earl Grey RISC-V).

```
Services (storage, telemetry)
  ↓
Platform Traits (platform/traits/hubris/ — concrete types for IDL codegen)
  ↓
Platform Implementations (platform/impls/{rustcrypto, mock, linux, tock, hubris})
  ↓
HAL Traits (hal/{blocking, async, nb} — three execution models)
  ↓
Hardware / RTOS (Hubris, Tock, bare-metal)
```

- **HAL traits** define the Init → Context → Op lifecycle; see `hal/blocking/src/digest.rs` for the canonical pattern
- **Scoped vs Owned** dual-API: scoped (borrowed) for baremetal one-shot, owned (move-based) for server/IPC with controller recovery
- **Hubris platform traits** use concrete types (not generics) for IDL compatibility — see `platform/traits/hubris/src/lib.rs`

## Build and Test

**Dual build system**: Cargo for workspace development, Bazel for embedded Earl Grey target.

```sh
# Primary dev workflow (runs fmt, clippy, headers, tests, build)
cargo xtask precheckin

# Individual commands
cargo xtask fmt             # rustfmt (add --check for CI)
cargo xtask clippy           # clippy with -D warnings
cargo xtask test             # cargo test
cargo xtask build            # cargo build
cargo xtask deny             # license/advisory/ban checks
cargo xtask header-check     # Apache-2.0 header enforcement
cargo xtask header-fix       # auto-add missing headers

# Bazel (Earl Grey RISC-V embedded target)
bazelisk build //target/earlgrey/...
bazelisk test //target/earlgrey/...
```

**Toolchain**: Rust nightly-2025-07-20, edition 2021, target `riscv32imc-unknown-none-elf`.

## Code Style

### Crate-level attributes (required on all HAL/platform crates)
```rust
#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
```

### Error definition pattern (every HAL module follows this exactly)
```rust
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[non_exhaustive]
pub enum ErrorKind { /* variants */ }

pub trait Error: core::fmt::Debug {
    fn kind(&self) -> ErrorKind;
}

impl Error for core::convert::Infallible {
    fn kind(&self) -> ErrorKind { match *self {} }
}

pub trait ErrorType {
    type Error: Error;
}
```

See `hal/blocking/src/cipher.rs`, `digest.rs`, `mac.rs` for reference implementations.

### Tests
- Inline only: `#[cfg(test)] mod tests { ... }` — no separate test directories
- Test modules allow `#[allow(clippy::unwrap_used)]` — the only place `unwrap()` is permitted
- Test both happy paths and error/recovery paths
- Verify controller recovery after owned-API operations

### Documentation
- `//!` module docs on every public module with purpose and examples
- `///` on all public items
- Security considerations documented inline
- Every file must start with `// Licensed under the Apache-2.0 license`

## Project Conventions

- **`#[non_exhaustive]`** on all error enums for forward compatibility
- **`#[repr(C)]`** on hardware-facing types (e.g., `Digest<N>`)
- **Marker traits** for type-safe mode selection (see cipher `CipherMode`, `BlockCipherMode`, `AeadCipherMode`)
- **Zero-copy** via `zerocopy::{FromBytes, IntoBytes}` for hardware buffer types
- **`SecureKey<N>`** with `Zeroize + ZeroizeOnDrop` + Debug prints `[REDACTED]`
- **Constant-time** via `subtle::ConstantTimeEq` for all secret comparisons
- **No heap**: use `heapless::Vec<T, N>`, `heapless::String<N>`, fixed-size arrays `[T; N]`

## Forbidden Patterns

| Forbidden | Required Alternative |
|-----------|----------------------|
| `value.unwrap()` | `match value { Some(v) => v, None => return Err(...) }` |
| `result.expect("msg")` | `match result { Ok(v) => v, Err(e) => return Err(e.into()) }` |
| `collection[index]` | `collection.get(index).ok_or(Error::OutOfBounds)?` |
| `a + b` (integers) | `a.checked_add(b).ok_or(Error::Overflow)?` |
| `ptr.read()` | `ptr.read_volatile()` (for MMIO) |
| `Vec<T>`, `HashMap<K,V>` | Fixed-size arrays `[T; N]`, `heapless::Vec<T, N>` |
| `String` | `heapless::String<N>` or `&str` |
| `Box<T>` | Stack allocation or `&mut T` |

## Security

- **Timing attacks**: Use `subtle` crate for constant-time comparisons on secrets
- **Zeroization**: Use `zeroize` crate; derive `ZeroizeOnDrop` on types holding key material
- **Key handles**: `KeyHandle` marker trait exposes no methods — prevents raw key extraction
- **Error messages**: Never leak sensitive data (key bytes, internal state) in errors or Debug output
- **Hardware access**: All register I/O must go through HAL traits, never raw pointer dereference
- **Dependency policy** (`deny.toml`): only crates.io, allowed licenses: MIT, Apache-2.0, BSD-2/3, ISC

## PR Review Checklist

- [ ] Panic-free: no `unwrap`/`expect`/`panic!`/direct indexing outside tests
- [ ] All fallible operations return `Result` or `Option`
- [ ] Integer ops use `checked_*`/`saturating_*`/`wrapping_*`
- [ ] `#![no_std]` compatible — no heap allocation
- [ ] Error enums are `#[non_exhaustive]`
- [ ] Unsafe blocks documented with `// SAFETY:` comments
- [ ] Crypto uses constant-time comparisons and zeroization
- [ ] License header present: `// Licensed under the Apache-2.0 license`
- [ ] Tests cover error paths, not just happy paths

---

## Integrating a New SoC with Pigweed (Out-of-Tree)

This section describes exactly how `target/earlgrey/` integrates the OpenTitan
Earl Grey RISC-V SoC with the Pigweed kernel in an out-of-tree Bazel project.
Follow these steps to integrate a different SoC (e.g., an ARM Cortex-M, another
RISC-V, or a custom core).

### Overview of the Integration Pattern

The Earl Grey integration is structured as a self-contained `target/<soc_name>/`
directory tree that provides everything Pigweed's `pw_kernel` needs:

```
target/<soc_name>/
├── BUILD.bazel              # Platform definition, entry, console, config libs
├── defs.bzl                 # TARGET_COMPATIBLE_WITH guard macro
├── config.rs                # KernelConfig: clocks, memory map, PLIC, timers
├── entry.rs                 # Boot entry point (#[no_main], arch init, kernel::main)
├── <mpu_or_pmp>.rs          # Memory protection unit setup (ePMP for RISC-V, MPU for ARM)
├── console.rs               # Console backend: UART driver for pw_log output
├── target.ld.jinja          # Linker script template (Jinja2, included by Pigweed)
├── registers/               # Per-peripheral register crates (via ureg or svd2rust)
│   ├── BUILD.bazel
│   ├── registers.rs         # Umbrella re-export crate
│   ├── uart.rs              # One file per peripheral
│   ├── gpio.rs
│   └── ...
├── signing/                 # Image signing keys and token configs (SoC-specific)
│   ├── keys/
│   │   ├── BUILD.bazel      # keyset() rules for FPGA and silicon keys
│   │   ├── defs.bzl         # FPGA_ECDSA_KEY, SILICON_ECDSA_KEY constants
│   │   └── *.pub.der        # Public key files
│   └── tokens/
│       ├── BUILD.bazel      # signing_tool() rules (local and HSM)
│       └── *.yaml           # HSM/CloudKMS token configurations
├── tooling/                 # Runner and signing Starlark rules
│   ├── BUILD.bazel          # Runfile bindings for devbundle artifacts
│   ├── <soc>_runner.bzl     # Rule to run/test on hardware/simulator
│   ├── <soc>_runner.py      # Python script for board communication
│   └── signing/             # Signing infrastructure (.bzl rules)
├── threads/kernel/          # Kernel-only test image (no userspace apps)
│   ├── BUILD.bazel
│   ├── system.json5
│   └── target.rs
├── ipc/user/                # IPC test image (kernel + userspace apps)
│   ├── BUILD.bazel
│   ├── system.json5
│   └── target.rs
├── syscall_latency/         # Syscall benchmark image
│   ├── BUILD.bazel
│   ├── system.json5
│   ├── target.rs
│   └── main.rs
└── unittest_runner/         # Unit test runner image
    ├── BUILD.bazel
    ├── system.json5
    └── target.rs
```

### Step 1: MODULE.bazel — Repo-Level Pigweed Integration

The root `MODULE.bazel` configures all Bazel dependencies. When adding a new SoC,
you modify this file to add SoC-specific external dependencies.

**Required dependencies (already present, shared by all SoCs):**
```starlark
bazel_dep(name = "pigweed")                    # Pigweed framework
bazel_dep(name = "rules_rust", version = "0.66.0")
bazel_dep(name = "platforms", version = "1.0.0")
bazel_dep(name = "ureg")                       # Register crate generator (Caliptra)

git_override(
    module_name = "pigweed",
    commit = "<pigweed_commit_hash>",
    remote = "https://pigweed.googlesource.com/pigweed/pigweed",
)
```

**Rust toolchain setup (shared):**
```starlark
pw_rust = use_extension("@pigweed//pw_toolchain/rust:extensions.bzl", "pw_rust")
pw_rust.toolchain(cipd_tag = "<rust_toolchain_tag>")
use_repo(pw_rust, "pw_rust_toolchains")
```

**Toolchain registration — add your SoC's C toolchain here:**
```starlark
register_toolchains(
    # Host toolchains (always needed)
    "@pigweed//pw_toolchain/host_clang:host_cc_toolchain_linux",
    "@pigweed//pw_toolchain/host_clang:host_cc_toolchain_macos",
    # RISC-V (Earl Grey):
    "@pigweed//pw_toolchain/riscv_clang:riscv_clang_cc_toolchain_rv32imc",
    # ARM Cortex-M (example for a new SoC):
    # "@pigweed//pw_toolchain/arm_clang:arm_clang_cc_toolchain_cortex_m4",
    "@pw_rust_toolchains//:all",
)
```

**Crate universe — add SoC-specific Rust embedded triples:**
```starlark
crate.from_cargo(
    name = "rust_crates",
    cargo_lockfile = "//third_party/crates_io:Cargo.lock",
    manifests = ["//third_party/crates_io:Cargo.toml"],
    supported_platform_triples = [
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        # Add your SoC target triple:
        "riscv32imc-unknown-none-elf",      # Earl Grey
        # "thumbv7em-none-eabihf",           # ARM Cortex-M4F example
    ],
)
```

**SoC-specific devbundle (optional):**
If your SoC has simulator/runner tools, add them similarly to `opentitan_devbundle`:
```starlark
bazel_dep(name = "opentitan_devbundle")
archive_override(
    module_name = "opentitan_devbundle",
    integrity = "sha256-...",
    url = "https://storage.googleapis.com/.../devbundle.tar.xz",
)
```

**Third-party Rust deps** go into `third_party/crates_io/Cargo.toml`:
```toml
[dependencies]
riscv = "0.12.1"
riscv-rt = "0.12.2"
embedded-io = "0.6.1"
# Add SoC-specific crates:
# cortex-m = "0.7"
# cortex-m-rt = "0.7"
```

### Step 2: Platform Definition — `target/<soc>/BUILD.bazel`

This is the most important file. It defines the Bazel `platform()` that tells
Pigweed about your SoC's architecture, extensions, and kernel configuration.

**Earl Grey reference** (RISC-V with IMC extensions, no atomic, Smepmp):
```starlark
load("@pigweed//pw_build:merge_flags.bzl", "flags_from_dict")
load("@pigweed//pw_kernel:flags.bzl", "KERNEL_DEVICE_COMMON_FLAGS")
load("@rules_rust//rust:defs.bzl", "rust_library")
load("//target/<soc>:defs.bzl", "TARGET_COMPATIBLE_WITH")

platform(
    name = "<soc>",
    constraint_values = [
        # 1. Custom constraint marking this as your SoC target
        ":target_<soc>",

        # 2. Architecture timer type (pick one from pw_kernel/arch/)
        "@pigweed//pw_kernel/arch/riscv:timer_mtime",        # RISC-V mtime
        # "@pigweed//pw_kernel/arch/arm_cortex_m:timer_systick", # ARM SysTick

        # 3. ISA extensions your SoC supports
        # RISC-V extensions:
        "@pigweed//pw_build/constraints/riscv/extensions:I",
        "@pigweed//pw_build/constraints/riscv/extensions:M",
        "@pigweed//pw_build/constraints/riscv/extensions:C",
        "@pigweed//pw_build/constraints/riscv/extensions:A.not",  # No atomics
        "@pigweed//pw_build/constraints/riscv/extensions:Smepmp", # ePMP
        # ARM: no extension constraints needed, they're in the CPU type

        # 4. Rust no_std constraint
        "@pigweed//pw_build/constraints/rust:no_std",

        # 5. CPU and OS
        "@platforms//cpu:riscv32",       # or @platforms//cpu:armv7e-m
        "@platforms//os:none",
    ],
    flags = flags_from_dict(
        KERNEL_DEVICE_COMMON_FLAGS | {
            # Point Pigweed to your kernel config, console backend, etc.
            "@pigweed//pw_kernel/arch/riscv:exceptions_reload_pmp": True,
            "@pigweed//pw_kernel/config:kernel_config": ":config",
            "@pigweed//pw_kernel/subsys/console:console_backend": ":console",
            "@pigweed//pw_log/rust:pw_log_backend": "@pigweed//pw_kernel:log_backend_basic",
        },
    ),
    visibility = [":__subpackages__"],
)
```

**Target type flag** (for multi-variant builds like silicon/FPGA/simulator):
```starlark
string_flag(
    name = "target_type",
    build_setting_default = "silicon",
    values = ["silicon", "fpga", "verilator"],  # Adjust for your SoC
)

config_setting(name = "silicon", flag_values = {":target_type": "silicon"})
config_setting(name = "fpga",    flag_values = {":target_type": "fpga"})
# ... add more as needed
```

**Custom constraint value** (required reference for platform compatibility):
```starlark
constraint_value(
    name = "target_<soc>",
    constraint_setting = "@pigweed//pw_kernel/target:target",
    visibility = [":__subpackages__"],
)
```

**Three core libraries** are defined in this same BUILD file:

#### 2a. Entry library (`entry`)
```starlark
rust_library(
    name = "entry",
    srcs = ["entry.rs", "<mpu_setup>.rs"],
    crate_features = select({...}),  # silicon/fpga/verilator variants
    edition = "2024",
    tags = ["kernel"],
    target_compatible_with = TARGET_COMPATIBLE_WITH,
    deps = [
        ":config",
        "@pigweed//pw_kernel/arch/riscv:arch_riscv",      # or arch/arm_cortex_m
        "@pigweed//pw_kernel/kernel",
        "@pigweed//pw_kernel/lib/memory_config",
        "@rust_crates//:riscv-rt",                         # or cortex-m-rt
    ],
)
```

#### 2b. Console library (`console`)
```starlark
rust_library(
    name = "console",
    srcs = ["console.rs"],
    crate_name = "console_backend",
    edition = "2024",
    target_compatible_with = TARGET_COMPATIBLE_WITH,
    deps = [
        "//target/<soc>/registers",     # Your SoC's UART registers
        "@pigweed//pw_kernel/arch/riscv:arch_riscv",
        "@pigweed//pw_kernel/kernel",
        "@pigweed//pw_status/rust:pw_status",
        "@rust_crates//:embedded-io",
    ],
)
```

#### 2c. Config library (`config`)
```starlark
rust_library(
    name = "config",
    srcs = ["config.rs"],
    crate_features = select({...}),
    crate_name = "kernel_config",
    edition = "2024",
    target_compatible_with = TARGET_COMPATIBLE_WITH,
    deps = [
        "@pigweed//pw_kernel/config:kernel_config_interface",
        "@pigweed//pw_kernel/lib/memory_config",
    ],
)
```

#### 2d. Linker script template
```starlark
filegroup(
    name = "linker_script_template",
    srcs = ["target.ld.jinja"],
    visibility = [":__subpackages__"],
)
```

### Step 3: Compatibility Guard — `target/<soc>/defs.bzl`

Prevents rules from being evaluated on the wrong platform:
```starlark
TARGET_COMPATIBLE_WITH = select({
    "//target/<soc>:target_<soc>": [],
    "//conditions:default": ["@platforms//:incompatible"],
})
```

### Step 4: Kernel Configuration — `config.rs`

Implements the Pigweed kernel config traits with your SoC's hardware parameters.

**Key items to fill in from your SoC datasheet:**
```rust
#![no_std]

use core::ops::Range;
use memory_config::{MemoryRegion, MemoryRegionType};

pub use kernel_config::{
    ExceptionMode, KernelConfigInterface, MTimeTimerConfigInterface,
    PlicConfigInterface, RiscVKernelConfigInterface,
};

// ── Memory Map (from SoC datasheet) ──
const FLASH_BASE: usize = 0x...;     // Code flash / execute-in-place region
const FLASH_SIZE: usize = 0x...;
const UART0_BASE: usize = 0x...;     // Console UART
const UART0_SIZE: usize = 0x...;
const TIMER_BASE: usize = 0x...;     // Timer peripheral
const TIMER_SIZE: usize = 0x...;
const PLIC_BASE:  usize = 0x...;     // Interrupt controller
const PLIC_SIZE:  usize = 0x...;

pub struct KernelConfig;

impl KernelConfigInterface for KernelConfig {
    // Clock speeds per target variant:
    #[cfg(feature = "silicon")] const SYSTEM_CLOCK_HZ: u64 = 100_000_000;
    #[cfg(feature = "fpga")]    const SYSTEM_CLOCK_HZ: u64 = 6_000_000;
}

// For RISC-V SoCs:
impl RiscVKernelConfigInterface for KernelConfig {
    type Timer = TimerConfig;

    const MTIME_HZ: u64 = KernelConfig::SYSTEM_CLOCK_HZ;
    const PMP_ENTRIES: usize = 16;           // From SoC spec
    const PMP_USERSPACE_ENTRIES: Range<usize> = 3..15;  // Range for user tasks
    const PMP_GRANULARITY: usize = 0;        // Usually 0 for 4-byte granularity

    const KERNEL_MEMORY_REGIONS: &'static [MemoryRegion] = &[
        MemoryRegion::new(MemoryRegionType::ReadWriteData, FLASH_BASE, FLASH_BASE + FLASH_SIZE),
        MemoryRegion::new(MemoryRegionType::Device, UART0_BASE, UART0_BASE + UART0_SIZE),
        MemoryRegion::new(MemoryRegionType::Device, TIMER_BASE, TIMER_BASE + TIMER_SIZE),
        MemoryRegion::new(MemoryRegionType::Device, PLIC_BASE,  PLIC_BASE  + PLIC_SIZE),
    ];

    fn get_exception_mode() -> ExceptionMode {
        ExceptionMode::Vectored(unsafe { MTVEC_TABLE.as_ptr() as usize })
    }
}

// For ARM Cortex-M: implement ArmCortexMKernelConfigInterface instead.

pub struct PlicConfig;
impl PlicConfigInterface for PlicConfig {
    const PLIC_BASE_ADDRESS: usize = PLIC_BASE;
    const MAX_IRQS: u32 = 186;  // From your SoC's interrupt map
}

pub struct TimerConfig;
impl MTimeTimerConfigInterface for TimerConfig {
    const MTIME_REGISTER:          usize = TIMER_BASE + 0x110;
    const MTIMECMP_REGISTER:       usize = TIMER_BASE + 0x118;
    const TIMER_CTRL_REGISTER:     usize = TIMER_BASE + 0x04;
    const TIMER_INTR_ENABLE_REGISTER: usize = TIMER_BASE + 0x100;
    const TIMER_INTR_STATE_REGISTER:  usize = TIMER_BASE + 0x104;
}

// RISC-V external interrupt vector table (from your SoC's vector layout):
unsafe extern "C" {
    #[link_name = "_mtvec_table"]
    static MTVEC_TABLE: [u32; 32];
}
```

### Step 5: Boot Entry Point — `entry.rs`

Minimal boot code that initializes memory protection and calls `kernel::main`.

```rust
#![no_std]
#![no_main]
use core::arch::global_asm;

// Use the correct arch module:
use arch_riscv::Arch;        // RISC-V
// use arch_arm_cortex_m::Arch; // ARM Cortex-M
use kernel::{self as _};

mod <mpu_setup>;   // your memory protection init module (epmp, mpu, etc.)

#[unsafe(no_mangle)]
#[allow(non_snake_case)]
pub unsafe extern "C" fn pw_assert_HandleFailure() -> ! {
    use kernel::Arch as _;
    Arch::panic()
}

#[riscv_rt::entry]               // or #[cortex_m_rt::entry] for ARM
fn main() -> ! {
    kernel::static_init_state!(static mut INIT_STATE: InitKernelState<Arch>);
    <mpu_setup>::init();          // Set up memory protection

    #[allow(static_mut_refs)]
    kernel::main(Arch, unsafe { &mut INIT_STATE });
}

// For RISC-V: include the machine trap vector table via global_asm!
// This is SoC-specific — see entry.rs for the Earl Grey vector table layout.
global_asm!("...");
```

### Step 6: Memory Protection Setup — `epmp.rs` / `mpu.rs`

Configure your SoC's memory protection unit before the kernel starts scheduling.

**For RISC-V (ePMP/PMP):**
- Lock kernel code as execute-only (entry 0-1 ToR)
- Lock kernel rodata as read-only (entry 2 ToR)
- Lock RAM as read-write NaPOT (entry 15)
- Zero unused PMP entries
- Set `MSeccfg` with `MML=true`, `MMWP=true`, `RLB=false`
- Write `KERNEL_THREAD_MEMORY_CONFIG`

See `target/earlgrey/epmp.rs` for the complete reference implementation.

**For ARM (MPU):**
- Configure MPU regions for flash (RX), SRAM (RW), peripherals (device)
- Set background region and privilege defaults
- Enable MPU before kernel init

### Step 7: Console Backend — `console.rs`

Provides `console_backend_write_all()` for Pigweed's `pw_log` output.

```rust
#![no_std]

use kernel::sync::spinlock::SpinLock;
use pw_status::Result;
use registers::<uart_module>;  // Your SoC's UART register crate

struct Uart {
    device: <uart_module>::Uart0,
}

impl Uart {
    fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        let reg = self.device.regs_mut();
        for &byte in buf.iter() {
            while reg.status().read().txfull() {
                // Busy-wait while TX FIFO is full
            }
            reg.wdata().write(|w| w.wdata(byte as u32));
        }
        Ok(())
    }
}

static UART: SpinLock<arch_riscv::Arch, Uart> = SpinLock::new(Uart {
    device: unsafe { <uart_module>::Uart0::new() },
});

// This exact function signature is required by Pigweed:
#[unsafe(no_mangle)]
pub fn console_backend_write_all(buf: &[u8]) -> Result<()> {
    let mut uart = UART.lock(arch_riscv::Arch);
    uart.write_all(buf)
}
```

### Step 8: Linker Script — `target.ld.jinja`

Jinja2 template that Pigweed processes via `target_linker_script()`.

**Key sections to define (from your SoC memory map):**
```ld
MEMORY
{
  /* Adjust origins and lengths from your SoC datasheet */
  ROM_EXT(rx) : ORIGIN = 0x..., LENGTH = 0x...    /* Optional boot stage */
  FLASH(rx) :   ORIGIN = 0x..., LENGTH = 0x...    /* Application code */
  RAM(rw) :     ORIGIN = 0x..., LENGTH = 0x...    /* SRAM */
}
```

**Required sections:**
- `.code` — executable code, trap handlers, rodata
- `.static_init_ram` — initialized data (loaded from FLASH to RAM)
- `.zero_init_ram` — BSS (zeroed at boot)
- `.stack` — kernel stack
- Symbols: `_code_start`, `_code_end`, `_kernel_end`, `_ram_start`, `_ram_end`
- `riscv-rt` symbols: `_sbss`, `_ebss`, `_sdata`, `_edata`, `_sidata`, `REGION_*` aliases
- Pigweed footer: `{% include "pigweed_linker_sections.ld.jinja" %}`

**For SoCs with secure boot manifests** (like OpenTitan), add a `.manifest` section
with the boot ROM's expected header layout. See `target/earlgrey/target.ld.jinja`
for the OpenTitan manifest structure.

### Step 9: Register Definitions — `registers/`

Each peripheral gets a separate `rust_library` using the `ureg` crate (or `svd2rust`).

**Pattern for each peripheral register file (generated or hand-written):**
```rust
#![no_std]
#![allow(clippy::erasing_op)]
#![allow(clippy::identity_op)]

pub struct <Peripheral> { _priv: () }
impl <Peripheral> {
    pub const PTR: *mut u32 = 0x<base_address> as *mut u32;

    pub const unsafe fn new() -> Self { Self { _priv: () } }

    pub fn regs(&self) -> RegisterBlock<ureg::RealMmio<'_>> {
        RegisterBlock { ptr: Self::PTR, mmio: core::default::Default::default() }
    }
    pub fn regs_mut(&mut self) -> RegisterBlock<ureg::RealMmioMut<'_>> {
        RegisterBlock { ptr: Self::PTR, mmio: core::default::Default::default() }
    }
}
```

**BUILD.bazel pattern:**
```starlark
rust_library(
    name = "<peripheral>",
    srcs = ["<peripheral>.rs"],
    edition = "2024",
    visibility = ["//visibility:public"],
    deps = ["@ureg"],
)
```

**Umbrella re-export crate** (`registers.rs`):
```rust
#![no_std]
pub extern crate uart;
pub extern crate gpio;
// ... one line per peripheral
```

**Alternative: SVD-based generation.** If your SoC has an SVD file, use `svd2rust`
to generate register crates instead of hand-writing `ureg` files.

### Step 10: System Configuration — `system.json5`

Each test image has a `system.json5` that tells Pigweed's code generator about
your SoC's memory layout and thread/process definitions.

**Kernel-only image** (simplest — for `threads/kernel/` or `unittest_runner/`):
```json5
{
  arch: { type: "riscv" },   // or "arm_cortex_m"
  kernel: {
    flash_start_address: 0xA0010000,  // Must match FLASH ORIGIN in linker script
    flash_size_bytes: 65536,
    ram_start_address: 0x10000000,    // Must match RAM ORIGIN
    ram_size_bytes: 32768,
    interrupt_table: { table: {} },
  },
}
```

**With userspace apps** (for IPC tests, real applications):
```json5
{
  arch: { type: "riscv" },
  kernel: {
    flash_start_address: 0xA0010000,
    flash_size_bytes: 65536,
    ram_start_address: 0x10000000,
    ram_size_bytes: 32768,
    interrupt_table: { table: {} },
  },
  apps: [
    {
      name: "my_app",
      flash_size_bytes: 16384,
      ram_size_bytes: 4096,
      process: {
        name: "my app process",
        // Optional: memory-mapped device regions for userspace drivers
        memory_mappings: [
          { name: "TIMER", type: "device", start_address: 0x40100000, size_bytes: 0x200 },
        ],
        // Optional: IPC channel objects
        objects: [
          { name: "IPC", type: "channel_initiator", handler_app: "handler", handler_object_name: "IPC" },
        ],
        threads: [
          { name: "main thread", stack_size_bytes: 1024 },
        ],
      },
    },
  ],
}
```

### Step 11: Target Binary — `target.rs`

Each test image has a `target.rs` that implements Pigweed's `TargetInterface`:

```rust
#![no_std]
#![no_main]

use target_common::{declare_target, TargetInterface};
use {console_backend as _, entry as _};
// For kernel-only: use {codegen as _, console_backend as _, entry as _};

pub struct Target {}

impl TargetInterface for Target {
    const NAME: &'static str = "<SoC> <Test Description>";

    fn main() -> ! {
        codegen::start();  // Start the generated system (threads, IPC, etc.)
        loop {}
    }

    fn shutdown(code: u32) -> ! {
        pw_log::info!("Shutting down with code {}", code as u32);
        match code {
            0 => pw_log::info!("PASS"),
            _ => pw_log::info!("FAIL: {}", code as u32),
        };
        loop {}
    }
}

declare_target!(Target);
```

### Step 12: Image BUILD Rules — Pigweed Tooling Macros

Each test image BUILD.bazel follows this exact pattern:

```starlark
load("@pigweed//pw_kernel/tooling:system_image.bzl", "system_image")
load("@pigweed//pw_kernel/tooling:target_codegen.bzl", "target_codegen")
load("@pigweed//pw_kernel/tooling:target_linker_script.bzl", "target_linker_script")
load("@pigweed//pw_kernel/tooling/panic_detector:rust_binary_no_panics_test.bzl", "rust_binary_no_panics_test")
load("@rules_rust//rust:defs.bzl", "rust_binary")
load("//target/<soc>:defs.bzl", "TARGET_COMPATIBLE_WITH")

# 1. System image: combines kernel + apps into a single flashable binary
system_image(
    name = "<test_name>",
    apps = ["@pigweed//pw_kernel/tests/ipc/user:initiator", ...],  # optional
    kernel = ":target",
    platform = "//target/<soc>",
    system_config = ":system_config",        # only needed if apps are specified
)

# 2. Linker script: generated from your Jinja2 template + system config
target_linker_script(
    name = "linker_script",
    system_config = ":system_config",
    tags = ["kernel"],
    template = "//target/<soc>:linker_script_template",
)

# 3. Panic detection test: ensures the binary has no panics
rust_binary_no_panics_test(
    name = "no_panics_test",
    binary = ":<test_name>",
)

# 4. System config: the JSON5 describing memory layout and processes
filegroup(
    name = "system_config",
    srcs = ["system.json5"],
)

# 5. Code generation: generates Rust glue code from system.json5
target_codegen(
    name = "codegen",
    arch = "@pigweed//pw_kernel/arch/riscv:arch_riscv",  # or arm_cortex_m
    system_config = ":system_config",
)

# 6. Target binary: the actual kernel entry point
rust_binary(
    name = "target",
    srcs = ["target.rs"],
    edition = "2024",
    target_compatible_with = TARGET_COMPATIBLE_WITH,
    deps = [
        ":codegen",
        ":linker_script",
        "//target/<soc>:entry",
        "@pigweed//pw_kernel/arch/riscv:arch_riscv",
        "@pigweed//pw_kernel/kernel",
        "@pigweed//pw_kernel/subsys/console:console_backend",
        "@pigweed//pw_kernel/target:target_common",
        "@pigweed//pw_kernel/userspace",      # only if apps exist
        "@pigweed//pw_log/rust:pw_log",
    ],
)
```

### Step 13: Runner and Test Rules (Optional)

If your SoC has a simulator or hardware board runner, create custom runner rules.

**Earl Grey pattern** (reusable template):

1. **`tooling/BUILD.bazel`** — bind devbundle artifacts as Python runfiles
2. **`tooling/<soc>_runner.bzl`** — Starlark rule that:
   - Uses a transition to set `target_type` (silicon/fpga/verilator)
   - Optionally signs the binary
   - Generates a shell script calling your runner with the ELF/BIN
3. **`tooling/<soc>_runner.py`** — Python script that:
   - Invokes your SoC's tool (e.g., `opentitantool`, `openocd`, `probe-rs`)
   - Pipes output through `pw_tokenizer.detokenize` for `pw_log` decoding
   - Supports `--exit-success` / `--exit-failure` regex for test pass/fail

### Step 14: Signing Infrastructure (Optional)

Only needed for SoCs with secure boot. The signing infrastructure has these layers:

1. **`signing/keys/defs.bzl`** — exports key constants:
   ```starlark
   FPGA_ECDSA_KEY = {"//target/<soc>/signing/keys:fpga_keyset": "app_prod_0"}
   SILICON_ECDSA_KEY = {"//target/<soc>/signing/keys:gb00_keyset": "gb00-app-key-prod-0"}
   ```

2. **`signing/keys/BUILD.bazel`** — defines `keyset()` rules with key files and profiles

3. **`signing/tokens/BUILD.bazel`** — defines `signing_tool()` rules for:
   - `local`: on-disk private keys (FPGA dev)
   - `token`: HSM-backed keys via PKCS#11 (silicon production)

4. **`tooling/signing/*.bzl`** — Starlark macros (`sign_binary`, `presigning_artifacts`,
   `post_signing_attach`) that orchestrate the signing pipeline

### Step 15: Build and Verify

```sh
# Build all images for your new SoC
bazelisk build //target/<soc>/...

# Run on simulator (if you have a runner)
bazelisk run //target/<soc>/threads/kernel:threads_runner_<simulator>

# Run tests
bazelisk test //target/<soc>/unittest_runner:<interface>_test

# Verify no panics in binary
bazelisk test //target/<soc>/threads/kernel:no_panics_test
```

### Checklist for a Minimal New SoC Integration

| # | File | What to provide |
|---|------|-----------------|
| 1 | `MODULE.bazel` | Add C toolchain, Rust triple, devbundle (if any) |
| 2 | `third_party/crates_io/Cargo.toml` | Add SoC-specific Rust crates |
| 3 | `target/<soc>/defs.bzl` | Copy and rename `TARGET_COMPATIBLE_WITH` |
| 4 | `target/<soc>/BUILD.bazel` | Platform + entry/console/config libs |
| 5 | `target/<soc>/config.rs` | Memory map, clocks, PLIC, timer registers |
| 6 | `target/<soc>/entry.rs` | Boot entry, assert handler, vector table |
| 7 | `target/<soc>/<mpu>.rs` | Memory protection init |
| 8 | `target/<soc>/console.rs` | UART driver with `console_backend_write_all` |
| 9 | `target/<soc>/target.ld.jinja` | Linker script with MEMORY regions |
| 10 | `target/<soc>/registers/` | Per-peripheral register crates |
| 11 | `target/<soc>/threads/kernel/` | Minimal kernel-only test image |

Items 12-14 (runner, signing, IPC tests) are optional for initial bringup.
