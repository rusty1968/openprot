use std::env;

fn main() {
    // Get the target triple
    let target = env::var("TARGET").unwrap();
    
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=TARGET");
    
    // RISC-V specific configuration
    if target.starts_with("riscv32") {
        println!("cargo:rustc-cfg=riscv");
        println!("cargo:rustc-cfg=riscv32");
        
        // OpenTitan-specific configuration
        if target == "riscv32imc-unknown-none-elf" {
            println!("cargo:rustc-cfg=opentitan_target");
            
            // Memory layout from OpenTitan baremetal/layout.ld
            println!("cargo:rustc-env=OPENTITAN_RAM_BASE=0x10000000");
            println!("cargo:rustc-env=OPENTITAN_RAM_SIZE=0x20000");      // 128KB
            println!("cargo:rustc-env=OPENTITAN_FLASH_BASE=0x20010000");
            println!("cargo:rustc-env=OPENTITAN_FLASH_SIZE=0x70000");    // 448KB
            
            // Hardware register base addresses from OpenTitan registers/*.rs
            println!("cargo:rustc-env=OPENTITAN_HMAC_BASE=0x41110000");
            println!("cargo:rustc-env=OPENTITAN_KMAC_BASE=0x41120000");
            println!("cargo:rustc-env=OPENTITAN_AES_BASE=0x41100000");
            println!("cargo:rustc-env=OPENTITAN_OTBN_BASE=0x41130000");
            println!("cargo:rustc-env=OPENTITAN_KEYMGR_BASE=0x41140000");
            println!("cargo:rustc-env=OPENTITAN_CSRNG_BASE=0x41150000");
            println!("cargo:rustc-env=OPENTITAN_EDN0_BASE=0x41170000");
            println!("cargo:rustc-env=OPENTITAN_EDN1_BASE=0x41180000");
            
            // Additional peripheral base addresses
            println!("cargo:rustc-env=OPENTITAN_GPIO_BASE=0x40040000");
            println!("cargo:rustc-env=OPENTITAN_UART0_BASE=0x40000000");
            println!("cargo:rustc-env=OPENTITAN_SPI_DEVICE_BASE=0x40050000");
            println!("cargo:rustc-env=OPENTITAN_I2C0_BASE=0x40080000");
            println!("cargo:rustc-env=OPENTITAN_TIMER_BASE=0x40100000");
            
            // CPU frequency (typical for OpenTitan Earlgrey)
            println!("cargo:rustc-env=OPENTITAN_CPU_FREQ_HZ=100000000"); // 100MHz
            
            // OpenTitan chip version
            println!("cargo:rustc-env=OPENTITAN_CHIP=earlgrey");
        }
    }
    
    // Bare metal configuration
    if target.ends_with("-none-elf") {
        println!("cargo:rustc-cfg=baremetal");
        
        // Link arguments for bare metal
        if cfg!(feature = "opentitan") {
            // These would point to OpenTitan-specific linker scripts
            // In a real implementation, these might be provided by the OpenTitan SDK
            println!("cargo:rustc-link-arg=-Tlink.x");
        }
    }
    
    // Development/testing support on hosted platforms
    if !target.ends_with("-none-elf") {
        println!("cargo:rustc-cfg=hosted");
        
        // Enable additional debugging/testing features for hosted builds
        if cfg!(debug_assertions) {
            println!("cargo:rustc-cfg=debug_simulation");
        }
    }
    
    // Feature-specific configuration
    if cfg!(feature = "opentitan") {
        println!("cargo:rustc-cfg=platform_opentitan");
        
        // Version information that might be useful for compatibility
        println!("cargo:rustc-env=OPENTITAN_VERSION=earlgrey");
        println!("cargo:rustc-env=OPENTITAN_REVISION=latest");
    }
}
