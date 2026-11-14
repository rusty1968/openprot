// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! The orchestrator under test: the real `orchestrator-sm` core driven by an
//! asynchronous effect executor.
//!
//! Nothing here blocks on a service. An effect becomes a command queued for
//! one of three channels (verifier, reset, events); the loop starts it when
//! the channel is idle, then sleeps in one WaitGroup wait whose deadline is
//! the earliest of the boot watchdogs and the in-flight verify deadlines.
//! Whatever wakes it is decoded into an `Event` and fed back to the core.
//!
//! A channel carries one transaction at a time, so commands for the same
//! service queue behind each other. The events channel always has one
//! `WaitBoot` long-poll outstanding.
//!
//! Scenarios (see `PLAN_A`, `PLAN_B`):
//!
//! | Scenario        | Shows                                                    |
//! |-----------------|----------------------------------------------------------|
//! | A flaky + slow  | C0 never boots on its first release. C1's verify is slow. |
//! |                 | C0's boot watchdog fires *while* C1 is still verifying,   |
//! |                 | the core recovers C0, C1's late verdict is dropped as     |
//! |                 | stale, and the re-walk ends in `Ready`.                   |
//! | B verify hangs  | C2's verify outlives the executor's deadline. The         |
//! |                 | executor cancels it and reports `VerificationFailed`;     |
//! |                 | the verifier's late reply is dropped; the re-walk ends    |
//! |                 | in `Ready`.                                               |

#![no_main]
#![no_std]

use app_orchestrator::handle;
use openprot_orchestrator_server::BootWatchdogs;
use openprot_orchestrator_sm::{
    Chain, ComponentAttrs, ComponentId, Effect, EffectError, Event, Orchestrator, PowerOnResult,
    State,
};
use pw_status::{Error, Result};
use userspace::entry;
use userspace::syscall::{self, Signals};
use userspace::time::{Clock, Duration, Instant, SystemClock};
use util_ipc::{AsyncChannelTransport, IpcHandle};
use util_transport::AsyncTransport;

const N: usize = 3;
const E: usize = 2 * N + 2;
const MAX_RETRY: u8 = 3;

type Core = Orchestrator<N, E>;
type Transport = AsyncChannelTransport<IpcHandle>;

// Wire opcodes, matching verifier_main.rs and devices_main.rs.
const OP_VERIFY: u8 = 1;
const OP_RELEASE: u8 = 2;
const OP_ASSERT: u8 = 3;
const OP_RESTORE: u8 = 4;
const OP_WAIT: u8 = 5;

/// A component that has not reported in this long after release is failed.
const BOOT_WINDOW: Duration = Duration::from_millis(50);
/// A verify not answered this long after it started is abandoned.
const VERIFY_DEADLINE: Duration = Duration::from_millis(150);
/// A whole scenario that takes longer than this has hung.
const SCENARIO_LIMIT: Duration = Duration::from_millis(4000);

const VERIFIER: usize = 0;
const RESET: usize = 1;
const EVENTS: usize = 2;

/// What each service is told to do, per component and per attempt (the
/// third attempt on repeats the last entry). Units are the services' own
/// (10 ms); a boot of 0 means the device never boots.
struct Plan {
    verify: [[u8; 3]; N],
    boot: [[u8; 3]; N],
}

/// C0 does not boot the first time; C1's verify is slow (100 ms), long
/// enough that C0's 50 ms watchdog fires while it is still running.
const PLAN_A: Plan = Plan {
    verify: [[1, 1, 1], [10, 1, 1], [1, 1, 1]],
    boot: [[0, 2, 2], [2, 2, 2], [2, 2, 2]],
};

/// C2's first verify takes 200 ms, past the 150 ms deadline.
const PLAN_B: Plan = Plan {
    verify: [[1, 1, 1], [1, 1, 1], [20, 1, 1]],
    boot: [[2, 2, 2], [2, 2, 2], [2, 2, 2]],
};

fn pick(table: &[[u8; 3]; N], id: u8, attempt: u8) -> u8 {
    table
        .get(usize::from(id))
        .and_then(|row| row.get(usize::from(attempt.min(2))))
        .copied()
        .unwrap_or(0)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cmd {
    Verify { id: u8, units: u8 },
    Release { id: u8, units: u8 },
    Assert { id: u8 },
    Restore { id: u8 },
    WaitBoot,
}

impl Cmd {
    fn encode(self, out: &mut [u8; 4]) -> usize {
        let (bytes, len) = match self {
            Cmd::Verify { id, units } => ([OP_VERIFY, id, units, 0], 3),
            Cmd::Release { id, units } => ([OP_RELEASE, id, units, 0], 3),
            Cmd::Assert { id } => ([OP_ASSERT, id, 0, 0], 2),
            Cmd::Restore { id } => ([OP_RESTORE, id, 0, 0], 2),
            Cmd::WaitBoot => ([OP_WAIT, 0, 0, 0], 1),
        };
        *out = bytes;
        len
    }

    fn deadline(self) -> Instant {
        match self {
            Cmd::Verify { .. } => SystemClock::now()
                .checked_add_duration(VERIFY_DEADLINE)
                .unwrap_or(Instant::MAX),
            _ => Instant::MAX,
        }
    }
}

#[derive(Clone, Copy)]
struct Inflight {
    cmd: Cmd,
    deadline: Instant,
}

/// One service channel: a transport, the command in flight on it, and the
/// commands waiting for it.
struct Lane {
    index: usize,
    tr: Transport,
    inflight: Option<Inflight>,
    queue: heapless::Deque<Cmd, 8>,
}

impl Lane {
    fn new(index: usize, tr: Transport) -> Self {
        Self {
            index,
            tr,
            inflight: None,
            queue: heapless::Deque::new(),
        }
    }

    fn idle(&self) -> bool {
        self.inflight.is_none() && self.queue.is_empty()
    }

    /// Start the next queued command if the channel is free.
    fn start_next(&mut self) -> Result<()> {
        if self.inflight.is_some() {
            return Ok(());
        }
        let Some(cmd) = self.queue.pop_front() else {
            return Ok(());
        };
        let mut req = [0u8; 4];
        let len = cmd.encode(&mut req);
        self.tr
            .start(req.get(..len).unwrap_or(&[]))
            .map_err(|_| Error::Internal)?;
        syscall::wait_group_add(handle::WG, self.tr.as_raw(), Signals::READABLE, self.index)?;
        self.inflight = Some(Inflight {
            cmd,
            deadline: cmd.deadline(),
        });
        Ok(())
    }

    /// Collect the response if it has landed.
    fn finish(&mut self) -> Result<Option<(Cmd, [u8; 2])>> {
        let Some(inflight) = self.inflight else {
            return Ok(None);
        };
        let mut resp = [0u8; 4];
        match self.tr.poll(&mut resp) {
            Ok(Some(n)) if n >= 2 => {
                syscall::wait_group_remove(handle::WG, self.tr.as_raw())?;
                self.inflight = None;
                Ok(Some((inflight.cmd, [resp[0], resp[1]])))
            }
            Ok(Some(_)) | Err(_) => Err(Error::Internal),
            Ok(None) => Ok(None),
        }
    }

    /// Abandon the command in flight and return it.
    fn cancel(&mut self) -> Result<Option<Cmd>> {
        let Some(inflight) = self.inflight.take() else {
            return Ok(None);
        };
        self.tr.cancel().map_err(|_| Error::Internal)?;
        syscall::wait_group_remove(handle::WG, self.tr.as_raw())?;
        Ok(Some(inflight.cmd))
    }
}

/// What the core was told, in the order the executor delivered it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Seen {
    Timeout(u8),
    Passed(u8),
    Failed(u8),
    Booted(u8),
    Restored(u8),
}

struct Exec {
    lanes: [Lane; 3],
    wd: BootWatchdogs<N>,
    plan: &'static Plan,
    verify_n: [u8; N],
    release_n: [u8; N],
    booted: [bool; N],
    seen: heapless::Vec<Seen, 48>,
    verify_cancels: u32,
}

impl Exec {
    fn begin(&mut self, plan: &'static Plan) {
        self.wd = BootWatchdogs::new();
        self.plan = plan;
        self.verify_n = [0; N];
        self.release_n = [0; N];
        self.booted = [false; N];
        self.seen.clear();
        self.verify_cancels = 0;
    }

    fn push(&mut self, lane: usize, cmd: Cmd) -> core::result::Result<(), EffectError> {
        self.lanes
            .get_mut(lane)
            .ok_or(EffectError)?
            .queue
            .push_back(cmd)
            .map_err(|_| EffectError)
    }

    fn bump(counter: &mut [u8; N], id: u8) -> u8 {
        match counter.get_mut(usize::from(id)) {
            Some(n) => {
                let was = *n;
                *n = n.saturating_add(1);
                was
            }
            None => 0,
        }
    }

    fn mark(&mut self, id: u8, booted: bool) {
        if let Some(b) = self.booted.get_mut(usize::from(id)) {
            *b = booted;
        }
    }

    /// Turn one effect into a command. Never blocks and never calls a
    /// service: the answer comes back later as an event.
    fn effect(&mut self, effect: Effect) -> core::result::Result<Option<Event>, EffectError> {
        match effect {
            Effect::VerifyFirmware(id) => {
                let id = id.get();
                let attempt = Self::bump(&mut self.verify_n, id);
                let units = pick(&self.plan.verify, id, attempt);
                self.push(VERIFIER, Cmd::Verify { id, units })?;
            }
            Effect::ReleaseReset(id) => {
                let id = id.get();
                let attempt = Self::bump(&mut self.release_n, id);
                let units = pick(&self.plan.boot, id, attempt);
                self.mark(id, false);
                self.push(RESET, Cmd::Release { id, units })?;
                self.wd
                    .arm_boot(ComponentId::new(id), BOOT_WINDOW)
                    .map_err(|_| EffectError)?;
            }
            Effect::AssertReset(id) => {
                let id = id.get();
                self.mark(id, false);
                self.wd.cancel_boot(ComponentId::new(id));
                self.push(RESET, Cmd::Assert { id })?;
            }
            Effect::RecoverComponent { id, .. } => {
                let id = id.get();
                self.mark(id, false);
                self.push(RESET, Cmd::Restore { id })?;
            }
            // Local, nothing to ask a service for.
            Effect::ReadFirmware(_) | Effect::CommitSvnFloor(_) => {}
            Effect::ReportIsolated(_)
            | Effect::ReportRecoveryFailed(_)
            | Effect::ReportBootFailed { .. }
            | Effect::ReportUpdateDeferred
            | Effect::ReportUpdateAborted => {}
            // Not exercised by this simulation.
            Effect::SignAttestation
            | Effect::AuthenticateUpdate
            | Effect::StageUpdate
            | Effect::ActivateUpdate
            | Effect::DiscardStaged
            | Effect::LatchLockdown => {}
            Effect::Emit(_) => return Err(EffectError),
        }
        Ok(None)
    }

    fn record(&mut self, event: Event) {
        let seen = match event {
            Event::Timeout(id) => {
                pw_log::info!("EV timeout {}", id.get() as u32);
                Seen::Timeout(id.get())
            }
            Event::VerificationPassed(id) => {
                pw_log::info!("EV passed {}", id.get() as u32);
                Seen::Passed(id.get())
            }
            Event::VerificationFailed(id) => {
                pw_log::info!("EV failed {}", id.get() as u32);
                Seen::Failed(id.get())
            }
            Event::Booted(id) => {
                pw_log::info!("EV booted {}", id.get() as u32);
                Seen::Booted(id.get())
            }
            Event::Restored(id) => {
                pw_log::info!("EV restored {}", id.get() as u32);
                Seen::Restored(id.get())
            }
            _ => return,
        };
        let _ = self.seen.push(seen);
    }

    /// Start whatever can start. The events lane always keeps a long-poll
    /// queued.
    fn pump(&mut self) -> Result<()> {
        if let Some(events) = self.lanes.get_mut(EVENTS)
            && events.idle()
        {
            let _ = events.queue.push_back(Cmd::WaitBoot);
        }
        for lane in self.lanes.iter_mut() {
            lane.start_next()?;
        }
        Ok(())
    }

    /// Decode a lane's response into an event for the core, if it has one.
    fn service(&mut self, lane: usize) -> Result<Option<Event>> {
        let Some(l) = self.lanes.get_mut(lane) else {
            return Ok(None);
        };
        let Some((cmd, resp)) = l.finish()? else {
            return Ok(None);
        };
        let ok = resp[0] == 0;
        Ok(match cmd {
            Cmd::Verify { id, .. } if ok => Some(Event::VerificationPassed(ComponentId::new(id))),
            Cmd::Verify { id, .. } => Some(Event::VerificationFailed(ComponentId::new(id))),
            Cmd::Restore { id } if ok => Some(Event::Restored(ComponentId::new(id))),
            Cmd::Release { .. } | Cmd::Assert { .. } | Cmd::Restore { .. } if !ok => {
                return Err(Error::Internal);
            }
            Cmd::Release { .. } | Cmd::Assert { .. } => None,
            Cmd::WaitBoot if ok => {
                let id = resp[1];
                self.mark(id, true);
                self.wd.cancel_boot(ComponentId::new(id));
                Some(Event::Booted(ComponentId::new(id)))
            }
            Cmd::WaitBoot => None,
            Cmd::Restore { .. } => None,
        })
    }

    /// Give up on a verify that has run past its deadline. The unknown
    /// outcome counts as a failed verification.
    fn expire_verify(&mut self) -> Result<Option<Event>> {
        let now = SystemClock::now();
        let Some(lane) = self.lanes.get_mut(VERIFIER) else {
            return Ok(None);
        };
        let Some(inflight) = lane.inflight else {
            return Ok(None);
        };
        if now < inflight.deadline {
            return Ok(None);
        }
        let _ = lane.cancel()?;
        self.verify_cancels += 1;
        Ok(match inflight.cmd {
            Cmd::Verify { id, .. } => Some(Event::VerificationFailed(ComponentId::new(id))),
            _ => None,
        })
    }

    /// The earliest moment the loop must wake without a response.
    fn next_deadline(&self) -> Instant {
        let mut at = self.wd.wait_deadline();
        if let Some(inflight) = self.lanes.get(VERIFIER).and_then(|l| l.inflight)
            && inflight.deadline < at
        {
            at = inflight.deadline;
        }
        at
    }

    /// Nothing but the long-poll is outstanding.
    fn quiescent(&self) -> bool {
        self.lanes
            .iter()
            .take(EVENTS)
            .all(|lane| lane.idle())
    }

    fn all_booted(&self) -> bool {
        self.booted.iter().all(|b| *b)
    }

    fn first(&self, what: Seen) -> Option<usize> {
        self.seen.iter().position(|s| *s == what)
    }
}

fn deliver(core: &mut Core, exec: &mut Exec, event: Event) {
    exec.record(event);
    core.dispatch_with(event, |effect| exec.effect(effect));
}

fn new_core() -> Result<Core> {
    let mut entries = heapless::Vec::<(ComponentId, ComponentAttrs), N>::new();
    for id in 0..N as u8 {
        entries
            .push((ComponentId::new(id), ComponentAttrs::passive_required()))
            .map_err(|_| Error::ResourceExhausted)?;
    }
    let chain: Chain<N> = entries.try_into().map_err(|_| Error::Internal)?;
    Ok(Orchestrator::new(chain, MAX_RETRY))
}

/// Run one scenario to `Ready` with every device booted.
fn run_scenario(exec: &mut Exec, plan: &'static Plan) -> Result<Core> {
    exec.begin(plan);
    let mut core = new_core()?;
    let start = SystemClock::now();

    deliver(
        &mut core,
        exec,
        Event::PowerGood(PowerOnResult::Provisioned),
    );

    loop {
        exec.pump()?;

        if core.state() == State::Locked {
            pw_log::error!("scenario: core latched Locked");
            return Err(Error::Internal);
        }
        if core.state() == State::Ready && exec.quiescent() && exec.all_booted() {
            return Ok(core);
        }
        if SystemClock::now() - start > SCENARIO_LIMIT {
            pw_log::error!("scenario: timed out");
            return Err(Error::DeadlineExceeded);
        }

        let limit = start
            .checked_add_duration(SCENARIO_LIMIT)
            .unwrap_or(Instant::MAX);
        let deadline = if exec.next_deadline() < limit {
            exec.next_deadline()
        } else {
            limit
        };
        match syscall::object_wait(handle::WG, Signals::READABLE, deadline) {
            Ok(wake) => {
                if let Some(event) = exec.service(wake.user_data)? {
                    deliver(&mut core, exec, event);
                }
            }
            Err(Error::DeadlineExceeded) => {}
            Err(e) => return Err(e),
        }
        while let Some(event) = exec.wd.poll_expired() {
            deliver(&mut core, exec, event);
        }
        if let Some(event) = exec.expire_verify()? {
            deliver(&mut core, exec, event);
        }
    }
}

/// C0 never boots the first time and C1 verifies slowly. C0's watchdog has
/// to fire during C1's verify, so the core is told about the timeout before
/// C1's verdict, which then arrives for a component the re-walk has moved
/// past.
fn scenario_a(exec: &mut Exec) -> Result<()> {
    pw_log::info!("scenario A: flaky boot beside a slow verify");
    run_scenario(exec, &PLAN_A)?;

    let timeout = exec.first(Seen::Timeout(0));
    let late_verdict = exec.first(Seen::Passed(1));
    match (timeout, late_verdict) {
        (Some(t), Some(v)) if t < v => {}
        _ => {
            pw_log::error!("A: C0 timeout did not precede C1's verdict");
            return Err(Error::Internal);
        }
    }
    if exec.first(Seen::Restored(0)).is_none() {
        pw_log::error!("A: C0 was never restored");
        return Err(Error::Internal);
    }
    if exec.verify_cancels != 0 {
        pw_log::error!("A: a verify was abandoned unexpectedly");
        return Err(Error::Internal);
    }
    Ok(())
}

/// A verify outlives its deadline: abandon it, fail the component, recover,
/// and finish.
fn scenario_b(exec: &mut Exec) -> Result<()> {
    pw_log::info!("scenario B: verify outlives its deadline");
    run_scenario(exec, &PLAN_B)?;

    if exec.verify_cancels != 1 {
        pw_log::error!("B: expected exactly one abandoned verify");
        return Err(Error::Internal);
    }
    if exec.first(Seen::Failed(2)).is_none() || exec.first(Seen::Restored(2)).is_none() {
        pw_log::error!("B: C2 was not failed and restored");
        return Err(Error::Internal);
    }
    Ok(())
}

static mut BUFS: [[u8; 8]; 6] = [[0; 8]; 6];

/// # Safety
/// Called once. Hands out each buffer exactly once.
unsafe fn lanes() -> [Lane; 3] {
    // Safety: see function doc; `BUFS` is not touched anywhere else.
    let bufs: &'static mut [[u8; 8]; 6] = unsafe { &mut *core::ptr::addr_of_mut!(BUFS) };
    let (a, rest) = bufs.split_at_mut(2);
    let (b, c) = rest.split_at_mut(2);
    let make = |index: usize, handle: u32, pair: &'static mut [[u8; 8]]| {
        let (send, recv) = pair.split_at_mut(1);
        let send: &'static mut [u8] = &mut send[0];
        let recv: &'static mut [u8] = &mut recv[0];
        Lane::new(
            index,
            AsyncChannelTransport::new(IpcHandle::new(handle), send, recv),
        )
    };
    [
        make(VERIFIER, handle::VERIFIER, a),
        make(RESET, handle::RESET, b),
        make(EVENTS, handle::EVENTS, c),
    ]
}

/// How many times each scenario runs. Services and lanes persist, so a
/// leftover from one run (a parked reply, a stale boot) shows up in the next.
const ROUNDS: u32 = 3;

fn run_all() -> Result<()> {
    // Safety: called once.
    let mut exec = Exec {
        lanes: unsafe { lanes() },
        wd: BootWatchdogs::new(),
        plan: &PLAN_A,
        verify_n: [0; N],
        release_n: [0; N],
        booted: [false; N],
        seen: heapless::Vec::new(),
        verify_cancels: 0,
    };
    for round in 0..ROUNDS {
        if let Err(e) = scenario_a(&mut exec).and_then(|_| scenario_b(&mut exec)) {
            pw_log::error!("failed in round {}", round as u32);
            return Err(e);
        }
    }
    Ok(())
}

#[entry]
fn entry() {
    let ret = run_all();
    match &ret {
        Ok(()) => pw_log::info!("All scenarios PASSED"),
        Err(e) => pw_log::error!("FAILED: status code {}", *e as u32),
    }
    let _ = syscall::debug_shutdown(ret);
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    let _ = userspace::syscall::debug_shutdown(Err(pw_status::Error::Internal));
    loop {}
}
