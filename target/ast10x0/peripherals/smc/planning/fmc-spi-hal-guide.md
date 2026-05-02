# AST1030 FMC and SPI Memory Controllers: HAL Implementation Guide

## Executive Summary

The ASPEED AST1030 MiniBMC SoC includes **three independent SPI Flash Memory Controllers** (1 FMC + 2 SPI) to address the market segment's critical need for redundancy, boot flexibility, and secure firmware management in embedded systems management processors. This document provides the complete architectural, register-level, and operational information required for bare-metal HAL development.

---

## 1. Market Context and Design Rationale

### 1.1 Target Market Segment

The AST1030 serves the **MiniBMC and Platform Root of Trust (PRoT)** processor market:
- **Embedded systems management** (server BMCs, IoT gateways, PLC controllers)
- **Security-critical applications** requiring secure boot and firmware verification
- **Distributed systems** where multiple processors need independent firmware images

### 1.2 Why Both FMC and SPI?

Three independent flash controllers provide:

| Feature | Use Case |
|---------|----------|
| **Redundancy** | Boot from primary or alternate firmware path without S/W fallback logic |
| **Separation of Concerns** | FMC (Firmware Code) vs. SPI1/SPI2 (Configuration, Certificates, Logs) |
| **Performance Flexibility** | Different flash devices have different timing requirements; independent controllers allow optimal tuning |
| **Firmware Update Strategy** | Update non-active flash while running from active; atomic switchover via hardware mux |
| **Secure Boot Topology** | Root-of-trust (ROM) on FMC, secondary bootloader on SPI1, OS on SPI2 |
| **Capacity Scalability** | Up to 768 MB total addressable flash (256 MB per controller) |

### 1.3 AST1030 Market Positioning

- **CPU**: ARM Cortex-M4F @ 200 MHz (real-time capable)
- **RAM**: SRAM only (no DDR) — optimized for embedded, not general-purpose computing
- **I/O**: Minimal peripheral set (UART, I2C, GPIO, Timer, WDT, ADC)
- **Security**: Secure Boot Controller (SBC), Hash/Crypto Engine (HACE), on-die key storage
- **Typical use**: Secure BMC, firmware root-of-trust, edge security processor

This class of SoC prioritizes **firmware integrity and secure updates** over raw computational power, hence the emphasis on multiple independent flash control paths.

---

## 2. Hardware Architecture

### 2.1 High-Level Block Diagram

```
        CPU (Cortex-M4F)
           |
      +----+----------+
      |               |
    VIC            Memory Bus
  (IRQ)               |
      |       +-------+-------+
      |       |               |
    [IRQ39]  FMC          +---+---+
             (SMC)        |       |
           [Flash W]  SPI1    SPI2
         Window @     (SMC)   (SMC)
        0x80000000
                     SPI Bus (SSI)
                         |
                    Flash Devices
                    (Concurrent or Muxed)
```

### 2.2 Memory Map

All controllers reside in the **I/O memory region** at `0x7E600000`. Register access uses 32-bit aligned addressing.

| Device | Controller Base | Flash Window Base | Flash Window Size | IRQ | Purpose |
|--------|-----------------|-------------------|-------------------|-----|---------|
| **FMC** | `0x7E620000` | `0x80000000` | 256 MB | 39 | Primary firmware; supports WDT-triggered boot mode switch |
| **SPI1** | `0x7E630000` | `0x90000000` | 256 MB | 65 | Secondary firmware or configuration |
| **SPI2** | `0x7E640000` | `0xB0000000` | 256 MB | 66 | Tertiary firmware, OS, or telemetry logs |

### 2.3 Chip Select (CS) Configuration

Each controller supports up to **2 Chip Select (CS) lines**, allowing **two flash devices per controller** (up to 6 total flash devices on AST1030).

| Controller | CS0 | CS1 | Note |
|------------|-----|-----|------|
| FMC | Primary boot image | Alternate image (via WDT) | CS bit fields control row/col address mapping for 2-chip mode |
| SPI1 | Primary app/cfg | Spare for future use | Independent CS timing controlled per device |
| SPI2 | Log storage | Future expansion | Typically run-time writable via secure update protocol |

---

## 3. Register Map and Control Structures

### 3.1 Register Organization

Each SMC controller instance (FMC, SPI1, SPI2) implements a **20-register space** at its base address:

```c
// Offset register layout (in bytes, 32-bit access)
0x00  R_CONF              - Configuration register (CE Type, Flash Type)
0x04  R_CE_CTRL           - Chip Enable Control (CS0-CS4 32-bit mode select)
0x08  R_INTR_CTRL         - Interrupt Control and Status
0x0C  R_CE_CMD_CTRL       - Command Control (address/data byte enable)
0x10  R_TIMINGS           - Read/Write timing register (CLK, HOLD, SETUP delays)
0x14  R_TIMINGS2          - Extended timing parameters (WP, RST timing)
0x30  R_DMA_CTRL          - DMA control and status (FMC & SPI only)
0x34  R_DMA_SPI_LEN       - DMA SPI length
0x38  R_DMA_DRAM_ADDR     - DMA DRAM source/dest address
0x3C  R_DMA_STATUS        - DMA completion status
... (additional segment control registers at 0x50–0x68)
```

**AST1030-specific register layout:**
- FMC implements full register set + WDT-based boot control (R_FMC_WDT2_CTRL @ 0x64)
- SPI1/SPI2 implement lean register set (no WDT control, shared DRAM mask)

### 3.2 Key Registers for HAL Implementation

#### 3.2.1 Configuration (R_CONF @ +0x00)

Controls flash device type per CS line:

```c
#define R_CONF          0x00

// Bit fields for FMC AST1030:
#define CONF_LEGACY_DISABLE  (1 << 31)   // Disable legacy mode
#define CONF_ENABLE_W4       (1 << 20)   // CS4 write-enable
#define CONF_ENABLE_W3       (1 << 19)   // CS3 write-enable
#define CONF_ENABLE_W2       (1 << 18)   // CS2 write-enable
#define CONF_ENABLE_W1       (1 << 17)   // CS1 write-enable
#define CONF_ENABLE_W0       (1 << 16)   // CS0 write-enable
#define CONF_FLASH_TYPE4     (3 << 8)    // CS4 flash type (2 bits)
#define CONF_FLASH_TYPE3     (3 << 6)    // CS3 flash type
#define CONF_FLASH_TYPE2     (3 << 4)    // CS2 flash type
#define CONF_FLASH_TYPE1     (3 << 2)    // CS1 flash type
#define CONF_FLASH_TYPE0     (3 << 0)    // CS0 flash type (2 bits)

// Flash type encoding:
#define FLASH_TYPE_NOR       0x0         // Parallel NOR (legacy, not used in AST1030)
#define FLASH_TYPE_NAND      0x1         // NAND Flash (not supported on AST1030 SPI path)
#define FLASH_TYPE_SPI       0x2         // SPI NOR Flash (typical for AST1030)
```

**HAL Usage:**
```c
// Enable CS0 and CS1 writes, set both to SPI NOR type
uint32_t conf = ((CONF_ENABLE_W1 | CONF_ENABLE_W0) |
                 (FLASH_TYPE_SPI << CONF_FLASH_TYPE0) |
                 (FLASH_TYPE_SPI << CONF_FLASH_TYPE1));
aspeed_smc_write(base, R_CONF, conf);
```

#### 3.2.2 Chip Enable Control (R_CE_CTRL @ +0x04)

Enables 32-bit addressing for high-capacity flash or 2-chip mode addressing:

```c
#define R_CE_CTRL            0x04

// Bit fields:
#define CTRL_EXTENDED4       (1 << 4)    // Enable 32-bit addressing for CS4
#define CTRL_EXTENDED3       (1 << 3)    // Enable 32-bit addressing for CS3
#define CTRL_EXTENDED2       (1 << 2)    // Enable 32-bit addressing for CS2
#define CTRL_EXTENDED1       (1 << 1)    // Enable 32-bit addressing for CS1
#define CTRL_EXTENDED0       (1 << 0)    // Enable 32-bit addressing for CS0
```

**HAL Usage:**
```c
// Enable 32-bit addressing for CS0 (supports >128MB flash devices)
aspeed_smc_write(base, R_CE_CTRL, CTRL_EXTENDED0);
```

#### 3.2.3 Interrupt Control (R_INTR_CTRL @ +0x08)

Manages DMA completion, command abort, and write-protect interrupts:

```c
#define R_INTR_CTRL       0x08

#define INTR_CTRL_DMA_STATUS            (1 << 11)   // DMA complete (RO)
#define INTR_CTRL_CMD_ABORT_STATUS      (1 << 10)   // Command abort flag (RO)
#define INTR_CTRL_WRITE_PROTECT_STATUS  (1 << 9)    // Write protect flag (RO)
#define INTR_CTRL_DMA_EN                (1 << 3)    // Enable DMA interrupt
#define INTR_CTRL_CMD_ABORT_EN          (1 << 2)    // Enable command abort interrupt
#define INTR_CTRL_WRITE_PROTECT_EN      (1 << 1)    // Enable write-protect interrupt
```

**HAL Usage (Poll-based):**
```c
// Poll DMA status until complete
while (!(aspeed_smc_read(base, R_INTR_CTRL) & INTR_CTRL_DMA_STATUS)) {
    // Busy-wait or yield
}
// Clear status by reading (or write 1 to clear per datasheet)
```

#### 3.2.4 DMA Control (R_DMA_CTRL @ +0x30) — FMC & SPI Only

Initiates high-speed flash-to-DRAM or DRAM-to-flash transfers:

```c
#define R_DMA_CTRL          0x30

#define DMA_CTRL_REQUEST    (1 << 0)    // Write 1 to start DMA
#define DMA_CTRL_CMD_ABORT  (1 << 1)    // Write 1 to abort DMA
#define DMA_CTRL_SPI_LEN    (???)       // SPI transfer length (lower bits)
```

**DMA masks for AST1030:**
```c
#define FMC_DMA_FLASH_MASK  0x0FFFFFFC  // Valid flash address bits
#define FMC_DMA_DRAM_MASK   0x000BFFFC  // Valid DRAM address bits (limited to SRAM window)
```

**HAL Usage:**
```c
// Initiate DMA: read 1KB from flash offset 0x100000 to SRAM at 0x04000000
aspeed_smc_write(base, R_DMA_DRAM_ADDR, 0x04000000);        // DRAM address
aspeed_smc_write(base, R_DMA_SPI_LEN, 1024);               // Length
aspeed_smc_write(base, 0x50 + cs*4, 0x00100000);           // Flash segment start (per CS)
aspeed_smc_write(base, R_DMA_CTRL, DMA_CTRL_REQUEST);      // Trigger DMA

// Wait for completion
while (!(aspeed_smc_read(base, R_INTR_CTRL) & INTR_CTRL_DMA_STATUS)) {
    // Poll
}
```

#### 3.2.5 Timing Registers (R_TIMINGS @ +0x10)

Must be programmed to match specific flash device speed and SPI bus clock:

```c
#define R_TIMINGS          0x10

// Bit field structure (varies slightly across SoCs):
// [31:28] = SETUP delay (clock cycles before CS assert)
// [27:24] = HOLD delay (clock cycles after CS deassert)
// [23:20] = CLK divisor (SPI clock = SYSCLK / 2^clk_div)
// [19:16] = Read command opcode (typically 0x3 for standard SPI read)
// etc.
```

**AST1030 Specific:** FMC and SPI1/SPI2 support timing registers at `R_TIMINGS` and `R_TIMINGS2`.

**HAL Usage (Conservative Settings):**
```c
// Assume worst-case slow flash, safe timing:
// CLK divisor = 7 (SYSCLK/128 = 200MHz/128 ≈ 1.56 MHz)
// SETUP = 2 cycles, HOLD = 2 cycles
uint32_t timings = (2 << 28) | (2 << 24) | (7 << 20);
aspeed_smc_write(base, R_TIMINGS, timings);

// For high-speed SPI reads (fast flash):
// CLK divisor = 3 (SYSCLK/8 = 25 MHz)
// SETUP = 1, HOLD = 1
timings = (1 << 28) | (1 << 24) | (3 << 20);
```

### 3.3 Segment Control Registers (Chip Select Address Mapping)

Each CS line maps a **256 MB window** into the flash address space. Typically:
- CS0 starts at flash_window_base (0x80000000 for FMC, 0x90000000 for SPI1, etc.)
- CS1 starts at flash_window_base + (device0_size)

```c
// Segment registers map physical addresses to SPI Flash offsets
// Offset 0x50 + (CS * 4): CS Segment Register
// Bits [31:16] = end address (4 KB units)
// Bits [15:0]  = start address (4 KB units)

#define SEG_START_ADDR(n)  ((n) & 0x0FFF)     // Lower 12 bits for start
#define SEG_END_ADDR(n)    (((n) >> 16) & 0x0FFF)

// Example: 16 MB device at address 0x00000000
uint32_t seg = (SEG_END_ADDR(16*1024*1024) << 16) | SEG_START_ADDR(0);
aspeed_smc_write(base, 0x50, seg);  // CS0 segment
```

---

## 4. Memory Map Summary for HAL Initialization

### SRAM and Flash Windows (from CPU perspective)

```
+------------------+
| 0x00000000       |  SRAM (32 KB default, up to 64 KB configurable)
|                  |  -> HAL stack, globals, runtime buffers
| 0x00008000       |
+------------------+

+------------------+
| 0x79000000       |  Secure SRAM (36 KB)
|                  |  -> Encryption keys, attestation data
| 0x79009000       |
+------------------+

+------------------+
| 0x7E600000       |  System I/O Region (2 MB)
| 0x7E620000       |    FMC Controller (@+0x000)
| 0x7E630000       |    SPI1 Controller (@+0x10000)
| 0x7E640000       |    SPI2 Controller (@+0x20000)
|                  |    + GPIO, Timer, UART, I2C, ADC, etc.
| 0x7E7FFFFF       |
+------------------+

+------------------+
| 0x80000000       |  FMC Flash Window (256 MB)
|                  |  -> Maps FMC flash devices (CS0, CS1)
| 0x8FFFFFFF       |
+------------------+

+------------------+
| 0x90000000       |  SPI1 Flash Window (256 MB)
|                  |  -> Maps SPI1 flash devices (CS0, CS1)
| 0x9FFFFFFF       |
+------------------+

+------------------+
| 0xB0000000       |  SPI2 Flash Window (256 MB)
|                  |  -> Maps SPI2 flash devices (CS0, CS1)
| 0xBFFFFFFF       |
+------------------+

Other reserved regions:
  0x7E600000 - 0x7EFFFFFF: I/O mapped peripherals (SCU, UART, I2C, etc.)
```

---

## 5. Interrupt Resources

### 5.1 IRQ Mapping (ARM NVIC, 0-based exception IDs)

| Peripheral | IRQ | Entry in NVIC | Typical Priority | Notes |
|------------|-----|---------------|------------------|-------|
| FMC | 39 | 39 | High (DMA completion) | Firmware access; WDT boot-mode switch also here |
| SPI1 | 65 | 65 | Medium (App config DMA) | Application/configuration updates |
| SPI2 | 66 | 66 | Low (Logging) | Best-effort telemetry writes |
| Timer1–8 | 16–23 | 16–23 | Variable | Used for delays, timeouts, scheduling |
| Watchdog | 24 | 24 | Critical | Must service or system resets on timeout (MSB-first: WDT1 at bottom) |
| UART1–13 | 47–64 | 47–64 | Low (debug/console) | Serial debug, management console |

**HAL Usage:**
```c
// Enable FMC DMA interrupt
void fmc_dma_isr(void) {
    // DMA transfer complete; safe to read data or start another DMA
    uint32_t status = aspeed_smc_read(FMC_BASE, R_INTR_CTRL);
    // Clear status (implementation-dependent; may be auto-clear on read)
}

// In HAL init:
nvic_enable_irq(39);  // FMC
nvic_set_priority(39, 2);  // High priority
```

---

## 6. Theory of Operation

### 6.1 Firmware Boot Sequence (Pre-HAL, Hardware)

1. **Power-on reset (POR)** → ARM Cortex-M4F wakes at 0x00000000
2. **ROM Boot Code** (internal, non-modifiable):
   - Initializes clock tree (SYSCLK sourcing)
   - Sets up **FMC controller** for initial read from boot flash (default: CS0)
   - Jumps to application code in FMC flash window (0x80000000)

3. **Primary Bootloader** (in FMC CS0):
   - Verifies FMC image signature using HACE
   - Reads SPI1 header and validates
   - Decides: boot from SPI1 or skip to ROM-loaded app?
   - Can switch boot sources on watchdog reset via FMC WDT2 control

### 6.2 HAL Initialization Phase

```c
void hal_smc_init(void) {
    // 1. Clock setup (assume SCU already configured SYSCLK @ 200 MHz)
    
    // 2. Configure FMC for typical 2-flash mode:
    //    CS0: 64 MB boot image
    //    CS1: 64 MB alternate image
    
    uint32_t conf = (CONF_ENABLE_W0 | CONF_ENABLE_W1) |
                    (FLASH_TYPE_SPI << CONF_FLASH_TYPE0) |
                    (FLASH_TYPE_SPI << CONF_FLASH_TYPE1);
    aspeed_smc_write(FMC_BASE, R_CONF, conf);
    
    // 3. Set timing for 25 MHz SPI bus (typical w25q flash)
    uint32_t timings = (1 << 28) | (1 << 24) | (3 << 20);
    aspeed_smc_write(FMC_BASE, R_TIMINGS, timings);
    
    // 4. Map CS0 to 0x00000000 (first 128 MB)
    //    and CS1 to 0x08000000 (second 128 MB)
    aspeed_smc_write(FMC_BASE, 0x50, 
        (SEG_END_ADDR(128*MiB) << 16) | SEG_START_ADDR(0));
    aspeed_smc_write(FMC_BASE, 0x54, 
        (SEG_END_ADDR(256*MiB) << 16) | SEG_START_ADDR(128*MiB));
    
    // 5. Enable DMA and interrupts (optional for fast reads)
    aspeed_smc_write(FMC_BASE, R_INTR_CTRL, INTR_CTRL_DMA_EN);
    nvic_enable_irq(39);  // FMC DMA IRQ
    
    // 6. SPI1 and SPI2 configured similarly...
}
```

### 6.3 Read Operation (Programmed I/O vs. DMA)

#### Programmed I/O (Slow but Simple)
```c
uint32_t fmc_read_pio(uint32_t flash_offset, uint8_t *buf, size_t len) {
    // Method: direct CPU access via flash window
    // CPU reads map through FMC controller → SPI master → flash device
    
    uint32_t *flash_window = (uint32_t *)(FMC_FLASH_WINDOW + flash_offset);
    for (size_t i = 0; i < len; i += 4) {
        *(uint32_t *)(buf + i) = *flash_window++;  // CPU fetch → SPI read
    }
    return len;
}

// Pros: No register setup, automatic pipelining via AHB
// Cons: Stalls on every read if timing incorrect; blocks CPU
```

#### DMA Operation (Fast, Non-blocking)
```c
int fmc_dma_read(uint32_t flash_offset, uintptr_t dram_addr, size_t len) {
    // Setup DMA registers for bulk transfer
    uint32_t segment = (SEG_END_ADDR(flash_offset + len) << 16) |
                       SEG_START_ADDR(flash_offset);
    
    aspeed_smc_write(FMC_BASE, 0x50, segment);  // Flash address range
    aspeed_smc_write(FMC_BASE, R_DMA_DRAM_ADDR, dram_addr & FMC_DMA_DRAM_MASK);
    aspeed_smc_write(FMC_BASE, R_DMA_SPI_LEN, len - 1);
    
    // Trigger DMA
    aspeed_smc_write(FMC_BASE, R_DMA_CTRL, DMA_CTRL_REQUEST);
    
    // Return immediately; caller polls or interrupts for completion
    return 0;  // Pending
}

void fmc_dma_isr(void) {
    // Called on DMA completion IRQ
    // Safe to read DRAM buffer now
}
```

### 6.4 Write Operation (Erase + Program + Verify)

Writes to SPI flash require a full sequence:

1. **Send WREN (Write Enable) command** via SPI bit-bang or command queue
2. **Send Sector Erase (0x20 or 0xD8)** + 24-bit address
3. **Poll WIP (Write In Progress) bit** until erase complete (~100–500 ms typical)
4. **Send WREN + PP (Page Program, 0x02)** + address + data (up to 256 bytes)
5. **Poll WIP** until program complete (~1–10 ms)
6. **Read back & verify** using DMA

**HAL Example (simplified):**
```c
int fmc_spi_flash_write(uint32_t addr, const uint8_t *data, size_t len) {
    // Assumes single 256-byte page write
    
    uint8_t cmd[4];
    
    // 1. WREN
    cmd[0] = 0x06;
    fmc_spi_write_cmd(cmd, 1);
    
    // 2. Erase sector (assume 4-KB sectors)
    cmd[0] = 0x20;
    cmd[1] = (addr >> 16) & 0xFF;
    cmd[2] = (addr >> 8) & 0xFF;
    cmd[3] = addr & 0xFF;
    fmc_spi_write_cmd(cmd, 4);
    
    // 3. Poll SR until !WIP
    while (fmc_spi_flash_read_status() & 0x01) {
        // Busy
    }
    
    // 4. WREN + PP
    cmd[0] = 0x06;
    fmc_spi_write_cmd(cmd, 1);
    
    cmd[0] = 0x02;
    cmd[1] = (addr >> 16) & 0xFF;
    cmd[2] = (addr >> 8) & 0xFF;
    cmd[3] = addr & 0xFF;
    fmc_spi_write_cmd_data(cmd, 4, data, len);
    
    // 5. Poll SR until !WIP
    while (fmc_spi_flash_read_status() & 0x01) {
        // Busy
    }
    
    // 6. Verify (DMA read and compare)
    return fmc_verify_write(addr, data, len);
}
```

### 6.5 Dual-Image Boot and Atomic Switch

**Typical topology:**
- **FMC CS0**: Primary kernel (read-only at runtime)
- **FMC CS1**: Alternate kernel (updated offline)
- **FMC WDT2_CTRL[4]**: Hardware mux bit → selects boot source

**Switch procedure:**
```c
int fmc_switch_boot_image(int use_alternate) {
    uint32_t wdt2_ctrl = aspeed_smc_read(FMC_BASE, 0x64);  // R_FMC_WDT2_CTRL
    
    if (use_alternate) {
        wdt2_ctrl |= (1 << 4);   // Set BOOT_SOURCE = alternate
    } else {
        wdt2_ctrl &= ~(1 << 4);  // Clear BOOT_SOURCE = primary
    }
    
    aspeed_smc_write(FMC_BASE, 0x64, wdt2_ctrl);
    
    // Force watchdog reset to activate switch
    watchdog_trigger_reset();   // Triggers WDT IRQ, then POR
    while (1) { }  // Hang until reset
}
```

---

## 7. HAL API Reference (Template)

### 7.1 Initialization

```c
/**
 * Initialize all three SMC controllers (FMC, SPI1, SPI2)
 * Assumes SCU clock already set to 200 MHz.
 */
int hal_smc_init(void);

/**
 * Configure a specific flash device on a given controller.
 * @param ctrl: HAL_SMC_FMC, HAL_SMC_SPI1, or HAL_SMC_SPI2
 * @param cs: Chip select (0 or 1)
 * @param capacity_mb: Device size in MB (e.g., 64, 128, 256)
 * @param speed_mhz: Desired SPI clock speed
 */
int hal_smc_configure_flash(int ctrl, int cs, int capacity_mb, int speed_mhz);
```

### 7.2 Read Operations

```c
/**
 * Synchronous (PIO) read from flash.
 * @param ctrl: HAL_SMC_FMC, HAL_SMC_SPI1, or HAL_SMC_SPI2
 * @param offset: Byte offset in flash
 * @param buf: Destination SRAM buffer
 * @param len: Byte count to read
 * @return: Bytes read, or negative on error
 */
int hal_smc_read(int ctrl, uint32_t offset, void *buf, size_t len);

/**
 * Asynchronous (DMA) read from flash.
 * @param ctrl: HAL_SMC_FMC, HAL_SMC_SPI1, or HAL_SMC_SPI2
 * @param offset: Flash offset
 * @param dram_addr: SRAM destination address
 * @param len: Byte count
 * @param callback: Optional callback on completion (NULL for polling)
 * @return: DMA request ID (for tracking), or negative on error
 */
int hal_smc_dma_read(int ctrl, uint32_t offset, uintptr_t dram_addr, 
                     size_t len, void (*callback)(int));

/**
 * Check DMA transfer status.
 * @param dma_id: Request ID from hal_smc_dma_read()
 * @return: 0 if complete, 1 if in-progress, <0 on error
 */
int hal_smc_dma_status(int dma_id);
```

### 7.3 Write Operations

```c
/**
 * Erase a sector in flash.
 * @param ctrl: Controller ID
 * @param sector_addr: Starting address (must be sector-aligned)
 * @param sector_size: 4096 (4K) or 65536 (64K) depending on device
 * @return: 0 on success, <0 on error
 */
int hal_smc_erase_sector(int ctrl, uint32_t sector_addr, int sector_size);

/**
 * Program (write) data to flash.
 * @param ctrl: Controller ID
 * @param offset: Flash offset (must be page-aligned)
 * @param data: Source buffer
 * @param len: Byte count (max 256 for typical page size)
 * @return: Bytes written, or <0 on error
 */
int hal_smc_program_page(int ctrl, uint32_t offset, const void *data, size_t len);

/**
 * Verify written data.
 * @param ctrl: Controller ID
 * @param offset: Flash offset
 * @param expected: Expected data buffer
 * @param len: Byte count to verify
 * @return: 0 if match, 1 if mismatch, <0 on error
 */
int hal_smc_verify(int ctrl, uint32_t offset, const void *expected, size_t len);
```

### 7.4 Interrupt and Watchdog Boot

```c
/**
 * Switch boot image (FMC only).
 * Atomically toggles FMC_WDT2_CTRL[4] and triggers reset.
 * @param use_alternate: 1 for CS1, 0 for CS0
 * @return: Does not return (CPU resets)
 */
int hal_smc_switch_boot_image(int use_alternate);

/**
 * Register DMA completion callback.
 * @param ctrl: Controller ID
 * @param handler: Interrupt handler function pointer
 */
void hal_smc_register_irq_handler(int ctrl, void (*handler)(int));
```

---

## 8. Common Pitfalls and Best Practices

### 8.1 Timing Misconfigurations

❌ **Mistake**: Not adjusting timing registers after reset
```c
// WRONG: Hardware defaults may not match your flash device
aspeed_smc_write(FMC_BASE, R_CONF, CONF_ENABLE_W0);
// Read fails silently or returns garbage
```

✅ **Correct**: Explicitly set timing
```c
// Set SPI clock to 25 MHz (safe for most W25Q flash)
uint32_t timings = (1 << 28) | (1 << 24) | (3 << 20);
aspeed_smc_write(FMC_BASE, R_TIMINGS, timings);
```

### 8.2 Segment Address Overflow

❌ **Mistake**: Segment registers use 4 KB granularity; misaligned ends
```c
// Incorrect: 0x12345 is not 4 KB aligned
aspeed_smc_write(FMC_BASE, 0x50, 0x00012345);
```

✅ **Correct**: Use 4 KB units
```c
#define TO_4K(addr) ((addr) >> 12)
// Map 64 MB starting at 0x00000000
aspeed_smc_write(FMC_BASE, 0x50, 
    (TO_4K(64*1024*1024) << 16) | TO_4K(0));
```

### 8.3 DMA Buffer Alignment and Masking

❌ **Mistake**: DRAM buffer not within SRAM window
```c
// WRONG: External DRAM (if configured) may fail or cause cache coherency issues
uint32_t external_dram = 0x80000000;  // Outside SRAM
aspeed_smc_write(FMC_BASE, R_DMA_DRAM_ADDR, external_dram);
```

✅ **Correct**: Use SRAM (0x00000000–0x00010000 or 0x79000000–0x79009000)
```c
uint32_t sram_buffer = 0x04000000;  // Within SRAM window
aspeed_smc_write(FMC_BASE, R_DMA_DRAM_ADDR, sram_buffer & FMC_DMA_DRAM_MASK);
```

### 8.4 Interrupt Race Conditions

❌ **Mistake**: DMA completion checked before controller finishes
```c
void dma_read_fast(uint32_t offset, void *buf, size_t len) {
    aspeed_smc_write(FMC_BASE, R_DMA_DRAM_ADDR, (uintptr_t)buf);
    aspeed_smc_write(FMC_BASE, R_DMA_SPI_LEN, len - 1);
    aspeed_smc_write(FMC_BASE, R_DMA_CTRL, DMA_CTRL_REQUEST);
    
    // Immediate read: data not ready yet!
    process_buffer(buf, len);
}
```

✅ **Correct**: Use interrupts or robust polling
```c
volatile int dma_done = 0;

void fmc_dma_isr(void) {
    dma_done = 1;
}

void dma_read_safe(uint32_t offset, void *buf, size_t len) {
    dma_done = 0;
    aspeed_smc_write(FMC_BASE, R_DMA_DRAM_ADDR, (uintptr_t)buf);
    aspeed_smc_write(FMC_BASE, R_DMA_SPI_LEN, len - 1);
    aspeed_smc_write(FMC_BASE, R_DMA_CTRL, DMA_CTRL_REQUEST);
    
    // Wait for interrupt
    while (!dma_done) {
        __wfe();  // Wait for event
    }
    process_buffer(buf, len);
}
```

### 8.5 CS Selection and Multi-Flash Issues

❌ **Mistake**: Assuming active CS line persists across reads
```c
uint32_t *ptr = (uint32_t *)FMC_FLASH_WINDOW;
uint32_t val1 = ptr[0];   // Reads from CS0
uint32_t val2 = ptr[128*MiB/4];  // Assumes CS1, but may still be CS0 if segment not set
```

✅ **Correct**: Explicitly configure segment registers
```c
// Set up CS0 and CS1 segments first (as in section 6.2)
aspeed_smc_write(FMC_BASE, 0x50,
    (TO_4K(64*MiB) << 16) | TO_4K(0));      // CS0: 0–64 MB
aspeed_smc_write(FMC_BASE, 0x54,
    (TO_4K(128*MiB) << 16) | TO_4K(64*MiB)); // CS1: 64–128 MB

uint32_t *ptr = (uint32_t *)FMC_FLASH_WINDOW;
uint32_t val1 = ptr[0];              // CS0[0]
uint32_t val2 = ptr[(64*MiB)/4];     // CS1[0]
```

### 8.6 Write Enable Bit Management

❌ **Mistake**: Forgetting to enable write on a CS
```c
aspeed_smc_write(FMC_BASE, R_CONF, FLASH_TYPE_SPI << CONF_FLASH_TYPE0);
// Writes to CS0 will fail silently (hardware ignores WR requests)
```

✅ **Correct**: Enable write per CS
```c
uint32_t conf = (CONF_ENABLE_W0 | CONF_ENABLE_W1) |
                (FLASH_TYPE_SPI << CONF_FLASH_TYPE0) |
                (FLASH_TYPE_SPI << CONF_FLASH_TYPE1);
aspeed_smc_write(FMC_BASE, R_CONF, conf);
```

---

## 9. Testing and Validation

### 9.1 Unit Tests for HAL

```c
void test_fmc_read_pio(void) {
    // Setup: FMC CS0 should contain known data (e.g., "BOOT" magic)
    
    uint8_t buf[4];
    int ret = hal_smc_read(HAL_SMC_FMC, 0, buf, 4);
    
    assert(ret == 4);
    assert(buf[0] == 'B' && buf[1] == 'O' && buf[2] == 'O' && buf[3] == 'T');
}

void test_fmc_dma_read(void) {
    // Trigger DMA, wait for completion, verify data in SRAM
    
    int dma_id = hal_smc_dma_read(HAL_SMC_FMC, 0x1000, 0x04000000, 1024, NULL);
    assert(dma_id >= 0);
    
    while (hal_smc_dma_status(dma_id) == 1) {
        // Poll
    }
    
    uint8_t *sram = (uint8_t *)0x04000000;
    assert(sram[0] == 0xAA);  // Expected magic
}

void test_dual_boot_switch(void) {
    // Read primary image magic, then switch to alternate
    
    uint8_t primary[4];
    hal_smc_read(HAL_SMC_FMC, 0, primary, 4);
    
    hal_smc_switch_boot_image(1);  // Does not return; CPU resets
}
```

### 9.2 Integration Test: Cold Boot to App

```
1. Flash HAL test binary to FMC CS0
2. Flash alternate firmware to FMC CS1
3. Power cycle AST1030
4. HAL initializes FMC, SPI1, SPI2
5. Test code reads both CS0 and CS1, verifies magic numbers
6. Switch boot and reset
7. Verify CPU booted from CS1 (by unique marker)
```

---

## 10. References and Further Reading

### 10.1 Aspeed Documentation

- **AST1030 Datasheet**: Available from Aspeed (internal link or GitHub)
  - Chapter 3: SoC Overview
  - Chapter 4: Memory Map
  - Chapter 5: FMC / SMC Controller
  - Chapter 6: SPI Controller

- **QEMU AST1030 Implementation**: `/home/rusty1968/tools/qemu/v8.2.4/`
  - `hw/arm/aspeed_ast10x0.c` – SoC device model
  - `hw/ssi/aspeed_smc.c` – SMC/FMC/SPI controller model
  - `hw/arm/aspeed_ast10x0_evb.c` – Board initialization (machine type)

### 10.2 Flash Device Specifications

- **Winbond W25Q80BL**: 8 Mb SPI Flash (1 MB)
  - Typical for FMC CS1 (backup images)
  - Datasheet: https://www.winbond.com/

- **Winbond W25Q256**: 256 Mb SPI Flash (32 MB)
  - Typical for FMC CS0 or SPI1/SPI2 primary
  - Supports 4-address byte mode for >128 MB

### 10.3 ARM Cortex-M4 Resources

- **ARM Generic Interrupt Controller (NVIC)**: ARM Cortex-M4 Devices Generic User Guide
  - Describes NVIC register model and exception prioritization

### 10.4 QEMU Aspeed Board Documentation

- `docs/system/arm/aspeed.rst` – QEMU Aspeed machine types and supported devices

---

## 11. Implementation Checklist

- [ ] Verify SCU has SYSCLK set to 200 MHz before HAL init
- [ ] Implement `hal_smc_init()` with FMC, SPI1, SPI2 configuration
- [ ] Program timing registers based on flash device datasheet
- [ ] Set up segment registers to map CS0 and CS1 non-overlapping windows
- [ ] Implement interrupt handlers for FMC, SPI1, SPI2 DMA completion
- [ ] Test PIO read from all three controllers
- [ ] Test DMA read with interrupt-driven completion
- [ ] Implement flash erase and program routines (SPI command sequences)
- [ ] Test dual-boot switching and watchdog-triggered reset
- [ ] Validate multi-image support (primary, alternate, config, log)
- [ ] Add error handling for timeout, CRC, erase verification
- [ ] Document HAL API and provide examples in application README

---

## 12. Appendix: HAL Template Code

```c
// hal_smc.h
#ifndef _HAL_SMC_H_
#define _HAL_SMC_H_

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

#define HAL_SMC_FMC   0
#define HAL_SMC_SPI1  1
#define HAL_SMC_SPI2  2

typedef void (*hal_smc_callback_t)(int ctrl, int status);

int hal_smc_init(void);
int hal_smc_configure_flash(int ctrl, int cs, int capacity_mb, int speed_mhz);
int hal_smc_read(int ctrl, uint32_t offset, void *buf, size_t len);
int hal_smc_dma_read(int ctrl, uint32_t offset, uintptr_t dram_addr, 
                     size_t len, hal_smc_callback_t callback);
int hal_smc_dma_status(int dma_id);
int hal_smc_erase_sector(int ctrl, uint32_t sector_addr, int sector_size);
int hal_smc_program_page(int ctrl, uint32_t offset, const void *data, size_t len);
int hal_smc_verify(int ctrl, uint32_t offset, const void *expected, size_t len);
int hal_smc_switch_boot_image(int use_alternate);
void hal_smc_register_irq_handler(int ctrl, void (*handler)(int));

#endif // _HAL_SMC_H_
```

---

**Document version**: 1.0  
**Last updated**: April 30, 2026  
**Target SoC**: ASPEED AST1030 (Rev A1)  
**QEMU version**: v8.2.4
