# FPGA bootstrap guide (Versal VMK180, Caliptra CI image)

This guide walks through the flow that worked end to end: bootstrap the board,
build firmware and tests, and run a hardware test. Example outputs are from a
successful run. Replace `<FPGA>` with `runner@<board-ip-or-name>` (the run below
used `runner@10.223.163.182`).

All commands run from the repo root on your **build machine** (an x86 Linux
box or WSL2), unless a step says "on the board".

## 0. How the flow works

```
build machine                                       FPGA board (Caliptra CI image)
  cargo xtask-fpga fpga bootstrap ──ssh/rsync──►    downloads bitstream (PDI), loads it
  cargo xtask-fpga fpga build      (container)      copies all-fw.zip
  cargo xtask-fpga fpga build-test (container)      copies caliptra-test-binaries.tar.zst
  cargo xtask-fpga fpga test       ──ssh──────────► runs cargo-nextest on the board
```

- Builds run locally in the `ghcr.io/chipsalliance/caliptra-builder` container
  (podman or docker) and are cross-compiled for the board (aarch64).
- Bootstrap downloads the bitstream pinned in
  `hw/fpga/bitstream_manifests/subsystem.toml` and loads it through
  `/sys/class/fpga_manager/fpga0/firmware`. It does **not** use any bitstream
  that was already on the board.
- Bootstrap must be rerun after every board boot or power cycle.

## 1. Requirements

### Board

- The Caliptra FPGA CI image (Petalinux-based). The stock Ubuntu image is not
  supported by this flow (no `fpga_manager` device, no run-time bitstream load).
- **Hostname must be exactly `caliptra-fpga`.** xtask uses the hostname to
  decide whether the board is a CI image. Any other name makes bootstrap skip
  the bitstream step and try to build and load the kernel modules itself.
- `rsync` and `cargo-nextest` installed (the CI image has them).
- Network access to the build machine, and to `storage.googleapis.com` (for the
  bitstream) and `github.com` (for step 6).
- SSH key login for the user (`runner` on the CI image). Password login may be
  disabled, so give the board owner your **public** key.

### Build machine

- `podman` or `docker`
- `rsync`
- `rustup` (the repo's `rust-toolchain.toml` pins the toolchain, which rustup
  installs on first use inside the repo)
- `cargo install cargo-nextest --locked`
- A clone of this repo **with the `hw/caliptra-ss` submodule**
- Free disk space. Roughly 25 GB was used by the toolchain (~9 GB), the builder
  image (~6.5 GB), and `target/` (~4.5 GB), so keep 30 GB or more free.

## 2. One-time setup

### 2.1 Clone and initialize the submodule

```
git clone git@github.com:chipsalliance/caliptra-mcu-sw.git
cd caliptra-mcu-sw
git submodule update --init hw/caliptra-ss
```

Expected:

```
Submodule path 'hw/caliptra-ss': checked out 'b5718a0f7fe7936b5082a8dfc23add896755af3a'
```

Without this, `build-test` fails (see Troubleshooting).

### 2.2 Pull the builder image

```
podman pull ghcr.io/chipsalliance/caliptra-builder:latest
```

Expected (the image is about 1.7 GB compressed):

```
Trying to pull ghcr.io/chipsalliance/caliptra-builder:latest...
Getting image source signatures
Copying blob ... done
...
Copying config b0cb95a2b6 done
Writing manifest to image destination
Storing signatures
b0cb95a2b66062a01e5c5823c987d1b6ae518caefdfc90b0f33de15012d424fe
```

Do this by hand the first time. The xtask pull hides its output and ignores
failures, so a broken pull looks like a hang.

### 2.3 SSH access to the board

1. Send the board owner your public key (`cat ~/.ssh/id_ed25519.pub`).
2. If your key has a passphrase, load it into an agent. The xtask SSH calls are
   non-interactive and cannot prompt:
   ```
   eval "$(ssh-agent -s)"
   ssh-add ~/.ssh/id_ed25519
   ```
3. Connect once by hand to accept the host key. Compare the fingerprint with
   the board owner's before answering `yes`:
   ```
   ssh <FPGA> hostname
   ```

### 2.4 Hostname

On the board:

```
sudo hostnamectl set-hostname caliptra-fpga
hostname
```

Expected:

```
caliptra-fpga
```

Check with the board owner first. The name is how other tools may find it.

### 2.5 Match the board's checkout to your tree

nextest runs inside `~/caliptra-mcu-sw` **on the board** and reads that
checkout's `.config/nextest.toml`. If the checkout is older than your tree, its
package filters do not match your build and nextest refuses to start. Check the
board's version, then update it to your commit:

```
ssh <FPGA> 'cd caliptra-mcu-sw && git log -1 --oneline && git status --short | head'
ssh <FPGA> 'cd caliptra-mcu-sw && git fetch --depth=1 origin <FULL-COMMIT-SHA> && git checkout --force FETCH_HEAD && git log -1 --oneline'
```

Use `git rev-parse HEAD` on the build machine for the SHA. `--force` discards
local changes on the board, so check `git status` first. Expected last line:

```
20f7fcf [2.0] Add DOT enable command support (#2187)
```

## 3. Bootstrap

```
cargo xtask-fpga fpga bootstrap --target-host <FPGA>
```

Expected (successful run):

```
Checking for build image updates
Bootstrapping FPGA
configuration: Subsystem
[FPGA] Running command: hostname
[FPGA] Running command: echo "subsystem" > /dev/shm/fpga-config
[FPGA] Running command: [ -d caliptra-mcu-sw ] || git clone https://github.com/chipsalliance/caliptra-mcu-sw --branch=main --depth=1
Downloading bitstream: subsystem-bitstream
URL: https://storage.googleapis.com/caliptra-github-ci-bitstreams/v1/<commit>/runtime_<commit>.pdi
Expected hash: 43e669a7...
Calculated hash: 43e669a7...
Hash verification successful!
PDI saved to: subsystem.pdi
sending incremental file list
subsystem.pdi
[FPGA] Running command: sudo mkdir -p /lib/firmware
[FPGA] Running command: sudo mv caliptra-bitstream.pdi /lib/firmware
[FPGA] Running command: sudo bash -c 'echo "caliptra-bitstream.pdi" > /sys/class/fpga_manager/fpga0/firmware'
```

What it did: wrote the configuration marker, made sure the repo exists on the
board, downloaded and verified the bitstream, and loaded it into the FPGA.
There is no kernel-module build here, because the CI image already has them.

### Verify

```
ssh <FPGA> 'cat /sys/class/fpga_manager/fpga0/state; ls /dev/uio*; lsmod | grep io_module; cat /dev/shm/fpga-config'
```

Expected:

```
operating
/dev/uio0
/dev/uio1
io_module              12288  0
subsystem
```

`operating` means the FPGA accepted the bitstream. It does not prove the
Caliptra logic responds. The test in step 5 does that.

## 4. Build

```
cargo xtask-fpga fpga build --target-host <FPGA>
cargo xtask-fpga fpga build-test --target-host <FPGA>
```

`build` ends with the firmware bundle and an rsync to the board:

```
Creating ZIP file: all-fw.zip
Adding caliptra_rom.bin: 98304 bytes
...
sending incremental file list
all-fw.zip
```

You may see lines like `error: the package 'mcu-rom-fpga' does not contain
this feature: test-fw-manifest-dot`. These come from feature ROMs the FPGA ROM
crate does not define. The builder skips them (`will use generic ROM`) and the
build still succeeds. Judge the build by whether `all-fw.zip` is created and
copied.

`build-test` ends with:

```
    Finished `release` profile [optimized + debuginfo] target(s) in 3m 04s
   Archiving 142 binaries ... to /work-dir/caliptra-test-binaries.tar.zst
sending incremental file list
caliptra-test-binaries.tar.zst
```

Compiler warnings in between are normal.

## 5. Run a hardware test

```
cargo xtask-fpga fpga test --target-host <FPGA> \
  --test-filter='package(caliptra-mcu-tests-integration) and test(test_jtag_taps)' \
  --test-output
```

Pick tests from the `nightly-ci` profile in `.config/nextest.toml`. Package
names carry the `caliptra-mcu-` prefix in this tree.

Expected:

```
Running test suite on FPGA
[FPGA] Running command: lsmod
...
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 254 filtered out; finished in 0.50s
        PASS [   0.511s] (1/1) caliptra-mcu-tests-integration jtag::test_jtag_taps::test::test_lcc_tap
     Summary [   0.522s] 1 test run: 1 passed, 254 skipped
Copying test log from FPGA to junit.xml
```

Some tests, especially ones that program OTP with the lifecycle controller in
the Dev state, are known to hang the FPGA (see the comments in
`.config/nextest.toml`). Start with a short test.

## 6. After a reboot or power cycle

The loaded bitstream, `io_module`, and `/dev/shm/fpga-config` are all lost.
Rerun step 3 (bootstrap), then the verify step. You do not need to repeat the
build.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `Neither 'podman' nor 'docker' found on PATH` | no container tool on the build machine | install podman or docker |
| `'cargo-nextest' not found on PATH` | missing locally (or on the board) | `cargo install cargo-nextest --locked` |
| `Host key verification failed` | non-interactive SSH, host key not yet accepted | connect once by hand (step 2.3) |
| `Permission denied (publickey,...)` | key not installed, or passphrase key not in an agent | install the key, run `ssh-add` |
| image pull stalls or fails with `unexpected EOF` | network path drops long downloads (VPN, MTU, proxy) | retry, or pull on another network and move the image with `podman save` / `podman load` |
| `No space left on device`, linker `Bus error` | disk full on the build machine | free space (30 GB or more) |
| bootstrap prints no bitstream steps, or fails at `insmod ... File exists` | board hostname is not `caliptra-fpga` | step 2.4 |
| `no such file` panic in `provisioning/fuses/lib/build.rs` | `hw/caliptra-ss` submodule not initialized | step 2.1 |
| `FPGA is not bootstrapped` | `/dev/shm/fpga-config` missing, usually after a reboot | rerun bootstrap |
| nextest: `failed to parse profile.nightly-ci.default-filter ... no packages matched` | the board's checkout is older than your tree | step 2.5 |
| `/sys/class/fpga_manager/fpga0` missing | not the CI image (for example stock Ubuntu) | use the Caliptra CI image |
| `ssh: Could not resolve hostname` | DNS name stopped resolving | use the board's IP, and confirm the fingerprint |
