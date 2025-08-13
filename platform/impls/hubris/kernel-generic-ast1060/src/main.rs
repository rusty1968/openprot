#![no_std]
#![no_main]

use ast1060_pac as pac;
use cortex_m_rt::entry;

fn clock_setup() -> u32 {
    // AST1060 comes up with a default clock configuration
    // The ARM Cortex-M4F typically runs at a reasonable frequency out of reset
    // For now, we'll use the default configuration
    // TODO: Add clock configuration features as needed
    
    // Return the system clock frequency in Hz
    // AST1060 default frequency - this may need adjustment based on actual hardware
    100_000_000 // 100 MHz placeholder
}

fn gpio_setup() {
    // AST1060-specific GPIO initialization
    // Configure JTAG pins for debugging
    let peripherals = unsafe { pac::Peripherals::steal() };
    peripherals.scu.scu41c().modify(|_, w| {
        // Set the JTAG pinmux to enable ARM JTAG functions
        w.enbl_armtmsfn_pin().bit(true)
            .enbl_armtckfn_pin().bit(true)
            .enbl_armtrstfn_pin().bit(true)
            .enbl_armtdifn_pin().bit(true)
            .enbl_armtdofn_pin().bit(true)
    });
    
    // TODO: Add other GPIO configuration for kernel profiling if needed
}

fn peripheral_setup() {
    // AST1060-specific peripheral initialization
    // Enable cryptographic peripherals for system use
    let peripherals = unsafe { pac::Peripherals::steal() };
    
    // Enable HACE (Hash and Crypto Engine) - Clock bit 13
    // Clear clock stop control to enable clock
    if peripherals.scu.scu080().read().bits() & (1 << 13) == (1 << 13) {
        peripherals.scu.scu084().write(|w| unsafe { w.bits(1 << 13) });
    }
    
    // Deassert HACE reset - Reset bit 13  
    peripherals.scu.scu044().write(|w| unsafe { w.bits(1 << 13) });
    
    // Enable RSA accelerator - Clock bit 24
    if peripherals.scu.scu080().read().bits() & (1 << 24) == (1 << 24) {
        peripherals.scu.scu084().write(|w| unsafe { w.bits(1 << 24) });
    }
    
    // Deassert RSA reset - Reset bit 24
    peripherals.scu.scu044().write(|w| unsafe { w.bits(1 << 24) });
}

#[entry]
fn main() -> ! {
    // Perform AST1060-specific hardware initialization
    let _cpu_hz = clock_setup();
    gpio_setup();
    peripheral_setup();

    cfg_if::cfg_if! {
        if #[cfg(feature = "kernel-profiling")] {
            // Initialize kernel profiling GPIOs if enabled
            // TODO: Implement AST1060-specific GPIO profiling
        }
    }

    // Simple demonstration loop - in a real ExHubris setup,
    // this would hand control to the ExHubris kernel
    loop {
        cortex_m::asm::wfi(); // Wait for interrupt
    }
    
    // TODO: When ExHubris is properly set up, replace the loop above with:
    // hubris_kern::startup::main(cpu_hz)
}

// AST1060-specific panic handler
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // In a real implementation, this might:
    // - Log panic info to UART
    // - Set a GPIO to indicate panic state
    // - Reset the system
    
    loop {
        cortex_m::asm::wfi();
    }
}

// Handle hard faults
#[cortex_m_rt::exception]
unsafe fn HardFault(_ef: &cortex_m_rt::ExceptionFrame) -> ! {
    // AST1060-specific hard fault handling
    loop {
        cortex_m::asm::wfi();
    }
}

// Handle default exceptions
#[cortex_m_rt::exception]
unsafe fn DefaultHandler(_irqn: i16) {
    // AST1060-specific default interrupt handling
    // This is called for unhandled interrupts
}
