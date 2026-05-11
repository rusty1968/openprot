# AST10x0 Pigweed Target

Pigweed kernel target for the AST10x0 platform.

## Building

Build all targets under the AST10x0 tree with:

```console
bazel build //target/ast10x0/...
```

## Running Tests

### QEMU kernel tests

The `virt_ast10x0` config sets `--platforms=//target/ast10x0` and launches
system images under Pigweed's QEMU runner (`ast1030-evb` machine, semihosting).
Targets tagged `integration` are excluded by default via the `k_common` tag
filter (`--test_tag_filters=-integration,...`):

```console
bazelisk test --config=virt_ast10x0 //target/ast10x0/...
```

### Full test run including integration tests

Pass an empty `--test_tag_filters=` to override `k_common` and include all
`integration`-tagged QEMU-only tests (multi-CS, program/erase, DMA, erase-state):

```console
bazelisk test --config=virt_ast10x0 --test_tag_filters= //target/ast10x0/...
```

For more detailed failures add `--verbose_failures` to either command.

## Notes

- `bazel build //target/ast10x0/...` builds all targets but does not execute
  any tests.
- `--config=virt_ast10x0` excludes `integration`-tagged targets by default.
  Use `--test_tag_filters=` (empty) to include them.
- The `integration` tag covers QEMU-only tests that depend on flash model
  behaviour (erase state, multi-CS routing, program/erase cycles) and cannot
  be assumed to be valid on silicon without further qualification.