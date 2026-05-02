1. Executive conclusion  
The workspace clearly configures AST1030/AST1060 platforms for multiple flash controllers and chip-select endpoints (FMC, SPI1, SPI2, plus CS nodes), and enables multi-device SPI NOR usage.  
At the application/HAL layer, flash access is routed by a device-id-to-device-name map into per-endpoint Zephyr devices (for example `spi1@0`, `spi1@1`, `spi2@0`, `spi2@1`, `fmc@0`, `fmc@1`), then executed synchronously with direct `flash_read`, `flash_write`, and `spi_nor_erase_by_cmd` calls.  
I did not find in-repo evidence of async pending-request ownership objects, per-controller in-flight tracking, or IRQ-completion re-dispatch for flash read/write/erase.  
The only IRQ-defer-complete pattern found is in SPI monitor logging: ISR callback identifies controller index and defers parsing via `k_work`.  
Driver instantiation/ISR/runtime internals for the underlying SPI NOR and `pfr_aspeed` drivers are not in this repo; they are pulled from external Zephyr/BSP.  
Based on repo evidence, the effective behavior here is serialized synchronous calls with limited lock-based protection (DMA buffer mutex), not explicit multi-channel async scheduling in this codebase.

2. Evidence table

| Claim | Proof (file + symbol + snippet) | Confidence |
|---|---|---|
| FMC/SPI1/SPI2 and CS endpoints are explicitly defined for AST1030 | [apps/spdm/boards/ast1030_evb.overlay#L1](apps/spdm/boards/ast1030_evb.overlay#L1) symbol: fmc node, snippet: "&fmc {"; [apps/spdm/boards/ast1030_evb.overlay#L31](apps/spdm/boards/ast1030_evb.overlay#L31) symbol: spi1 node, snippet: "&spi1 {"; [apps/spdm/boards/ast1030_evb.overlay#L47](apps/spdm/boards/ast1030_evb.overlay#L47) symbol: spi2 node, snippet: "&spi2 {" | High |
| AST1060-style boards map CS endpoints to specific SPI monitor controllers | [apps/aspeed-pfr/boards/ast1060_prot.overlay#L189](apps/aspeed-pfr/boards/ast1060_prot.overlay#L189) symbol: spi1_cs0 property, snippet: "spi-monitor-ctrl = <&spim1>;"; [apps/aspeed-pfr/boards/ast1060_prot.overlay#L196](apps/aspeed-pfr/boards/ast1060_prot.overlay#L196), [apps/aspeed-pfr/boards/ast1060_prot.overlay#L203](apps/aspeed-pfr/boards/ast1060_prot.overlay#L203), [apps/aspeed-pfr/boards/ast1060_prot.overlay#L210](apps/aspeed-pfr/boards/ast1060_prot.overlay#L210) | High |
| AST1060 overlay encodes multi-CS topology per SPI controller | [apps/aspeed-pfr/boards/ast1060_prot.overlay#L176](apps/aspeed-pfr/boards/ast1060_prot.overlay#L176) symbol: spi1 num-cs, snippet: "num-cs = <2>;"; [apps/aspeed-pfr/boards/ast1060_prot.overlay#L220](apps/aspeed-pfr/boards/ast1060_prot.overlay#L220) symbol: spi2 num-cs, snippet: "num-cs = <2>;" | High |
| Project configs enable multi-device SPI NOR model | [apps/aspeed-pfr/prj.conf#L16](apps/aspeed-pfr/prj.conf#L16), [apps/preload-fw/prj.conf#L18](apps/preload-fw/prj.conf#L18), [apps/mcu-runtime/prj.conf#L12](apps/mcu-runtime/prj.conf#L12), [apps/spdm/boards/ast1030_evb.conf#L10](apps/spdm/boards/ast1030_evb.conf#L10) symbol: CONFIG_SPI_NOR_MULTI_DEV, snippet: "CONFIG_SPI_NOR_MULTI_DEV=y" | High |
| App-side routing is device-name based, one target selected per call | [lib/hrot_hal/flash/flash_aspeed.c#L25](lib/hrot_hal/flash/flash_aspeed.c#L25) symbol: Flash_Devices_List, snippet includes spi1@0/spi1@1/spi2@0/spi2@1/fmc@0/fmc@1; [lib/hrot_hal/flash/flash_aspeed.c#L171](lib/hrot_hal/flash/flash_aspeed.c#L171) symbol: get_flash_dev | High |
| Device identity/channel is represented by numeric device_id enum, not async request object | [lib/hrot_hal/flash/flash_aspeed.h#L29](lib/hrot_hal/flash/flash_aspeed.h#L29) symbol: device enum, snippet: "BMC_SPI...PCH_SPI...ROT_INTERNAL_ACTIVE..."; [lib/hrot_hal/flash/flash_aspeed.c#L159](lib/hrot_hal/flash/flash_aspeed.c#L159) symbol: SPI_Command_Xfer, snippet routes by DeviceId | High |
| Read/write path is synchronous blocking | [lib/hrot_hal/flash/flash_aspeed.c#L279](lib/hrot_hal/flash/flash_aspeed.c#L279) symbol: bmc_pch_flash_read, snippet: "ret = flash_read(...)"; [lib/hrot_hal/flash/flash_aspeed.c#L341](lib/hrot_hal/flash/flash_aspeed.c#L341) symbol: bmc_pch_flash_write, snippet: "ret = flash_write(...)" | High |
| Erase path is synchronous command call, return code propagated | [lib/hrot_hal/flash/flash_aspeed.c#L402](lib/hrot_hal/flash/flash_aspeed.c#L402), [lib/hrot_hal/flash/flash_aspeed.c#L407](lib/hrot_hal/flash/flash_aspeed.c#L407) symbol: bmc_pch_flash_erase, snippet: "ret = spi_nor_erase_by_cmd(...)" | High |
| Serialization in this layer is a shared mutex for DMA staging buffer, with timeout | [lib/hrot_hal/flash/flash_aspeed.c#L35](lib/hrot_hal/flash/flash_aspeed.c#L35) symbol: flash_rw_mutex; [lib/hrot_hal/flash/flash_aspeed.c#L281](lib/hrot_hal/flash/flash_aspeed.c#L281) snippet: "k_mutex_lock(..., K_MSEC(1000))" | High |
| In-repo flash path has no visible IRQ/pending/completion orchestration | [lib/hrot_hal/flash/flash_aspeed.c](lib/hrot_hal/flash/flash_aspeed.c) searched for pending/k_sem/k_msgq/k_work/irq/isr/completion and found only mutex usage; no callback ownership structure in this file | Medium |
| IRQ-deferred completion exists for SPI monitor logging (not flash read/write/erase request completion) | [apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L113](apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L113) symbol: demo_spim_isr_callback; [apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L117](apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L117) snippet: "k_work_submit(&log_ctrls[ctrl_idx - 1].log_work);"; [apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L140](apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L140) symbol: spim_isr_callback_install | High |
| Low-level driver instantiation/ISR internals are external to this repo | [README.md#L3](README.md#L3) snippet: developing on top of Zephyr BSP; [west.yml#L13](west.yml#L13) symbol: external zephyr project; no in-repo spi_nor*.c or pfr_aspeed*.c files found | High |

3. Sequence summary: request -> defer -> IRQ -> completion

Flash read/write/erase path in this repo:
1. Upper wrapper invokes SPI_Command_Xfer (for example [lib/hrot_wrapper/flash/flash_wrapper.c#L171](lib/hrot_wrapper/flash/flash_wrapper.c#L171), [lib/hrot_hal/flash/flash_aspeed.c#L159](lib/hrot_hal/flash/flash_aspeed.c#L159)).  
2. Routing selects endpoint by device_id and device_get_binding against static names (see [lib/hrot_hal/flash/flash_aspeed.c#L25](lib/hrot_hal/flash/flash_aspeed.c#L25), [lib/hrot_hal/flash/flash_aspeed.c#L171](lib/hrot_hal/flash/flash_aspeed.c#L171)).  
3. Operation executes synchronously via flash_read, flash_write, or spi_nor_erase_by_cmd and returns immediately with status ([lib/hrot_hal/flash/flash_aspeed.c#L279](lib/hrot_hal/flash/flash_aspeed.c#L279), [lib/hrot_hal/flash/flash_aspeed.c#L341](lib/hrot_hal/flash/flash_aspeed.c#L341), [lib/hrot_hal/flash/flash_aspeed.c#L402](lib/hrot_hal/flash/flash_aspeed.c#L402)).  
4. Optional lock timeout only guards shared DMA staging buffer path: k_mutex_lock(..., K_MSEC(1000)) ([lib/hrot_hal/flash/flash_aspeed.c#L281](lib/hrot_hal/flash/flash_aspeed.c#L281)).  
5. No in-repo pending request object, no flash ISR callback mapping, no completion wake path found.

SPI monitor logging path (separate from flash data path):
1. Driver callback is installed per SPIM device in init ([apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L140](apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L140)).  
2. ISR callback maps dev -> ctrl_idx and defers work via k_work_submit ([apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L113](apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L113), [apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L117](apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L117)).  
3. Work handler reads log RAM and parses entries ([apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L68](apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L68), [apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L75](apps/aspeed-pfr/src/platform_monitor/spim_monitor.c#L75)).  
4. This is deferred IRQ handling for monitor logs, not request completion for flash read/write/erase callers.

4. Design mapping to your decision

Multi-channel implications
1. Topology is multi-endpoint and can represent multiple chip-select channels per controller in DT ([apps/aspeed-pfr/boards/ast1060_prot.overlay#L176](apps/aspeed-pfr/boards/ast1060_prot.overlay#L176), [apps/aspeed-pfr/boards/ast1060_prot.overlay#L220](apps/aspeed-pfr/boards/ast1060_prot.overlay#L220)).  
2. Existing app/HAL code does not maintain per-request channel context for async completion; it does immediate synchronous calls.  
3. If you introduce async multi-channel-per-controller, you will need new explicit ownership metadata (client id, channel/cs id, callback context, timeout state) that is not present now.

Single-channel implications
1. Current behavior aligns more closely with serialized synchronous access where caller thread blocks and gets return code directly.  
2. The only explicit timeout in this layer is mutex acquisition for shared DMA buffer, not per-operation async completion timeout.  
3. This is simpler and matches current code assumptions.

Which pattern this workspace actually uses
1. Effective observed pattern in this repo: one logical operation in-flight per caller path, synchronous API semantics, plus limited lock-based serialization; no in-repo async channel scheduler for flash ops.  
2. Controller-driver internals (true per-controller IRQ and pending-state behavior) are in external Zephyr/BSP, not auditable from this workspace alone.

Recommendation
Given the repo-backed evidence, prefer one-driver-binary-per-controller with single active channel semantics unless you have a concrete requirement for concurrent multi-client async operations. If you do need concurrency, add explicit multi-channel ownership and completion state machines at the driver boundary; that machinery is absent in the current in-repo architecture.

5. Open gaps

1. Missing source for underlying SPI NOR and ASPEED pfr_aspeed drivers in this workspace, so I cannot prove their internal per-controller instance structs, ISR handlers, pending op structs, or queue model.  
2. Searched in-repo for driver instantiation symbols and implementation files, including patterns like DEVICE_DT_INST_DEFINE, IRQ_CONNECT, DT_INST_FOREACH_STATUS_OKAY, spi_nor*.c, pfr_aspeed*.c; none found in this workspace.  
3. To close the gaps, audit the external Zephyr/BSP source pinned by [west.yml#L13](west.yml#L13) and revision [west.yml#L15](west.yml#L15), specifically the SPI NOR and ASPEED flash/monitor driver files under the imported Zephyr tree.

6. Zephyr extension addendum (`~/work/os/zephyr`)

This section extends the audit with direct evidence from the imported Zephyr tree.

### 6.1 Source selection and topology wiring

- `drivers/flash/CMakeLists.txt`:
	- `zephyr_library_sources_ifdef(CONFIG_SPI_NOR spi_nor.c)`
	- `zephyr_library_sources_ifdef(CONFIG_SPI_NOR_MULTI_DEV spi_nor_multi_dev.c)`

Given this project enables `CONFIG_SPI_NOR_MULTI_DEV=y`, the active backend is `drivers/flash/spi_nor_multi_dev.c`.

- `drivers/flash/spi_nor_multi_dev.c`:
	- `#define DT_DRV_COMPAT jedec_spi_nor`
	- `DT_INST_FOREACH_STATUS_OKAY(SPI_NOR_MULTI_INIT)`

This confirms one Zephyr flash device instance per `jedec,spi-nor` DT instance.

### 6.2 Request ownership and in-flight model (SPI NOR multi-dev)

- Runtime state includes a lock semaphore:
	- `struct spi_nor_data { ... struct k_sem sem; ... }`
- Access is serialized by explicit acquire/release helpers:
	- `acquire_device()` -> `k_sem_take(&driver_data->sem, K_FOREVER)`
	- `release_device()` -> `k_sem_give(&driver_data->sem)`
- Initialization sets semaphore count to 1:
	- `k_sem_init(&driver_data->sem, 1, K_SEM_MAX_LIMIT)`

Implication: one caller at a time per flash device instance; no per-request queue object is used in this driver.

### 6.3 Erase/reset behavior and completion style

- `spi_nor_erase_by_cmd()`:
	- takes device lock via `acquire_device(dev)`
	- loops erase operations and calls `spi_nor_wait_until_ready(dev)` after each
	- releases lock via `release_device(dev)`
- `spi_nor_wait_until_ready()`:
	- polls status register (`SPI_NOR_CMD_RDSR`) until WIP clears
	- uses `k_usleep(1)` poll delay
- `spi_nor_rst_by_cmd()`:
	- serialized by `acquire_device()` / `release_device()`
	- uses busy waits (`k_busy_wait(10 * 1000)`, `k_busy_wait(50 * 1000)`)

Implication: synchronous API over polling, not IRQ-driven completion, at SPI NOR flash layer.

### 6.4 SPI controller driver model (`drivers/spi/spi_aspeed.c`)

- Per-instance runtime state:
	- `struct aspeed_spi_data { struct spi_context ctx; ... }`
- Per-instance device construction:
	- `DEVICE_DT_INST_DEFINE(..., &aspeed_spi_data_##n, &aspeed_spi_config_##n, ...)`
	- `DT_INST_FOREACH_STATUS_OKAY(ASPEED_SPI_INIT)`
- DMA IRQ path:
	- init connects `irq_connect_dynamic(config->irq_num, ..., aspeed_spi_dma_isr, dev, 0)`
	- ISR uses `dev->data` (`struct aspeed_spi_data`) and ends with `spi_context_complete(ctx, dev, 0)`
- Transfer flow:
	- `spi_context_lock(...)`
	- start transfer
	- `spi_context_wait_for_completion(ctx)`
	- `spi_context_release(ctx, ret)`

Implication: completion and ownership are per controller instance via `spi_context`; one active transfer context per instance at a time.

### 6.5 SPI monitor driver IRQ model (`drivers/spi/spi_monitor_aspeed.c`)

- Per-monitor-instance runtime state:
	- `struct aspeed_spim_data` contains `sem_spim`, `irq_ctrl_lock`, `isr_callback`, `log_work`
- Callback registration:
	- `spim_isr_callback_install()` stores callback in instance data
- IRQ dispatch:
	- `spim_isr()` receives `dev`, takes spinlock, invokes `data->isr_callback(dev)`, then ACKs IRQ status bits
- Instance creation:
	- child monitor nodes use `DEVICE_DT_DEFINE(...)`
	- common node uses `DEVICE_DT_INST_DEFINE(...)`
	- expanded by `DT_INST_FOREACH_STATUS_OKAY(ASPEED_SPI_MONITOR_COMMON_INIT)`

Implication: IRQ is mapped to controller context by `dev` and instance-local callback pointer.

### 6.6 DT bindings that shape multi-controller/multi-channel topology

- `dts/bindings/spi/aspeed,spi-controller.yaml`:
	- `num-cs`, `internal-mux-master`, `spi-monitor-output-base`, `spi-monitor-common-ctrl`
- `dts/bindings/mtd/jedec,spi-nor.yaml`:
	- `spi-monitor-ctrl` phandle on each flash node
	- `spi-nor-caps-mask`
- `dts/bindings/spi/aspeed,spi-monitor-controller.yaml`:
	- monitor controller with child monitor nodes and per-node IRQ/property sets

### 6.7 Updated decision mapping (with Zephyr internals)

- Zephyr implementation observed here is **multi-instance per controller/flash node**, with **single in-flight operation per instance** enforced by semaphore/spi_context locking.
- No evidence of per-controller multi-client request queue object in these drivers.
- For your design choice, this strengthens the recommendation:
	- favor **one-driver-binary-per-controller** with **single active channel/in-flight op per instance** semantics,
	- and layer any multi-channel policy above it (or explicitly add queueing/ownership metadata if true concurrent multi-client behavior is required).

### 6.8 Remaining gap after extension

- The above confirms driver behavior in the local `~/work/os/zephyr` tree, but not necessarily every downstream/private fork commit beyond this checkout.
