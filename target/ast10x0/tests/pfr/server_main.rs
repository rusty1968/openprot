// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! PFR software-mailbox server (single process, synchronous read serving).
//!
//! Unlike the generic i2c server-runtime + IPC client split, this app owns the
//! `SwmbxCtrl` mailbox state *in the same process that handles the slave IRQ*.
//! A master read is served inside the IRQ wake — `get_msg` + `write_slave_response`
//! run before the AST controller releases its TX clock-stretch — so there is no
//! cross-process IPC round-trip in the read path. This mirrors aspeed-rust's
//! in-ISR `swmbx_target` callback model and fixes the FIFO/back-to-back read
//! failures the IPC design suffered from.

#![no_main]
#![no_std]

use app_i2c_server::{handle, signals};
use ast10x0_peripherals::i2c::{ClockConfig, I2cConfig, I2cSpeed, I2cXferMode};
use ast10x0_pfr::{
    SwmbxCtrl, SWMBX_BUF_BASE, SWMBX_FIFO, SWMBX_FIFO_NOTIFY_STOP, SWMBX_NODE_COUNT, SWMBX_NOTIFY,
    SWMBX_PROTECT,
};
use i2c_api::seam::{I2cIsrEvent, I2cSlaveBuffer, I2cSlaveCore, I2cSlaveEvent};
use userspace::entry;
use userspace::syscall;
use userspace::time::Duration;

const SLAVE_ADDR: u8 = 0x38;
const RX_BUFFER_SIZE: usize = 256;

const UFM_WRITE_FIFO: u8 = 0x0d;
const UFM_READ_FIFO: u8 = 0x0e;
const SWMBX_WRITE_FIFO_SIZE: usize = 64;
const SWMBX_READ_FIFO_SIZE: usize = 128;
const BMC_UPDATE_INTENT: u8 = 0x13;
const IRQ_POLL_MS: i64 = 1;

// Per-port write-protection bitmaps: 8 packed 32-bit words = 256 mailbox nodes.
// Bit (word*32 + n) set => offset n is write-protected when global PROTECT is
// on. Values mirror aspeed-rust's swmbx access-control tables.
const BMC_ACCESS_CONTROL: [u32; 8] = [
    0xfff7_04ff, 0xffff_ffff, 0xffff_ffff, 0xffff_fff2, 0xffff_ffff, 0xffff_ffff, 0x0000_0000,
    0x0000_0000,
];
const PCH_ACCESS_CONTROL: [u32; 8] = [
    0xfff8_84ff, 0xffff_ffff, 0xffff_ffff, 0xffff_fff5, 0x0000_0000, 0x0000_0000, 0xffff_ffff,
    0xffff_ffff,
];

const SLAVE_CFG: I2cConfig = I2cConfig {
    speed: I2cSpeed::Standard,
    xfer_mode: I2cXferMode::DmaMode,
    multi_master: true,
    smbus_timeout: true,
    smbus_alert: false,
    clock_config: ClockConfig::ast1060_default(),
};

/// 16-byte-aligned DMA backing store. The AST1060 slave/master DMA
/// base-address registers ignore the low address bits (16-byte granularity);
/// an unaligned buffer makes the engine fetch/store at the aligned-down
/// address, so CPU and DMA see different memory. RX may survive by luck, but
/// slave TX then clocks out stale/idle bytes and the master reads 0xff.
#[repr(C, align(16))]
struct DmaBuf<const N: usize>([u8; N]);

#[unsafe(link_section = ".ram_nc")]
static mut MASTER_DMA_BUF: DmaBuf<4096> = DmaBuf([0u8; 4096]);
#[unsafe(link_section = ".ram_nc")]
static mut SLAVE_DMA_BUF: DmaBuf<256> = DmaBuf([0u8; 256]);

macro_rules! fail {
    ($($arg:tt)*) => {{
        pw_log::error!($($arg)*);
        loop {}
    }};
}

#[allow(dead_code)]
fn isr_event_code(kind: I2cIsrEvent) -> u32 {
    match kind {
        I2cIsrEvent::SlaveRdReq => 1,
        I2cIsrEvent::SlaveWrReq => 2,
        I2cIsrEvent::SlaveRdProc => 3,
        I2cIsrEvent::SlaveWrRecvd => 4,
        I2cIsrEvent::SlaveWrRecvdStop => 5,
        I2cIsrEvent::SlaveStop => 6,
    }
}

/// Per-transaction mailbox cursor (mirrors aspeed `swmbx_target` state).
struct MailboxState {
    swmbx: SwmbxCtrl,
    port: usize,
    cursor: u8,
    first_write: bool,
}

impl MailboxState {
    /// Apply a received RX burst: first byte is the offset (opens the FIFO
    /// transaction), following bytes are written at the wrapping cursor.
    fn on_write(&mut self, data: &[u8]) {
        for &byte in data {
            if self.first_write {
                self.cursor = byte;
                // Hot path: no logging between offset receipt and the TX-DMA
                // arm in write_slave_response. Blocking UART here delays the
                // arm past the master's read window (master samples 0xff).
                let _ = self.swmbx.send_start(self.port, self.cursor);
                self.first_write = false;
            } else {
                if let Err(e) = self.swmbx.send_msg(self.port, self.cursor, byte) {
                    pw_log::error!("MBX write err=0x{:04x}", e.code() as u32);
                }
                self.cursor = self.cursor.wrapping_add(1);
            }
        }
    }

    /// Produce the next byte the master is reading.
    fn on_read(&mut self) -> u8 {
        let byte = match self.swmbx.get_msg(self.port, self.cursor) {
            Ok(byte) => byte,
            Err(e) => {
                pw_log::error!("MBX read err=0x{:04x}", e.code() as u32);
                0
            }
        };
        self.cursor = self.cursor.wrapping_add(1);
        byte
    }

    fn on_stop(&mut self) {
        let _ = self.swmbx.send_stop(self.port);
        self.first_write = true;
    }
}

fn drain_slave_events(
    driver: &mut i2c_backend::BusDriver,
    mbx: &mut MailboxState,
    rx: &mut [u8; RX_BUFFER_SIZE],
) {
    let mut pending_stop = false;
    loop {
        match driver.try_next_slave_event() {
            Ok(Some((kind, len))) => {
                let _ = len;
                // HOT PATH — no logging from here until the TX-DMA arm in
                // write_slave_response. Each blocking-UART line added ~ms of
                // delay before the slave drove SDA, so the master sampled an
                // idle bus (0xff) before the byte was clocked out.
                match kind {
                    I2cIsrEvent::SlaveWrRecvd | I2cIsrEvent::SlaveWrRecvdStop => {
                        let stop_after_data = kind == I2cIsrEvent::SlaveWrRecvdStop;
                        let n = match driver.read_slave_buffer(rx) {
                            Ok(n) => n,
                            Err(_) => {
                                pw_log::error!("read_slave_buffer failed");
                                0
                            }
                        };
                        let wtx = driver.slave_waiting_for_tx();
                        if n > 0 {
                            mbx.on_write(&rx[..n]);
                        }
                        if n == 1 || wtx {
                            let byte = mbx.on_read();
                            let _ = driver.write_slave_response(&[byte]);
                        }
                        if stop_after_data {
                            pending_stop = true;
                        }
                    }
                    I2cIsrEvent::SlaveRdReq | I2cIsrEvent::SlaveRdProc => {
                        let byte = mbx.on_read();
                        let _ = driver.write_slave_response(&[byte]);
                    }
                    I2cIsrEvent::SlaveStop => {
                        mbx.on_stop();
                        pending_stop = false;
                    }
                    I2cIsrEvent::SlaveWrReq => {
                        mbx.first_write = true;
                        pw_log::info!("EV WrReq");
                    }
                }
            }
            Ok(None) => break,
            Err(_) => {
                let (_, _, s24) = driver.dbg_slave_regs();
                pw_log::error!("try_next_slave_event failed s24=0x{:08x}", s24 as u32);
                break;
            }
        }
    }

    // After the events are drained (TX already armed, so not on the hot path),
    // report any latched mailbox notifications. Fires only when a write hit a
    // notify-enabled address; mirrors aspeed-rust's `[NOTIFY] …` poll.
    while let Some((port, addr)) = mbx.swmbx.take_notify() {
        pw_log::info!("notify triggered port={} addr=0x{:02x}", port as u32, addr as u32);
    }

    if pending_stop {
        mbx.on_stop();
    }
}

#[entry]
fn entry() {
    // SAFETY: server process exclusively owns Bus 0 (I2C0 @ 0x7e7b0080), the
    // controller physically cross-wired to the master on this harness.
    if unsafe { i2c_backend::init_bus(0, &SLAVE_CFG) }.is_err() {
        fail!("init_bus(0) failed");
    }

    // SAFETY: buffers are non-cached SRAM and uniquely owned for bus lifetime.
    let master_dma_buf: &'static mut [u8] =
        unsafe { &mut (*core::ptr::addr_of_mut!(MASTER_DMA_BUF)).0 };
    let slave_dma_buf: &'static mut [u8] =
        unsafe { &mut (*core::ptr::addr_of_mut!(SLAVE_DMA_BUF)).0 };
    let mut driver =
        match unsafe { i2c_backend::open_bus_dma(0, &SLAVE_CFG, master_dma_buf, slave_dma_buf) } {
            Ok(d) => d,
            Err(_) => fail!("open_bus_dma(0) failed"),
        };

    // SAFETY: SWMBX_BUF_BASE is the platform mailbox SRAM region, mapped to this
    // process (see system.json5 `swmbx_buf`).
    let swmbx = unsafe { SwmbxCtrl::new_with_regions(SWMBX_NODE_COUNT, SWMBX_BUF_BASE) };
    let mut mbx = MailboxState {
        swmbx,
        port: 0,
        cursor: 0,
        first_write: true,
    };

    // Mailbox policy: protect + notify + FIFO behaviors, two FIFO endpoints.
    if mbx
        .swmbx
        .enable_behavior(SWMBX_PROTECT | SWMBX_NOTIFY | SWMBX_FIFO, true)
        .is_err()
    {
        fail!("enable_behavior failed");
    }
    if mbx
        .swmbx
        .update_notify(0, UFM_WRITE_FIFO, true)
        .is_err()
    {
        fail!("update_notify fifo write failed");
    }
    if mbx
        .swmbx
        .update_fifo(0, UFM_WRITE_FIFO, SWMBX_WRITE_FIFO_SIZE, SWMBX_FIFO_NOTIFY_STOP, true)
        .is_err()
    {
        fail!("update_fifo write failed");
    }
    if mbx
        .swmbx
        .update_fifo(1, UFM_READ_FIFO, SWMBX_READ_FIFO_SIZE, SWMBX_FIFO_NOTIFY_STOP, true)
        .is_err()
    {
        fail!("update_fifo read failed");
    }
    if mbx.swmbx.update_notify(0, BMC_UPDATE_INTENT, true).is_err() {
        fail!("update_notify failed");
    }

    // Per-port write-protection bitmaps (packed 32-bit words, 8 words = 256
    // nodes/port). A 1 bit marks that mailbox offset write-protected once global
    // PROTECT is enabled. Mirrors aspeed-rust's swmbx access-control tables
    // (port 0 = BMC, port 1 = PCH).
    pw_log::info!("apply_protect port=0 start_idx=0 words={}", BMC_ACCESS_CONTROL.len() as u32);
    if mbx.swmbx.apply_protect(0, &BMC_ACCESS_CONTROL, 0).is_err() {
        fail!("apply_protect BMC failed");
    }
    pw_log::info!("apply_protect port=0 done");
    pw_log::info!("apply_protect port=1 start_idx=0 words={}", PCH_ACCESS_CONTROL.len() as u32);
    if mbx.swmbx.apply_protect(1, &PCH_ACCESS_CONTROL, 0).is_err() {
        fail!("apply_protect PCH failed");
    }
    pw_log::info!("apply_protect port=1 done");

    // Register the IRQ with the wait group BEFORE arming the controller's slave
    // interrupts, so the kernel is ready to deliver the first edge. (The old
    // server-runtime design registered the IRQ first and was armed later over
    // IPC; arming the HW before registration loses the first interrupt.)
    if syscall::wait_group_add(
        handle::WG,
        handle::I2C0_IRQ,
        signals::I2C0,
        handle::I2C0_IRQ as usize,
    )
    .is_err()
    {
        fail!("wait_group_add irq failed");
    }

    // Enter slave/target mode at SLAVE_ADDR and arm the slave IRQ.
    if driver.configure_slave_address(SLAVE_ADDR).is_err() {
        fail!("configure_slave_address failed");
    }
    if driver.enable_slave_mode().is_err() {
        fail!("enable_slave_mode failed");
    }

    let (s20, c00, s24) = driver.dbg_slave_regs();
    pw_log::info!(
        "slave regs: i2cs20(irq_en)=0x{:08x} i2cc00(ctrl)=0x{:08x} i2cs24(sts)=0x{:08x}",
        s20 as u32,
        c00 as u32,
        s24 as u32
    );
    // Unmask the controller IRQ at the kernel: NVIC 110 starts masked after
    // registration; the first interrupt_ack arms delivery of controller-driven
    // interrupts (debug_trigger bypassed the mask, hence it woke but real slave
    // activity did not).
    let _ = syscall::interrupt_ack(handle::I2C0_IRQ, signals::I2C0);
    let (s20, c00, s24) = driver.dbg_slave_regs();
    pw_log::info!(
        "after initial irq_ack: i2cs20=0x{:08x} i2cc00=0x{:08x} i2cs24=0x{:08x}",
        s20 as u32,
        c00 as u32,
        s24 as u32
    );

    pw_log::info!("PFR mailbox server ready on Bus 0, slave addr=0x{:02x}", SLAVE_ADDR as u32);

    let mut rx = [0u8; RX_BUFFER_SIZE];
    loop {
        let deadline = syscall::debug_clock_now() + Duration::from_millis(IRQ_POLL_MS);
        let Ok(w) = syscall::object_wait(handle::WG, signals::I2C0, deadline) else {
            // Poll fallback: drain if the controller latched anything while we
            // were not woken. No logging on this path — blocking UART here slows
            // servicing enough that the hardware batches multiple bus phases
            // into one PKT_DONE status.
            let (_, _, s24) = driver.dbg_slave_regs();
            if s24 != 0 {
                drain_slave_events(&mut driver, &mut mbx, &mut rx);
                let _ = syscall::interrupt_ack(handle::I2C0_IRQ, signals::I2C0);
            }
            continue;
        };
        if !w.pending_signals.contains(signals::I2C0) {
            continue;
        }
        let acked = w.pending_signals & signals::I2C0;

        // Drain every event the controller has latched this IRQ.
        drain_slave_events(&mut driver, &mut mbx, &mut rx);

        if syscall::interrupt_ack(handle::I2C0_IRQ, acked).is_err() {
            fail!("interrupt_ack failed");
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
