/* Licensed under the Apache-2.0 license */
/* SPDX-License-Identifier: Apache-2.0 */

/*
 * Memory layout for QEMU RISC-V 32 virt machine.
 * This file is named memory.x and consumed by riscv-rt's link.x.
 *
 * QEMU virt provides:
 *   - RAM at 0x80000000
 *   - CLINT at 0x02000000
 *   - PLIC  at 0x0C000000
 *   - UART  at 0x10000000
 *
 * LENGTH defines the firmware's memory budget and should match the
 * target hardware.  riscv-rt's link.x allocates .heap and .stack to
 * fill the remaining space, so the resulting ELF claims this entire
 * region.  640 KiB is representative of a typical embedded MCU.
 *
 * IMPORTANT: when running under QEMU, the `-m` flag must be set to a
 * value LARGER than LENGTH (e.g. `-m 128M`) because QEMU places a DTB
 * (Device Tree Blob) at the top of physical RAM.  If the firmware's
 * LOAD segment spans all of physical RAM, there is no room for the DTB
 * and QEMU will refuse to start.  On real hardware this is not an issue.
 */

MEMORY
{
    RAM (rwx) : ORIGIN = 0x80000000, LENGTH = 640K
}

REGION_ALIAS("REGION_TEXT",   RAM);
REGION_ALIAS("REGION_RODATA", RAM);
REGION_ALIAS("REGION_DATA",   RAM);
REGION_ALIAS("REGION_BSS",    RAM);
REGION_ALIAS("REGION_HEAP",   RAM);
REGION_ALIAS("REGION_STACK",  RAM);
