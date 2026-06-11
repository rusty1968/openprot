<!-- Licensed under the Apache-2.0 license -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# AST10x0 SGPIOM Interrupt Bring-up (`sgpiom_irq`)

Real-hardware test image that exercises the SGPIOM interrupt path end-to-end
through the OpenPRoT microkernel **wait-on-object** model — the same model the
EarlGrey GPIO driver documents, where interrupts are delivered to userspace via
syscalls rather than in-driver ISR callbacks.

This is **not** a QEMU test: `sgpiom_irq_test` is tagged `hardware` and is marked
incompatible when `qemu_enabled`. It must run on a real AST1060 board.

## What it does

A single userspace app (`sgpiom_irq_server`):

1. Owns the SGPIOM register block (device mapping `sgpiom_regs` @ `0x7e780500`).
2. Programs both-edge sensitivity on a watched SGPIO input mask via the HAL
   `GpioInterrupt::irq_configure`.
3. Registers the SGPIOM interrupt object (**IRQ 51**, `INTR_SGPIOM`) with a wait
   group, then enables the pins via `GpioInterrupt::irq_control(Enable)` and
   unmasks the line at the kernel with `interrupt_ack`.
4. Blocks on `object_wait(WG, signals::SGPIOM)`. On each wakeup it reads the
   latched interrupt-status register, clears it (`irq_control(Clear)`), and
   re-arms delivery.
5. After `EXPECTED_EDGES` wakeups it shuts down **PASS**; if the wait budget
   (`WAIT_BUDGET_MS`) elapses first it shuts down **FAIL**.

Console sentinels: `TEST_RESULT:PASS` / `TEST_RESULT:FAIL` (emitted by the
kernel `Target::shutdown`).

## Pin mux

The kernel applies `pinctrl::PINCTRL_SGPIOM` at boot (SCU41C[8:11]):

| pin       | SCU41C bit | function          |
|-----------|------------|-------------------|
| `sgpmclk` | 8          | serial clock out  |
| `sgpmld`  | 9          | load/latch out    |
| `sgpmo`   | 10         | serial data out   |
| `sgpmi`   | 11         | serial data in    |

SGPIOM is clocked by PCLK (always running), so no clock ungate or controller
reset is performed.

## External stimulus (REQUIRED)

SGPIOM samples external serial GPIO **inputs**; an interrupt fires only when a
watched input bit changes. A bare board produces no edges, so the test will
time out **FAIL** without stimulus.

Drive the watched SGPIO_A input bits externally — e.g. a second board acting as
an SGPIO source, or the input shift-register chain wired to the SGPIOM serial
pins (`sgpmclk`/`sgpmld`/`sgpmo`/`sgpmi`). This mirrors the two-board harness
model of the I2C IRQ master/slave test (`tests/peripherals/i2c/i2c_irq`).

Watched mask: bank A, pins `0..=3` (`WATCH_MASK = 0x0f`); both edges count.

## Build / run

```
bazel build //target/ast10x0/tests/peripherals/sgpiom/sgpiom_irq:sgpiom_irq
bazel test  //target/ast10x0/tests/peripherals/sgpiom/sgpiom_irq:sgpiom_irq_test   # hardware only
bazel test  //target/ast10x0/tests/peripherals/sgpiom/sgpiom_irq:no_panics_test
```

## Tunables (`sgpiom_irq_server_main.rs`)

| const             | meaning                                  |
|-------------------|------------------------------------------|
| `NGPIOS`          | total SGPIO count in the global config   |
| `CLOCK_DIV`       | serial clock divider                     |
| `WATCH_MASK`      | which bank-A pins to watch                |
| `EXPECTED_EDGES`  | wakeups required for PASS                 |
| `POLL_MS`         | per-`object_wait` timeout                |
| `WAIT_BUDGET_MS`  | total wait before FAIL                    |
