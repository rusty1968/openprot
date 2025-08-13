# Building ExHubris Applications

This document describes how to build applications using the ExHubris microkernel operating system.

## What is ExHubris?

ExHubris is an external distribution of the Hubris microkernel operating system, developed by Cliff Biffle to make Hubris accessible for applications outside the main Hubris repository. It provides:

- **Microkernel OS**: Robust, safety-focused operating system for microcontrollers
- **Task-based architecture**: Applications consist of isolated tasks communicating via messages
- **Hardware abstraction**: Support for STM32 microcontrollers (G0, L4, U5 series)
- **Custom build system**: Uses KDL configuration files and the `hubake` build tool

## Prerequisites

### Required Tools

1. **Rust toolchain** (specified in `rust-toolchain.toml`)
2. **ARM cross-compilation target**:
   ```bash
   rustup target add thumbv6m-none-eabi thumbv7em-none-eabi thumbv8m.main-none-eabi
   ```
3. **Build dependencies**:
   ```bash
   # Ubuntu/Debian
   sudo apt install build-essential pkg-config

   # Fedora/RHEL
   sudo dnf install gcc pkg-config
   ```

### Hardware Support

Currently supported boards:
- STM32G031K8 (Nucleo)  
- STM32L412KB
- STM32U575ZI (Nucleo)

## Installation

### 1. Install Hubake Build Tool

From the ExHubris repository root:

```bash
cargo install --path tools/hubake
```

This installs the `hubake` command globally, which serves as the main build interface.

### 2. Verify Installation

```bash
hubake --help
```

## Building Applications

### Application Structure

ExHubris applications are defined using **KDL configuration files** (`app.kdl`) that specify:

- **Target board**: Hardware platform (e.g., `nucleo-g031k8`)
- **Kernel configuration**: Memory layout, features, stack sizes
- **Task definitions**: Individual components with priorities and resources
- **Peripheral assignments**: Which tasks control which hardware
- **IPC relationships**: Inter-task communication setup

### Basic Build Process

1. **Navigate to application directory**:
   ```bash
   cd app/demo  # or any application directory
   ```

2. **Build the application**:
   ```bash
   hubake build app.kdl
   ```

3. **Build outputs**: Generated in `target/` directory with firmware images

### Example Application Build

Let's build the demo application:

```bash
# From ExHubris root
cd app/demo

# Build for the default board (STM32G031K8)
hubake build app.kdl

# The build will:
# 1. Parse app.kdl configuration
# 2. Build the kernel for the target board
# 3. Compile all tasks defined in the config
# 4. Link everything into a complete firmware image
```

### Build Configuration

The `app.kdl` file structure:

```kdl
// Application name
app demo

// Target hardware
board "proj:boards/nucleo-g031k8.kdl"

// Kernel configuration
kernel {
    workspace-crate kernel-generic-stm32g031
    stack-size 544
    features clock-64mhz-hsi16
}

// Task definitions
task super {
    workspace-crate minisuper
    stack-size 128
    priority 0
}

task sys {
    workspace-crate drv-stm32xx-sys
    stack-size 256
    priority 1
    uses-peripheral rcc
    uses-peripheral gpios
}
```

## Available Applications

### Demo Applications

1. **`app/demo/`**: Basic skeleton application
   - Minimal task setup
   - Good starting point for new projects

2. **`app/demo-nucleo-u575/`**: STM32U575 demonstration
   - More complex example
   - Shows advanced features

3. **`app/kbd/`**: Keyboard scanner
   - Real-world application example
   - Multiple cooperating tasks

### Building Specific Applications

```bash
# Build demo for STM32G031
cd app/demo
hubake build app.kdl

# Build keyboard scanner
cd app/kbd
hubake build app.kdl

# Build U575 demo
cd app/demo-nucleo-u575
hubake build app.kdl
```

## Build System Architecture

### Key Components

- **`hubake`**: Main build orchestrator
- **KDL files**: Declarative configuration format
- **Board definitions**: Hardware-specific configurations in `boards/`
- **Kernel variants**: Pre-configured kernels in `kernel-generic-*/`
- **Tasks**: Reusable components in `task/`
- **Drivers**: Hardware drivers in `drv/`

### Build Environment

The build system uses environment variables for configuration:

- **`HUBRIS_*` variables**: Build metadata (see `doc/build-env.adoc`)
- **`hubris-env.toml`**: Project-specific environment settings
- **Cargo features**: Selected based on board and application requirements

### Build Artifacts

Successful builds produce:
- **Firmware images**: Complete flashable binaries
- **Debug information**: For debugging and analysis
- **Memory maps**: Task and peripheral layouts
- **Metadata**: Build configuration and task information

## Troubleshooting

### Common Issues

1. **Missing target**: Install required Rust targets
   ```bash
   rustup target add thumbv6m-none-eabi
   ```

2. **Board not found**: Ensure board definition exists in `boards/`

3. **Task compilation errors**: Check task dependencies and workspace setup

4. **Peripheral conflicts**: Verify peripheral assignments in `app.kdl`

### Build Debugging

Enable verbose output:
```bash
hubake build app.kdl --verbose
```

Check build environment:
```bash
hubake env
```

## Advanced Usage

### Custom Applications

1. **Create application directory**:
   ```bash
   mkdir app/my-app
   cd app/my-app
   ```

2. **Create `app.kdl`** based on existing examples

3. **Define custom tasks** or reuse existing ones

4. **Build and test**:
   ```bash
   hubake build app.kdl
   ```

### Custom Tasks

Tasks are Rust crates in the workspace. Create new tasks by:

1. Adding crate to `task/` directory
2. Implementing required Hubris task interface
3. Referencing in application `app.kdl`

### Board Support

Add new boards by:

1. Creating board definition in `boards/`
2. Specifying CPU, memory layout, peripherals
3. Adding any required drivers

## Integration with OpenPRoT

ExHubris serves as a platform implementation for OpenPRoT, providing:

- **Secure execution environment**: Isolated tasks for crypto operations
- **Hardware abstraction**: Access to crypto accelerators
- **Message-based IPC**: Secure inter-component communication
- **Robust error handling**: Task isolation and recovery

To use OpenPRoT on ExHubris:

1. Build ExHubris application with OpenPRoT tasks
2. Configure crypto hardware access
3. Define IPC relationships for crypto services
4. Deploy complete firmware image

## References

- [ExHubris Repository](https://github.com/cbiffle/exhubris)
- [ExHubris Demo](https://github.com/cbiffle/exhubris-demo) 
- [Original Hubris](https://hubris.oxide.computer/)
- [KDL Configuration Language](https://kdl.dev/)

## Next Steps

1. **Try building** one of the demo applications
2. **Examine** the generated firmware images
3. **Create** a custom application for your hardware
4. **Integrate** OpenPRoT cryptographic services
5. **Deploy** to target hardware for testing
