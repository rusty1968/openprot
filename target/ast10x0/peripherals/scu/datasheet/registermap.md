| Offset | Size | Register Name | Access | Description |
| --- | --- | --- | --- | --- |
| 0x000 | 32b | Protection Key Register | RW | Unlock SCU registers (key: 0x1688A8A8) |
| 0x004 | 32b | Silicon Revision ID Register | RO | Hardware revision and chip bonding info |
| 0x010 | 32b | Protection Key Register 2 | RW | Additional protection key |
| 0x014 | 32b | Silicon Revision ID Register (dup) | RO | Same as 0x004 |
| 0x040 | 32b | System Reset Control Register | RW1S | Reset control for SRAM, peripherals |
| 0x044 | 32b | System Reset Control Clear Register | W1C | Clear bits in SCU040 |
| 0x050 | 32b | System Reset Control Register Set 2 | RW1S | Reset UART, JTAG, ADC, I3C controllers |
| 0x054 | 32b | System Reset Control Clear Register 2 | W1C | Clear bits in SCU050 |
| 0x060 | 32b | EXTRST# Reset Selection | RW | External reset selection control |
| 0x070 | 32b | EXTRST# Reset Selection 2 | RW | Additional external reset selection |
| 0x074 | 32b | Reset Event Log Register Set 2-1 | W1C | WDT and system reset event logs |
| 0x078 | 32b | Reset Event Log Register Set 2-2 | W1C | Peripheral reset event logs |
| 0x080 | 32b | Clock Stop Control Register | RW1S | Stop clocks for power saving |
| 0x084 | 32b | Clock Stop Control Clear | W1C | Clear clock stop control bits |
| 0x090 | 32b | Clock Stop Control Register Set 2 | RW1S | Stop I3C, RSA, REFCLK clocks |
| 0x094 | 32b | Clock Stop Control Clear Set 2 | W1C | Clear bits in SCU090 |
| 0x0D0 | 32b | Miscellaneous Control Register 3 | RW | JTAG routing, RTC clock divider |
| 0x0D4 | 32b | Miscellaneous Control Register 4 | RW | GPIO interrupt enable/select |
| 0x0D8 | 32b | Debug Control Register 2 | RW | Debug disable controls |
| 0x0F0 | 32b | QSPI Monitor Mux Control | RW | SPI PF reset and mux control |
| 0x200 | 32b | H-PLL Parameter Register | WO | PLL control and frequency setup |
| 0x204 | 32b | Extended H-PLL Parameter Register | WO | Additional PLL parameters |
| 0x310 | 32b | Clock Selection Register Set 4 | RW | I3C, APB, UART clock selection |
| 0x314 | 32b | Clock Selection Register Set 5 | RW | HCLK and I3CHCLK selection |
| 0x330 | 32b | Frequency Counter Control Register | RW | Frequency measurement control |
| 0x334 | 32b | Frequency Counter Comparison Range | RW | Frequency compare limits |
| 0x410 | 32b | Multi-function Pin Control #1 | RW | Reserved |
| 0x414 | 32b | Multi-function Pin Control #2 | RW | I2C/I3C SDA/SCL enable |
| 0x418 | 32b | Multi-function Pin Control #3 | RW | I3C SDA/SCL LV enable |
| 0x430 | 32b | Multi-function Pin Control #5 | RW | SPI, GPIO pin selection |
| 0x434 | 32b | Multi-function Pin Control #6 | RW | GPIO pin selection |
| 0x438 | 32b | Multi-function Pin Control #7 | RW | GPIO passthrough debounce enable |
| 0x450 | 32b | Multi-function Pin Control #9 | RW | I2C pin enable |
| 0x454 | 32b | Multi-function Pin Control #10 | RW | JTAG master routing |
| 0x458 | 32b | Multi-function Pin Control #11 | RW | GPIO voltage select |
| 0x4B0 | 32b | Multi-function Pin Control #13 | RW | GPIO passthrough, SALT pins |
| 0x4B8 | 32b | Multi-function Pin Control #15 | RW | I2C/I3C pin enable |
| 0x4BC | 32b | Multi-function Pin Control #16 | RW | JTAG Master #1 pins |
| 0x500 | 32b | Hardware Strap Register 1 | RW | Power-on reset configuration |
| 0x504 | 32b | Hardware Strap1 Clear Register | W1C | Clear strap bits |
| 0x508 | 32b | Hardware Strap1 Protection Register | RW1S | Write protection for strap registers |
| 0x510 | 32b | Hardware Strap Register 2 | RW | Power-on reset configuration |
| 0x514 | 32b | Hardware Strap2 Clear Register | W1C | Clear strap bits |
| 0x518 | 32b | Hardware Strap2 Protection Register | RW1S | Write protection for strap registers |
| 0x51C | 32b | Hardware Pin Strap Register | RW | Secure boot and mirror bits |
| 0x530 | 32b | RNG2 Control Register | RW | Random number generator control |
| 0x534 | 32b | RNG2 Data Register | RO | Random number output |
| 0x600 | 32b | eFuse Configuration Register |  |  |