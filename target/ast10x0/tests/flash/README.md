# AST10x0 Flash QEMU Test

This test validates the flash IPC path end-to-end on QEMU:

- flash client sends `Exists`
- flash server receives IPC request
- AST10x0 flash backend probes JEDEC via FMC/SMC
- client reports pass and triggers test shutdown

## Targets

- `//target/ast10x0/tests/flash:flash`
- `//target/ast10x0/tests/flash:flash_test`

## Build

```bash
cd /home/rusty1968/work/storage/reference
bazelisk build --config=virt_ast10x0 //target/ast10x0/tests/flash:flash
```

## Run Test

```bash
cd /home/rusty1968/work/storage/reference
bazelisk test --config=virt_ast10x0 --test_tag_filters= //target/ast10x0/tests/flash:flash_test
```

## Run With Streamed Output

Use this to watch server/client logs in real time.

```bash
cd /home/rusty1968/work/storage/reference
bazelisk test \
  --config=virt_ast10x0 \
  --test_tag_filters= \
  //target/ast10x0/tests/flash:flash_test \
  --test_output=streamed \
  --nocache_test_results
```

Note: the flag is `--nocache_test_results` (plural).

## Inspect Saved Test Log

```bash
cd /home/rusty1968/work/storage/reference
cat bazel-testlogs/target/ast10x0/tests/flash/flash_test/test.log
```

Or follow while running:

```bash
cd /home/rusty1968/work/storage/reference
tail -f bazel-testlogs/target/ast10x0/tests/flash/flash_test/test.log
```

## Expected Messages

When the path is healthy, streamed output should include lines similar to:

- `flash_server: ipc rx ch=... req_len=... op=0x01`
- `flash_server: ipc tx ch=... resp_len=...`
- `flash exists check passed`

## Design Note: Per-Controller Process Isolation

Each SMC controller (FMC, SPI1, SPI2) runs as a separate pigweed kernel process with its
own MPU domain. This is an intentional minimal-TCB design choice, not an artifact of the
test structure.

**Why not one process for all three controllers (Zephyr style)?**

Zephyr's `aspeed_flash` driver uses a single kernel-mode driver that owns all three
controllers and their shared `.ram_nc` DMA region. This is efficient — one contiguous
non-cacheable SRAM allocation, one allocator, easy buffer reuse across controllers — but
it means a bug in any controller path can corrupt the state of the others.

In the reference architecture, process isolation is the security primitive:

- FMC holds the RoT boot image. A defect in SPI1/SPI2 handling cannot corrupt FMC state.
- Each process is granted only the hardware resources (register block, flash window, IRQ,
  DMA buffer) it needs. The MPU enforces this at runtime, not by convention.
- A compromised or faulting SPI1 server is contained. The kernel can restart it without
  touching FMC or SPI2.

**DMA buffer allocation in the per-process model**

Each server process will be granted its own 4 KB slice of non-cacheable SRAM
(`0xA0000`–`0xBFFFF`, the 128 KB NC window above the 640 KB cached region) via a
`memory_mapping` entry in `system.json5` with `type: "device"`. This maps to MPU
attributes TEX=000, C=0, B=1 — identical to the `#[link_section = ".ram_nc"]`
placement used in `aspeed-rust`'s bare-metal test.

The codegen emits `mapping::FOO_DMA_BUF_START_ADDRESS` for each mapping, giving the
server the physical address to pass as the DMA destination. Reads larger than the
128-byte DMA minimum use the NC buffer; smaller reads use PIO directly from the
memory-mapped flash window.

The static 4 KB per-controller allocation is the tradeoff for isolation: no cross-process
pool, no dynamic allocation, but also no cross-process corruption risk.
