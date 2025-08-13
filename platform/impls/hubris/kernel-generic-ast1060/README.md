# generic ast1060 kernel

This is a generic Hubris kernel wrapper / startup routine that fires up the
kernel on an AST1060 SoC (Aspeed Root of Trust microcontroller). 

The AST1060 is an ARM Cortex-M4F based security processor designed for 
Root of Trust applications, featuring:

- ARM Cortex-M4F core
- Hardware cryptographic accelerator (HACE)
- RSA/ECC acceleration
- Secure boot capabilities
- Multiple communication interfaces

## Features

- `clock-default`: Use default AST1060 clock configuration
- `kernel-profiling`: Enable specialized kernel profiling via GPIOs

## Hardware Support

This kernel supports the AST1060 SoC with initialization including:

- Clock configuration
- GPIO setup
- **Cryptographic peripheral enablement**: Automatically enables HACE and RSA/ECC accelerators
- Exception handling

## Cryptographic Support

The kernel automatically initializes cryptographic peripherals on startup:

- **HACE (Hash and Crypto Engine)**: Enabled via YCLK clock and reset deassertion
- **RSA/ECC Accelerator**: Enabled via RSACLK clock for public key operations
- Error handling for already-initialized peripherals

This ensures that Hubris applications can use hardware cryptographic acceleration
without additional setup.

## Usage

This kernel is designed to be used as part of a larger Hubris application
that targets AST1060-based hardware platforms. The crypto peripherals will
be ready for use by Hubris tasks immediately after kernel startup.
