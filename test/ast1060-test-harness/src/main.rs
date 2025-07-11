#![no_std]
#![no_main]
use panic_halt as _;

#[unsafe(link_section = ".vector_table.reset_vector")]
#[unsafe(no_mangle)]
pub static __RESET_VECTOR: extern "C" fn() -> ! = reset_handler;

#[unsafe(no_mangle)]
extern "C" fn reset_handler() -> ! {
    unsafe extern "C" {
        static mut _sbss: u32;
        static mut _ebss: u32;
        static mut _sdata: u32;
        static mut _edata: u32;
        static _sidata: u32;
    }


    // Initialize (Zero) BSS
    unsafe {
        let mut sbss: *mut u32 = &raw mut _sbss;
        let ebss: *mut u32 = &raw mut _ebss;

        while sbss < ebss {
            core::ptr::write_volatile(sbss, 0);
            sbss = sbss.offset(1);
        }
    }

    // Initialize Data
    unsafe {
        let mut sdata: *mut u32 = &raw mut _sdata;
        let edata: *mut u32 = &raw mut _edata;
        let mut sidata: *const u32 = &_sidata;

        while sdata < edata {
            core::ptr::write_volatile(sdata, core::ptr::read_volatile(sidata));
            sdata = sdata.offset(1);
            sidata = sidata.offset(1);
        }
    }

    
    // Set VTOR to RAM

    main();
}




// Entry point for the program
#[unsafe(no_mangle)]
pub extern "C" fn main() -> ! {
    loop {}
}

