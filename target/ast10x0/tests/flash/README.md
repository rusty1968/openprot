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
