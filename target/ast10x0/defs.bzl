# Licensed under the Apache-2.0 license
# SPDX-License-Identifier: Apache-2.0
"""Common definitions used by all ast10x0 targets."""

load("@pigweed//pw_kernel/tooling:system_image.bzl", "SystemImageInfo")

TARGET_COMPATIBLE_WITH = select({
    "//target/ast10x0:target_ast10x0": [],
    "//conditions:default": ["@platforms//:incompatible"],
})

def _system_image_test_impl(ctx):
    master_elf = ctx.attr.image[SystemImageInfo].elf
    master_bin = ctx.attr.image[SystemImageInfo].bin

    executable_symlink = ctx.actions.declare_file(ctx.label.name)
    ctx.actions.symlink(output = executable_symlink, target_file = master_elf)

    master_bin_symlink = ctx.actions.declare_file(ctx.label.name + ".bin")
    ctx.actions.symlink(output = master_bin_symlink, target_file = master_bin)

    runfiles = ctx.runfiles(files = [master_bin_symlink]).merge(ctx.attr.image[DefaultInfo].default_runfiles)

    if ctx.attr.slave_image:
        slave_elf = ctx.attr.slave_image[SystemImageInfo].elf
        slave_bin = ctx.attr.slave_image[SystemImageInfo].bin
        slave_symlink = ctx.actions.declare_file(ctx.label.name + ".slave.elf")
        slave_bin_symlink = ctx.actions.declare_file(ctx.label.name + ".slave.bin")
        ctx.actions.symlink(output = slave_symlink, target_file = slave_elf)
        ctx.actions.symlink(output = slave_bin_symlink, target_file = slave_bin)
        runfiles = ctx.runfiles(files = [slave_symlink, slave_bin_symlink]).merge(
            runfiles.merge(ctx.attr.slave_image[DefaultInfo].default_runfiles),
        )

    return [DefaultInfo(
        executable = executable_symlink,
        runfiles = runfiles,
    )]

system_image_test = rule(
    implementation = _system_image_test_impl,
    test = True,
    attrs = {
        "image": attr.label(
            doc = "The system_image target to test.",
            mandatory = True,
            providers = [SystemImageInfo],
            executable = True,
            cfg = "target",
        ),
        "slave_image": attr.label(
            doc = "Optional slave system_image for paired two-device tests.",
            mandatory = False,
            default = None,
            providers = [SystemImageInfo],
            cfg = "target",
        ),
    },
)
