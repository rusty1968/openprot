# AST10x0 Peripherals

This crate provides high-level abstractions for AST10x0 peripherals including UART and SMC (System Management Controller) interfaces.

## Building

This crate requires the AST10x0 target platform to be specified during builds. The platform provides the necessary `target_ast10x0` constraint value.

### Build Command

```bash
bazelisk build --platforms=//target/ast10x0 //target/ast10x0/peripherals:peripherals
```

### Alternative: Using Configuration

If you're running tests or other commands in the AST10x0 context, use the predefined `virt_ast10x0` config:

```bash
bazelisk build --config=virt_ast10x0 //target/ast10x0/peripherals:peripherals
```

## Modules

- **uart**: UART peripheral driver and configuration
- **smc**: System Management Controller with register definitions, types, controller logic, and interrupt handling

## Dependencies

- `ast1060_pac`: AST1060 Peripheral Access Crate
- `bitflags`: Bit manipulation utilities
- `embedded-hal`: Hardware abstraction layer traits
- `embedded-hal-nb`: Non-blocking HAL traits
- `embedded-io`: I/O abstractions for embedded systems
- `nb`: Non-blocking trait definitions
