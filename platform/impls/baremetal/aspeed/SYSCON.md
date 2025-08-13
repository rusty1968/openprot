# System Controller Driver for AST1060

This module provides comprehensive clock and reset management for the Aspeed AST1060 system-on-chip through the System Control Unit (SCU).

## Overview

The AST1060 System Control Unit manages:
- **Clock Control**: Enable/disable peripheral clocks
- **Reset Control**: Assert/deassert peripheral resets  
- **Frequency Management**: Configure clock frequencies for supported peripherals
- **Power Management**: Control peripheral power states through clocks

## Key Features

### Clock Management
- Enable/disable clocks for all major peripherals
- Support for multiple clock groups (Group 0, Group 1)
- Frequency configuration for I3C and HCLK
- Clock source selection

### Reset Management  
- Assert/deassert reset for individual peripherals
- Reset pulse functionality with configurable duration
- Reset state checking
- Support for multiple reset groups

### Hardware Integration
- Direct register access through ast1060-pac
- Memory-mapped SCU registers
- Hardware abstraction for system initialization

## Supported Peripherals

### Clocks
- `ClkMCLK` - Main system clock
- `ClkYCLK` - HACE/crypto engine clock  
- `ClkRSACLK` - RSA accelerator clock
- `ClkI3C0-3` - I3C controller clocks
- `ClkHCLK` - High-speed clock
- `ClkPCLK` - Peripheral clock
- `ClkREFCLK` - Reference clock

### Resets
- `RstHACE` - Hash and Crypto Engine
- `RstSRAM` - System RAM
- `RstUART1-4` - UART controllers
- `RstI3C0-3` - I3C controllers
- `RstI2C` - I2C subsystem
- `RstADC` - Analog-to-Digital Converter
- `RstJTAGM0/1` - JTAG masters

## Usage Examples

### Basic Initialization

```rust
use openprot_platform_aspeed::syscon::{SysCon, ClockId, ResetId};
use ast1060_pac::Peripherals;

let peripherals = unsafe { Peripherals::steal() };
let mut syscon = SysCon::new(peripherals.scu);

// Enable HACE for cryptographic operations
syscon.enable_clock(ClockId::ClkYCLK)?;
syscon.reset_deassert(ResetId::RstHACE)?;

// Enable RSA/ECC accelerator
syscon.enable_clock(ClockId::ClkRSACLK)?;
```

### Integrated HACE Initialization

```rust
use openprot_platform_aspeed::{SysCon, HaceController};
use ast1060_pac::Peripherals;

let peripherals = unsafe { Peripherals::steal() };

// Initialize system controller and HACE together
let mut syscon = SysCon::new(peripherals.scu);
let hace_controller = 
    HaceController::new_with_syscon(peripherals.hace, &mut syscon)?;

// Now ready for cryptographic operations
```

### Convenience Methods

```rust
let mut syscon = SysCon::new(peripherals.scu);

// Initialize HACE (clock + reset)
syscon.init_hace()?;

// Initialize RSA/ECC
syscon.init_rsa_ecc()?;
```

### Frequency Management

```rust
// Set I3C0 clock to 12.5 MHz
syscon.set_frequency(ClockId::ClkI3C0, 12_500_000)?;

// Set HCLK to 200 MHz  
syscon.set_frequency(ClockId::ClkHCLK, 200_000_000)?;

// Check current frequencies
let i3c_freq = syscon.get_frequency(ClockId::ClkI3C0)?;
let hclk_freq = syscon.get_frequency(ClockId::ClkHCLK)?;
```

### Reset Operations

```rust
// Check reset state
let is_reset = syscon.reset_is_asserted(ResetId::RstHACE)?;

// Manual reset sequence
syscon.reset_assert(ResetId::RstUART1)?;
// ... delay ...
syscon.reset_deassert(ResetId::RstUART1)?;

// Or use pulse for convenience
use core::time::Duration;
syscon.reset_pulse(ResetId::RstUART2, Duration::from_millis(1))?;
```

## Register Mapping

### Clock Control Registers
- **SCU080**: Clock Stop Control Clear (Group 0)
- **SCU084**: Clock Stop Control Clear Status (Group 0) 
- **SCU090**: Clock Stop Control Set (Group 1)
- **SCU094**: Clock Stop Control Set Status (Group 1)

### Reset Control Registers  
- **SCU040**: Reset Control Clear (Group 0)
- **SCU044**: Reset Control Set (Group 0)
- **SCU050**: Reset Control Clear (Group 1) 
- **SCU054**: Reset Control Set (Group 1)

### Frequency Control Registers
- **SCU310**: I3C Clock Control
- **SCU314**: HCLK Control

## Clock Groups

### Group 0 (Bits 0-31)
- Bit 0: `ClkMCLK` - Main clock
- Bit 13: `ClkYCLK` - HACE clock
- Other system clocks

### Group 1 (Bits 32-63)  
- Bit 2: `ClkREFCLK` - Reference clock
- Bit 6: `ClkRSACLK` - RSA clock
- Bits 8-11: `ClkI3C0-3` - I3C clocks

## Reset Groups

### Group 0 (Bits 0-31)
- Bit 0: `RstSRAM` - SRAM reset
- Bit 4: `RstHACE` - HACE reset

### Group 1 (Bits 32-63)
- Bit 2: `RstI2C` - I2C reset  
- Bits 7-11: `RstI3C*` - I3C resets
- Bits 22-31: Various peripheral resets

## Error Handling

The system controller provides comprehensive error handling:

```rust
use openprot_platform_aspeed::syscon::Error;

match syscon.enable_clock(ClockId::ClkYCLK) {
    Ok(()) => {
        // Clock enabled successfully
    },
    Err(Error::ClockAlreadyEnabled) => {
        // Clock was already enabled - not necessarily an error
    },
    Err(Error::ClockNotFound) => {
        // Invalid clock ID
    },
    Err(e) => {
        // Other errors
        panic!("Clock enable failed: {:?}", e);
    }
}
```

## Hardware Considerations

### Clock Dependencies
- Some peripherals require specific clock sequences
- HACE requires YCLK before reset deassertion
- I3C controllers need individual clock enables

### Reset Timing
- Hardware resets require proper timing
- Use `reset_pulse()` for automatic timing
- Consider peripheral-specific reset requirements

### Power Management
- Disabling clocks saves power
- Some clocks cannot be disabled during operation
- Check hardware reference for restrictions

## Integration with HACE

The system controller integrates seamlessly with the HACE controller:

```rust
// Method 1: Integrated initialization  
let mut syscon = SysCon::new(peripherals.scu);
let hace_controller = HaceController::new_with_syscon(peripherals.hace, &mut syscon)?;

// Method 2: Manual control
let mut syscon = SysCon::new(peripherals.scu);
syscon.init_hace()?;  // Convenience method
let hace_controller = HaceController::new(peripherals.hace);
```

## Future Enhancements

- Power domain management
- Clock gating controls  
- Advanced frequency synthesis
- Thermal management integration
- Dynamic voltage and frequency scaling

## References

- AST1060 Hardware Reference Manual
- Aspeed SCU Register Specifications
- OpenPRoT HAL Documentation
