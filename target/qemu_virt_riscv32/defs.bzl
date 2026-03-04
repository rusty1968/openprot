# Licensed under the Apache-2.0 license

"""Common definitions used by all qemu_virt_riscv32 targets."""

TARGET_COMPATIBLE_WITH = select({
    "//target/qemu_virt_riscv32:target_qemu_virt_riscv32": [],
    "//conditions:default": ["@platforms//:incompatible"],
})
