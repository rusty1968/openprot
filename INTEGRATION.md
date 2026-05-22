# OpenProt Platform Integration Guide

## Intent

Enable integrators to **assemble platform-specific binaries from shared, trait-based services and drivers** without forking code.

## Bazel Design

### How It Works

**Trait-based abstraction + Bazel visibility/deps**

```
Shared (visibility: public)
├── //drivers/usart/api:usart_api         (trait definitions)
├── //services/mctp/...:mctp_server_lib   (service logic)
└── //services/mctp/transport-serial      (generic adapter)

Platform-specific (visibility: internal)
├── //target/YOUR_PLATFORM/drivers/...   (impl trait for your UART)
├── //target/YOUR_PLATFORM/serial/...    (impl SerialPort for your backend)
└── //target/YOUR_PLATFORM/...           (final binary deps on all above)
```

**Bazel's role:**
- Declares trait APIs as public deps
- Platform implementations listed as private (only used by platform)
- Bazel resolves concrete types at **link time** → **no code duplication**

### Example

```bazel
# Shared (public)
rust_library(
    name = "usart_api",
    srcs = ["backend.rs"],  # trait UsartBackend { ... }
    visibility = ["//visibility:public"],
)

# Platform-specific (private)
rust_library(
    name = "usart_backend_impl",
    srcs = ["ast10x0_impl.rs"],  # impl UsartBackend for Uart { ... }
    deps = [":usart_api"],
    visibility = ["//target/ast10x0:__subpackages__"],
)

# Binary pulls both
rust_binary(
    name = "mctp_server",
    deps = [
        "//drivers/usart/api",           # trait
        "//target/ast10x0/drivers:impl",  # concrete impl
    ],
)
```

**Bazel enforces:**
- Traits public → anyone can depend
- Impls private → only platform uses them
- Link-time resolution → no runtime polymorphism overhead

## Result

**One codebase, multiple platforms**—just change which `impl` you link against.

## Integration Layers

### 1. Trait APIs (Shared - openprot)
- `//openprot/hal` — HAL trait definitions (USART, I2C, etc.)
- `//services/mctp/api` — MCTP protocol definitions
- `//services/mctp/transport-serial` — SerialPort trait

### 2. Services and Drivers (Shared - openprot)
- `//services/mctp/server` — MCTP dispatch logic
- `//drivers/usart/server` — USART service (defines `UsartBackend` trait)
- `//drivers/usart/client` — USART IPC client

### 3. Platform Implementations (Released - target/)
- `//target/YOUR_PLATFORM/backend/usart` — Implements `UsartBackend` trait from userspace services/drivers
- `//target/YOUR_PLATFORM/backend/mctp_serial` — Implements `SerialPort` trait from `//services/mctp/transport-serial`
- `//target/YOUR_PLATFORM/mctp` — Platform-specific MCTP server

### 4. Architecture Patterns

**Direct HAL** (Single monolithic kernel/process):
- Synchronous hardware access
- Lower latency, simpler
- Platform-private: HAL UART → SerialPort impl → MCTP server

**IPC Microkernel** (Userspace drivers):
- Async syscalls via channels (UsartClient)
- Privilege separation, isolation
- Platform-private: UsartClient → SerialPort impl → MCTP server

## For Integrators

1. **Define your platform traits** in `//target/YOUR_PLATFORM/drivers/`
2. **Implement them** against your hardware HAL
3. **Wire into generic transports** via platform-specific BUILD rules
4. **Bazel links concrete impls** at build time
5. **Ship multi-platform binary** with no code duplication

## Key Files

- [services/mctp/transport-serial](services/mctp/transport-serial) — Generic serial adapter
- [target/ast10x0/serial](target/ast10x0/serial) — AST10x0 SerialPort implementations
- [drivers/usart/api](drivers/usart/api) — USART trait definitions

## Target Folder Structure (Released Implementations)

Each `target/PLATFORM/` contains **released OpenProt implementations** for a platform:

- **config.rs** — Platform config (CPU, memory, features)
- **entry.rs** — Kernel boot
- **peripherals/** — HAL implementations (impl traits from `//openprot/hal`)
- **backend/** — Backend implementations (impl traits defined by userspace services/drivers)
  - `usart/` — USART backend
  - `mctp_serial/` — MCTP over serial transport backend (Direct HAL or IPC)
- **mctp/** — Platform-specific MCTP server binary
- **tests/** — Integration tests

**Use:** Integrators can use these directly or as a basis for their own platform integrations.
