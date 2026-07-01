# AST1060 Orchestrator Runner

**Location:** `target/ast1060/orchestrator/`

This document specifies the AST1060-specific implementation of the resiliency
orchestrator runner. It is a concrete instantiation of the generic runner
pattern described in `services/orchestrator/README.md` and `docs/src/architecture.md` — read that first.

---

## What this document covers

- The AST1060 channel handle set (`Handles`)
- Event sources: which services push events, and how they are decoded
- Effect sinks: which services own each hardware resource, and the IPC call for each
- `BUILD.bazel` dependencies
- AST1060-specific behavioral notes (watchdog delivery, sgpiom GPIO model)

---

## Channel handles (`Handles`)

The runner is granted the following channel handles at spawn. These are its
only hardware authority — it holds channels, never MMIO registers.

```rust
pub struct Handles {
    // ── Event sources (channels that wake the WaitGroup) ──────────────────
    /// smbus-mailbox service: delivers PowerOn, RebootRequested,
    /// SeamlessUpdateRequested from the host-side mailbox protocol.
    mailbox_evt: u32,

    /// verifier service: delivers VerifyComplete, RecoveryComplete,
    /// UpdateComplete with their result payloads.
    verifier_evt: u32,

    /// watchdog service: delivers WatchdogTimeout { target } when a domain's
    /// watchdog timer expires.
    /// NOTE: On AST1060 the watchdog fires as an IPC notification from the
    /// watchdog service, not as a direct register read. The service translates
    /// the hardware timer interrupt into a channel push.
    wdt_evt: u32,

    /// sgpiom GPIO service: delivers ResetDetected { target } when a
    /// monitored reset-detect line asserts. The GPIO service owns the IRQ;
    /// it translates IRQ → channel push so the runner never touches the
    /// interrupt controller directly.
    gpio_evt: u32,

    // ── Effect sinks (channels the runner drives) ─────────────────────────
    /// sgpiom service: HoldBoot, ReleaseBoot, ArmMonitors, DisarmMonitors.
    gpio: u32,

    /// watchdog service: ArmWatchdog, DisarmWatchdog.
    wdt: u32,

    /// smbus-mailbox service: SetPlatformState, LogPanic.
    mailbox: u32,

    /// power/reset service: Reboot, HaltBoot.
    power: u32,
}
```

> **`gpio_evt` vs `gpio`** — these are two separate channels to the same
> `sgpiom` service: one for receiving push notifications (events the service
> sends to us) and one for issuing transactional commands (effects we send to
> the service). The service multiplexes both directions; the runner keeps them
> in separate fields to keep the WaitGroup wiring unambiguous.

---

## Event sources and decoding

The runner multiplexes all four event-source channels on a single `WaitGroup`,
tagged by index:

```rust
pub fn run(h: Handles) -> ! {
    let mut sm = Orchestrator::default().uninitialized_state_machine().init();
    let mut plat = IpcPlatform { h: &h };

    let wg = syscall::wait_group_create().unwrap();
    for (handle, tag) in [
        (h.mailbox_evt, 0usize),
        (h.verifier_evt, 1),
        (h.wdt_evt,      2),
        (h.gpio_evt,     3),
    ] {
        syscall::wait_group_add(wg, handle, Signals::READABLE, tag).ok();
    }

    loop {
        let Ok(w) = syscall::object_wait(wg, Signals::READABLE, Instant::MAX) else { continue };
        let Some(event) = read_event(&h, w.user_data) else { continue };
        sm.handle(&event);
        for effect in sm.drain_effects() {
            plat.execute(effect);
        }
    }
}
```

> **Draining requires `&mut` access to the inner model.** The state machine
> framework exposes the initialized machine via `Deref` but restricts mutable
> access to an `unsafe` method. The SM crate therefore exports a safe
> `drain_effects()` forwarder so the runner never needs `unsafe`. Unit tests
> can read `pending` immutably; the runner loop must call `drain_effects()` to
> consume and clear the queue after each `sm.handle()` call.

### `read_event` decode arms

```rust
fn read_event(h: &Handles, tag: usize) -> Option<Event> {
    // Wire format owned by services/orchestrator/api/ (not yet defined).
    // Byte 0 = message type discriminant; remaining bytes = payload.
    let mut buf = [0u8; 16];
    match tag {
        0 => { let n = syscall::channel_read(h.mailbox_evt, 0, &mut buf).ok()?; decode_mailbox(&buf[..n]) }
        1 => { let n = syscall::channel_read(h.verifier_evt, 0, &mut buf).ok()?; decode_verify(&buf[..n]) }
        2 => { let n = syscall::channel_read(h.wdt_evt, 0, &mut buf).ok()?; decode_wdt(&buf[..n]) }
        3 => { let n = syscall::channel_read(h.gpio_evt, 0, &mut buf).ok()?; decode_gpio(&buf[..n]) }
        _ => None,
    }
}
```

### Event source table

| Tag | Channel | Service | Events produced |
|-----|---------|---------|----------------|
| 0 | `mailbox_evt` | smbus-mailbox | `PowerOn`, `RebootRequested`, `SeamlessUpdateRequested` |
| 1 | `verifier_evt` | verifier | `VerifyComplete { result }`, `RecoveryComplete { result }`, `UpdateComplete { result }` |
| 2 | `wdt_evt` | watchdog | `WatchdogTimeout { target }` |
| 3 | `gpio_evt` | sgpiom | `ResetDetected { target }` |

> **`SeamlessUpdateRequested`** is suppressed by `decode_mailbox` when the
> AST1060 build is configured without seamless update support — the variant is
> never passed to `sm.handle()`.

---

## Effect sinks (`IpcPlatform`)

Each `Effect` the SM emits becomes a single `channel_transact` to the service
that owns the hardware:

```rust
struct IpcPlatform<'a> { h: &'a Handles }

impl ResiliencyPlatform for IpcPlatform<'_> {
    fn execute(&mut self, effect: Effect) {
        let mut resp = [0u8; 8];
        match effect {
            Effect::HoldBoot(t)         => transact(self.h.gpio,    &gpio_hold(t, true),  &mut resp),
            Effect::ReleaseBoot(t)      => transact(self.h.gpio,    &gpio_hold(t, false), &mut resp),
            Effect::ArmMonitors         => transact(self.h.gpio,    &MON_ARM,             &mut resp),
            Effect::DisarmMonitors      => transact(self.h.gpio,    &MON_DISARM,          &mut resp),
            Effect::ArmWatchdog         => transact(self.h.wdt,     &WDT_ARM,             &mut resp),
            Effect::DisarmWatchdog      => transact(self.h.wdt,     &WDT_DISARM,          &mut resp),
            Effect::SetPlatformState(s) => transact(self.h.mailbox, &mbx_state(s),        &mut resp),
            Effect::LogPanic            => transact(self.h.mailbox, &MBX_PANIC,           &mut resp),
            Effect::Reboot              => { transact(self.h.power, &PWR_REBOOT,          &mut resp); }
            Effect::HaltBoot            => transact(self.h.power,   &PWR_HALT,            &mut resp),
        }
    }
}

fn transact(handle: u32, req: &[u8], resp: &mut [u8]) {
    if syscall::channel_transact(handle, req, resp, Instant::MAX).is_err() {
        pw_log::error!("orchestrator effect IPC failed");
    }
}
```

### Effect sink table

| Effect | Channel field | Owning service | Hardware |
|--------|--------------|----------------|----------|
| `HoldBoot(RoT)` | `gpio` | sgpiom | RoT boot-hold GPIO line |
| `HoldBoot(HostTarget)` | `gpio` | sgpiom | BMC/host boot-hold GPIO line |
| `ReleaseBoot(RoT)` | `gpio` | sgpiom | RoT boot-hold GPIO line |
| `ReleaseBoot(HostTarget)` | `gpio` | sgpiom | BMC/host boot-hold GPIO line |
| `ArmMonitors` | `gpio` | sgpiom | Reset-detect GPIO lines |
| `DisarmMonitors` | `gpio` | sgpiom | Reset-detect GPIO lines |
| `ArmWatchdog` | `wdt` | watchdog | AST1060 watchdog timer |
| `DisarmWatchdog` | `wdt` | watchdog | AST1060 watchdog timer |
| `SetPlatformState(s)` | `mailbox` | smbus-mailbox | Mailbox registers / status LEDs |
| `LogPanic` | `mailbox` | smbus-mailbox | Last-panic store |
| `Reboot` | `power` | power/reset | Reset controller |
| `HaltBoot` | `power` | power/reset | Reset controller |

---

## `BUILD.bazel`

Mirrors `i2c_server_runtime` conventions:

```python
rust_binary(
    name = "orchestrator",
    srcs = ["src/runner.rs", "src/platform.rs"],
    tags = ["kernel"],
    target_compatible_with = TARGET_COMPATIBLE_WITH,
    deps = [
        "@pigweed//pw_kernel/userspace",
        "@pigweed//pw_log",
        "//services/orchestrator/sm:openprot_resiliency_sm",
        # IPC client crates for each owning service:
        "//services/sgpiom:client",
        "//services/watchdog:client",
        "//services/smbus_mailbox:client",
        "//services/power_reset:client",
    ],
)
```

---

## AST1060-specific behavioral notes

**Watchdog delivery.** On AST1060 the watchdog timer interrupt is owned by the
watchdog service, which translates it into a channel push (`wdt_evt`). The
runner never reads a watchdog register directly. This differs from some other
AST10x0 variants where the runner reads the register inline; the IPC model is
used here to preserve the no-MMIO-in-runner invariant.

**sgpiom GPIO model.** The sgpiom service on AST1060 owns all SGPIO-mapped
lines. Both boot-hold lines and reset-detect lines are accessed via the same
service but over separate channels (`gpio_evt` for inbound notifications,
`gpio` for outbound commands). The `gpio_hold()` helper encodes the
`BootTarget` variant into the sgpiom wire request; the service maps it to the
correct SGPIO line index.

**Wire format.** The `decode_*` helpers and `mbx_state()` / `gpio_hold()`
request builders are not yet defined. They will live in a future
`services/orchestrator/api/` crate (mirroring `services/mctp/api/`) that owns
the binary encoding shared between the pushing services and this runner.

---

## Open questions

- [ ] Define the `services/orchestrator/api/` wire format (byte 0 discriminant
  + payload layout for each message type).
- [ ] Confirm whether `PowerOn` arrives via `mailbox_evt` or a dedicated power
  sequencing channel; this determines whether `decode_mailbox` handles it or a
  fifth channel is needed.
- [ ] Pin the sgpiom wire encoding for `gpio_hold(BootTarget)` and
  `MON_ARM` / `MON_DISARM` requests.
- [ ] Confirm `ResetDetected { target }` is the correct event name and payload
  for reset-detect line assertions pushed by sgpiom.
