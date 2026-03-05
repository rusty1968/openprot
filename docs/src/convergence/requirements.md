# Caliptra MCU Async/Await Runtime — Requirements

Requirements for the async/await runtime supporting caliptra-mcu-sw userspace applications.

---

## 1. Scope

This document defines kernel-agnostic requirements for the async runtime that enables Rust `async`/`await` in caliptra-mcu-sw userspace processes. The runtime bridges an underlying kernel's event-delivery mechanism with Rust's cooperative `Future`-based concurrency model.

The current implementation targets Tock kernel, but these requirements are written to be portable to other embedded operating systems or bare-metal environments.

### 1.1 In Scope

- Task executor lifecycle and scheduling
- Future primitives for asynchronous I/O completion
- kernel abstraction layer for event delivery and process suspension
- Memory allocation strategy for async state
- Synchronization primitives for concurrent tasks
- Safety invariants for kernel/userspace and hardware boundaries

### 1.2 Out of Scope

- kernel internals and kernel-side scheduling
- Application-layer protocol logic (PLDM, SPDM, MCTP message semantics)
- Hardware-specific driver behavior beyond the async I/O interface

---

## 2. Platform Constraints

### PLAT-1: `no_std` Environment

The runtime SHALL operate in a `no_std` environment on embedded targets. A `std`-compatible mode MAY be provided for host-side testing.

### PLAT-2: Single-Threaded Execution

The runtime SHALL assume single-threaded, single-core execution. The kernel SHALL NOT re-enter the process except at explicit yield points controlled by the runtime.

### PLAT-3: Primary Target Architecture

The runtime SHALL target RISC-V 32-bit (riscv32imc and variants) as the primary architecture. Platform-specific code (yield mechanism, atomics) SHALL be isolated behind a platform abstraction layer.

### PLAT-4: Cooperative Scheduling

The runtime SHALL use cooperative (non-preemptive) task scheduling. Tasks yield control voluntarily at `.await` points. The kernel MAY preempt the entire process at yield boundaries but SHALL NOT preempt individual async tasks.

### PLAT-5: Constrained Resources

The runtime SHALL be designed for resource-constrained embedded MCUs with limited RAM, no MMU, and no virtual memory. Memory allocation strategies SHALL account for heap fragmentation and bounded memory usage.

---

## 3. Executor Requirements

### EXEC-1: Single Executor Instance

The runtime SHALL support at most one active executor per process.

This is required because the waker signal path uses a process-global flag (the `__pender` callback and associated atomic). If two executor instances existed simultaneously, both would share that flag and each would steal wake signals meant for the other, silently dropping tasks.

Enforcement is structural: the primary entry point (`start_async`) diverges (`-> !`), so the call site can never return to construct a second instance. The `!Send` marker prevents the executor from being moved to another thread, but does not by itself prevent multiple stack-allocated instances within the same thread.

### EXEC-2: Executor Lifecycle

The runtime SHALL provide entry points to:
1. Create an executor, spawn an initial task, and run forever (diverging)
2. Spawn a task onto an already-running executor

The primary entry point SHALL NOT return.

### EXEC-3: Poll Loop

The executor SHALL implement a poll loop with the following behavior:

1. Poll all ready tasks
2. If new work was signaled during polling, loop back immediately
3. If no work is pending, suspend the process until the kernel delivers an event

The executor SHALL NOT busy-wait or spin when no tasks are ready.

### EXEC-4: Waker Integration

The executor SHALL integrate with Rust's standard `core::task::Waker` mechanism. When a waker is invoked (e.g., from a kernel event callback or interrupt context), the executor SHALL be signaled to re-poll. The signaling mechanism SHALL be safe to invoke from callback/interrupt context.

### EXEC-5: Task Spawning

The spawning handle SHALL be `Copy` so tasks can freely distribute it to sub-tasks without consuming or cloning it.

### EXEC-6: Bounded Task Count

The maximum number of concurrently spawned tasks SHALL be bounded at compile time. The runtime SHALL NOT support unbounded dynamic task creation.

---

## 4. Asynchronous I/O Future Requirements

### IO-1: Event Completion Future

The runtime SHALL provide a future type that completes when a kernel or hardware event is delivered. This future SHALL implement `Future<Output = Result<T, E>>` where:
- `T` carries the event payload (e.g., status codes, byte counts)
- `E` represents kernel or driver error codes

### IO-2: Buffer Sharing

The I/O future layer SHALL support sharing memory buffers with the kernel or hardware for zero-copy I/O:
- Mutable buffers for read operations (kernel writes into application memory)
- Immutable buffers for write operations (kernel reads from application memory)
- Combined read + write buffer operations

Buffer lifetime SHALL be tied to the future's lifetime to prevent use-after-free.

### IO-3: Pinning and Stability

If the kernel or hardware requires a stable pointer to future state (e.g., for callback data), the future SHALL be pinned in memory. The runtime SHALL enforce pinning at the API level.

### IO-4: Drop Safety

If an I/O future has registered a callback or shared a pointer with the kernel, it SHALL NOT be silently dropped while the kernel still holds a reference. The runtime SHALL enforce this via one or more of:
- Panicking on premature drop
- Requiring explicit cancellation before drop
- Automatically deregistering callbacks on drop (if the kernel supports it)

### IO-5: Cancellation

The runtime SHALL provide a mechanism to cancel an in-progress I/O future, allowing it to be safely dropped without panic. Cancellation SHALL deregister or invalidate any outstanding kernel callbacks.

### IO-6: Error Propagation

If the kernel rejects an I/O request at submission time, the future SHALL resolve to an error immediately on the next poll, without waiting for an event that will never arrive.

### IO-7: Callback-to-Waker Bridge

The runtime SHALL bridge the kernel's event notification mechanism (callbacks, interrupts, completion queues, etc.) to Rust's `Waker` system. When an event fires:
1. The event payload SHALL be stored in the future's state
2. The associated `Waker` SHALL be invoked to signal the executor

This bridge code SHALL be safe to execute in callback or interrupt context.

---

## 5. Kernel Abstraction Layer

### KERN-1: Pluggable kernel Backend

The runtime SHALL isolate all kernel-specific code behind an abstraction boundary. Porting to a new kernel SHALL require implementing this boundary without modifying the executor or consumer code.

### KERN-2: Required kernel Capabilities

The kernel backend SHALL provide:

| Capability | Description |
|---|---|
| Event registration | Register interest in a driver/hardware event with a callback |
| Buffer sharing | Share application memory with kernel drivers for I/O |
| Process suspension | Suspend the process until an event is delivered |
| Event delivery | Invoke a callback or signal when an event completes |

### KERN-3: Yield / Suspend

The kernel backend SHALL provide a mechanism to suspend the process when no async work is pending. This SHALL:
- Release the CPU to the kernel scheduler or idle loop
- Resume the process when an event is ready for delivery
- Be implementable via architecture-specific instructions (e.g., `ecall` on RISC-V, `svc` on ARM)

### KERN-4: Host-Side Testing Backend

A test backend SHALL be provided that simulates kernel event delivery on host platforms (x86_64, aarch64), enabling unit testing of async code without real hardware or kernel.

---

## 6. Synchronization

### SYNC-1: Critical Section

The runtime SHALL provide a critical section implementation appropriate for the execution model. For single-threaded cooperative runtimes, a no-op critical section is sufficient and sound.

### SYNC-2: Async Mutex

The runtime ecosystem SHALL support async-aware mutexes for serializing access to shared resources across concurrent tasks. Contended lock attempts SHALL yield (return `Pending`) rather than spinning.

### SYNC-3: Async Signals

The runtime ecosystem SHALL support one-shot or multi-shot async notification primitives for inter-task coordination (e.g., signaling a background task to start work).

### SYNC-4: Portable Atomics

The runtime SHALL use portable atomic operations that work on targets without native atomic instructions (e.g., single-core RISC-V without the A extension).

---

## 7. Memory Requirements

### MEM-1: Heap Allocation for I/O Futures

I/O future instances that must be pinned for kernel callback stability SHALL be heap-allocated. A global allocator suitable for embedded targets (e.g., `embedded-alloc`) SHALL be available.

### MEM-2: Minimal Executor Allocation

The executor itself SHALL NOT require heap allocation. It MAY be stack-allocated or placed in static memory.

### MEM-3: Static Synchronization State

Shared synchronization primitives (mutexes, signals) SHALL support `'static` placement with lazy or compile-time initialization. They SHALL NOT require heap allocation.

### MEM-4: Bounded Memory Usage

The runtime's memory footprint SHALL be bounded and predictable:
- Task count bounded at compile time (EXEC-6)
- No unbounded queues or dynamic collections in the executor
- I/O future allocations are individually bounded in size

---

## 8. Safety Requirements

### SAFE-1: No Use-After-Free for kernel Callbacks

The runtime SHALL prevent the kernel from invoking a callback into freed memory. This SHALL be enforced by pinning, drop guards, and/or explicit cancellation (see IO-3, IO-4, IO-5).

### SAFE-2: Sound Lifetime Extension

Any `'static` lifetime extension of the executor SHALL only be used when the executor genuinely lives for the duration of the process. This SHALL be documented with safety comments and structurally limited to diverging entry points.

### SAFE-3: No Data Races

The combination of single-threaded execution (PLAT-2), cooperative scheduling (PLAT-4), and appropriate critical sections (SYNC-1) SHALL guarantee freedom from data races within userspace.

### SAFE-4: Panic Safety in Callbacks

kernel event callbacks SHALL use RAII guards to ensure consistent state if a panic occurs during callback execution. The guard SHALL prevent the process from continuing with corrupted state.

---

## 9. Testability Requirements

### TEST-1: Host-Side Compilation

The runtime SHALL compile and run on host platforms (x86_64, aarch64) for unit testing. All target-specific code SHALL be gated behind platform conditionals.

### TEST-2: Simulated kernel Backend

A simulated kernel backend (KERN-4) SHALL allow tests to:
- Trigger event delivery programmatically
- Verify future polling and waker behavior
- Test error paths and cancellation

### TEST-3: Mockable kernel Interface

The kernel abstraction (KERN-1) SHALL be parameterized or trait-based so that test doubles can be injected for all kernel interactions.

---

## 10. Downstream Consumer Requirements

Assumption for this section: hardware drivers are implemented in pw-kernel (kernel space). Userspace async code interacts with kernel-exposed objects/services rather than owning direct driver implementations.

### DOWN-1: Driver Integration Pattern

The runtime SHALL support a standard async kernel-service integration pattern:

1. Initiate an I/O operation (register event interest, optionally share buffers)
2. Trigger the operation via the kernel/hardware
3. `.await` the I/O future to receive the completion result
4. Process the result or propagate the error

Userspace service wrappers around kernel objects SHALL compose naturally with `async`/`await` syntax and the `?` operator.

### DOWN-2: Supported Kernel-Backed I/O Classes

The runtime SHALL enable async wrappers for the following kernel-backed caliptra-mcu-sw I/O classes:

| I/O Class | Operations |
|---|---|
| Flash storage | Read, write (chunked, with shared buffers) |
| DMA | Asynchronous memory transfers |
| Mailbox / command interface | Send command, receive response (with mutual exclusion) |
| Message transport (MCTP) | Receive request, send response |
| Data obfuscation (DOE) | Receive message, send message |
| Logging | Asynchronous log output |
| Timers / alarms | Async delay, periodic wakeup |

### DOWN-3: Long-Running Service Support

The runtime SHALL support long-running daemon-style tasks that:
- Spawn multiple concurrent sub-tasks (responder, initiator, periodic)
- Run indefinitely in a service loop
- Coordinate via async mutexes and signals

Examples: PLDM firmware update daemon, SPDM responder, MCTP-VDM daemon.

### DOWN-4: Dynamic Dispatch Compatibility

The runtime SHALL be compatible with async trait objects (`dyn Future`, `#[async_trait(?Send)]`) for use cases requiring dynamic dispatch (e.g., pluggable image loader implementations).

---

## 11. Non-Functional Requirements

### NFR-1: Minimal Overhead

The executor poll loop SHALL add negligible overhead. The runtime SHALL NOT introduce unnecessary allocations, copies, or indirection in the critical path between event delivery and task resumption.

### NFR-2: Deterministic Wake Behavior

When a kernel event fires, the corresponding task SHALL be woken and polled in the immediately following executor poll cycle. There SHALL be no spurious wakes that cause unnecessary polling.

### NFR-3: Power Efficiency

When no tasks are ready, the runtime SHALL suspend the process via the kernel (KERN-3), allowing the system to idle the processor or schedule other work. The runtime SHALL NOT busy-wait.

### NFR-4: Code Size

The runtime SHALL minimize code size. Generic/monomorphized code SHALL be limited to what is necessary for type safety. Shared logic SHALL be factored into non-generic helpers where practical.

### NFR-5: Licensing

All runtime code SHALL be licensed under Apache-2.0, consistent with the caliptra-mcu-sw project. Third-party dependencies SHALL have compatible licenses (Apache-2.0, MIT, or BSD).

---

## 12. Requirement Ownership (Executor vs Reactor)

This section assigns each requirement to the primary implementation owner.

- `Executor`: task scheduling, polling, spawning, and top-level runtime lifecycle
- `Reactor`: kernel event integration, wait-group multiplexing, and I/O futures
- `Shared`: cross-cutting contract that both layers must satisfy

| Requirement | Primary Owner | Notes |
|---|---|---|
| PLAT-1 | Shared | Both crates are `no_std`. |
| PLAT-2 | Shared | Single-threaded cooperative assumption is system-wide. |
| PLAT-3 | Shared | Platform abstractions affect both layers. |
| PLAT-4 | Executor | Cooperative scheduling policy is executor behavior. |
| PLAT-5 | Shared | Memory/resource constraints apply to all runtime pieces. |
| EXEC-1 | Executor | Singleton executor instance and lifecycle constraints. |
| EXEC-2 | Executor | Entry points and diverging runtime loop. |
| EXEC-3 | Executor + Reactor | Executor runs poll loop; Reactor should provide blocking idle path. |
| EXEC-4 | Executor | Core waker-to-repoll integration via pender/signal path. |
| EXEC-5 | Executor | Spawner model and dynamic task creation. |
| EXEC-6 | Executor | Bounded task count is an executor-level capacity contract. |
| IO-1 | Reactor | Event-completion futures and result modeling. |
| IO-2 | Reactor | Buffer-sharing futures and lifetime management. |
| IO-3 | Reactor | Pinning/stable pointer guarantees for callback state. |
| IO-4 | Reactor | Drop safety of in-flight I/O registrations. |
| IO-5 | Reactor | Explicit cancellation semantics for in-progress I/O. |
| IO-6 | Reactor | Submission-time error propagation from kernel/backend. |
| IO-7 | Reactor | Callback/interrupt event bridge to waker signaling. |
| KERN-1 | Reactor | kernel abstraction boundary is owned by reactor/kernel bridge layer. |
| KERN-2 | Reactor | Event registration, buffer sharing, suspension, delivery APIs. |
| KERN-3 | Reactor | Suspend/wait capability exposed for executor idle strategy. |
| KERN-4 | Reactor | Simulated backend for host tests of I/O/event behavior. |
| SYNC-1 | Shared | Critical-section contract spans executor signal path + reactor state. |
| SYNC-2 | Shared | Async mutexes are ecosystem-level primitives used by tasks. |
| SYNC-3 | Shared | Async signaling primitives are cross-cutting. |
| SYNC-4 | Executor | Portable atomics are core to executor wake signaling. |
| MEM-1 | Reactor | Pinned I/O futures and callback-associated state ownership. |
| MEM-2 | Executor | Executor should remain heap-free. |
| MEM-3 | Shared | Static sync primitives used across runtime layers. |
| MEM-4 | Shared | Bounded memory is a full-runtime invariant. |
| SAFE-1 | Reactor | Callback lifetime safety is primarily in I/O/event layer. |
| SAFE-2 | Executor | `'static` lifetime extension pattern is in executor startup path. |
| SAFE-3 | Shared | Race-freedom depends on combined model and invariants. |
| SAFE-4 | Reactor | Callback guard behavior belongs to event/callback bridge. |
| TEST-1 | Shared | Host compile/test support spans both crates. |
| TEST-2 | Reactor | Simulated event backend is reactor/backend concern. |
| TEST-3 | Reactor | Mockable kernel interface belongs to kernel abstraction layer. |
| DOWN-1 | Reactor + Executor | Userspace tasks await kernel-service futures; executor schedules those tasks. |

---

## Appendix A: Current Implementation Mapping

The current implementation targets Tock kernel. This appendix maps abstract requirements to Tock-specific mechanisms for reference.

| Requirement | Tock Implementation |
|---|---|
| KERN-2: Event registration | `SUBSCRIBE` syscall with upcall function pointer |
| KERN-2: Buffer sharing | `ALLOW_RW` / `ALLOW_RO` syscalls |
| KERN-3: Process suspension | `yield-wait` syscall (class 0, id 1) |
| KERN-2: Event delivery | Kernel invokes registered upcall with 3 × u32 args |
| IO-1: Event completion future | `TockSubscribe` type |
| IO-7: Callback-to-waker bridge | `extern "C" fn kernel_upcall` stores result, wakes `Waker` |
| SAFE-4: Panic safety | `ExitOnDrop<S>` RAII guard in upcall |
| KERN-4: Test backend | `libtock_unittest::fake::Syscalls` |
| EXEC-2: Executor | `TockExecutor` wrapping Embassy `raw::Executor` |
| SYNC-1: Critical section | `NullCriticalSection` (no-op, sound for single-threaded Tock) |
| KERN-3: Yield (RISC-V) | Inline `asm!("ecall")` with full register clobber |
| MEM-1: Allocator | `embedded-alloc` global allocator |


