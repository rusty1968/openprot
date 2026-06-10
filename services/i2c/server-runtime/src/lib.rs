// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! IPC dispatch loop. **One IPC channel per bus** — a client process is wired
//! by configuration to exactly one bus's channel and physically cannot
//! address another. The server owns one driver per bus (master *and* slave)
//! and routes each wake-up to its bus by channel handle.
//!
//! Topology-agnostic, like the usart server's loop: the runtime takes a list
//! of bus bindings + the i2c IRQ, registers them with the WaitGroup, and
//! routes wake-ups via `user_data`.
//!
//! ## Slave-receive notification (thin slice)
//!
//! The WaitGroup multiplexes the per-bus client channels (`READABLE`) **and**
//! the i2c hardware IRQ. On the IRQ: for every notification-armed bus, drain
//! the slave RX into that bus's event queue, `interrupt_ack`, then raise
//! `Signals::USER` on that bus's client channel. The client then issues
//! `SlaveReceive`, which returns the oldest queued event (status `NoData` if empty).
//! Arm/disarm is `EnableSlaveNotification` / `DisableSlaveNotification`.
//!
//! The **only** kernel-tagged server crate; it wraps the host-buildable
//! `i2c_server::{dispatch, slave::dispatch_slave}` in the Pigweed loop.

#![no_std]

use i2c_api::seam::{
    I2c, I2cBusRecovery, I2cIsrEvent, I2cSlaveBuffer, I2cSlaveEvent, SevenBitAddress,
};
use i2c_api::{I2cError, I2cOp, I2cRequestHeader, I2cResponseHeader, SlaveEvent, MAX_PAYLOAD_SIZE};
use i2c_server::slave::dispatch_slave;
use i2c_server::{dispatch, MAX_BUF_SIZE};
use userspace::syscall::{self, Signals};
use userspace::time::Instant;

const SLAVE_EVENT_QUEUE_DEPTH: usize = 4;

/// One bus the server owns: its dedicated IPC channel, IRQ handle, the driver instance
/// (master + slave), and the per-bus slave event notification queue.
pub struct Bus<B> {
    /// IPC channel handle (`channel_handler`) dedicated to this bus.
    pub channel: u32,
    /// IRQ handle for this bus's controller.
    pub irq: u32,
    /// The bus driver — implements both the master and slave seams.
    pub driver: B,
    notif_enabled: bool,
    event_data: [[u8; MAX_PAYLOAD_SIZE]; SLAVE_EVENT_QUEUE_DEPTH],
    event_len: [usize; SLAVE_EVENT_QUEUE_DEPTH],
    event_source: [u8; SLAVE_EVENT_QUEUE_DEPTH],
    event_kind: [SlaveEvent; SLAVE_EVENT_QUEUE_DEPTH],
    event_head: usize,
    event_tail: usize,
    event_count: usize,
}

impl<B> Bus<B> {
    pub const fn new(channel: u32, irq: u32, driver: B) -> Self {
        Self {
            channel,
            irq,
            driver,
            notif_enabled: false,
            event_data: [[0u8; MAX_PAYLOAD_SIZE]; SLAVE_EVENT_QUEUE_DEPTH],
            event_len: [0; SLAVE_EVENT_QUEUE_DEPTH],
            event_source: [0; SLAVE_EVENT_QUEUE_DEPTH],
            event_kind: [SlaveEvent::DataReceived; SLAVE_EVENT_QUEUE_DEPTH],
            event_head: 0,
            event_tail: 0,
            event_count: 0,
        }
    }

    fn clear_slave_events(&mut self) {
        self.event_head = 0;
        self.event_tail = 0;
        self.event_count = 0;
        self.event_len = [0; SLAVE_EVENT_QUEUE_DEPTH];
    }

    fn has_slave_event(&self) -> bool {
        self.event_count > 0
    }

    fn push_slave_event(&mut self, kind: SlaveEvent, source: u8, data: &[u8]) -> bool {
        if self.event_count == SLAVE_EVENT_QUEUE_DEPTH {
            return false;
        }

        let slot = self.event_tail;
        let n = data.len().min(MAX_PAYLOAD_SIZE);
        if n > 0 {
            self.event_data[slot][..n].copy_from_slice(&data[..n]);
        }
        self.event_len[slot] = n;
        self.event_source[slot] = source;
        self.event_kind[slot] = kind;
        self.event_tail = (self.event_tail + 1) % SLAVE_EVENT_QUEUE_DEPTH;
        self.event_count += 1;
        true
    }

    /// Push an event, logging if the queue is full (the common call shape).
    fn push_or_warn(&mut self, kind: SlaveEvent, source: u8, data: &[u8]) {
        if !self.push_slave_event(kind, source, data) {
            pw_log::error!("slave event queue full");
        }
    }

    fn pop_slave_event(&mut self) {
        if self.event_count == 0 {
            return;
        }
        self.event_len[self.event_head] = 0;
        self.event_head = (self.event_head + 1) % SLAVE_EVENT_QUEUE_DEPTH;
        self.event_count -= 1;
    }
}

fn encode_error(resp: &mut [u8], e: I2cError) -> usize {
    let h = I2cResponseHeader::error(e);
    resp[..I2cResponseHeader::SIZE].copy_from_slice(zerocopy::IntoBytes::as_bytes(&h));
    I2cResponseHeader::SIZE
}

fn encode_ok(resp: &mut [u8], payload_len: usize) -> usize {
    let h = I2cResponseHeader::success(payload_len as u16);
    resp[..I2cResponseHeader::SIZE].copy_from_slice(zerocopy::IntoBytes::as_bytes(&h));
    I2cResponseHeader::SIZE + payload_len
}

/// Parse just the request header (op + the SlaveReceive max-len in op_count).
fn header(req: &[u8]) -> Option<(I2cOp, usize)> {
    if req.len() < I2cRequestHeader::SIZE {
        return None;
    }
    let h =
        zerocopy::Ref::<_, I2cRequestHeader>::from_bytes(&req[..I2cRequestHeader::SIZE]).ok()?;
    Some((h.operation().ok()?, h.op_count_value()))
}

/// Run the i2c server forever.
///
/// Registers every bus channel (`READABLE`) and each bus's IRQ (`irq_signals`)
/// with `wg`, then loops. `buses` must be non-empty with distinct channels.
pub fn run<B>(wg: u32, irq_signals: Signals, buses: &mut [Bus<B>]) -> !
where
    B: I2c<SevenBitAddress> + I2cSlaveBuffer<SevenBitAddress> + I2cSlaveEvent + I2cBusRecovery,
{
    for bus in buses.iter() {
        if let Err(_) =
            syscall::wait_group_add(wg, bus.channel, Signals::READABLE, bus.channel as usize)
        {
            pw_log::error!("wait_group_add bus channel failed");
        }
        if let Err(_) = syscall::wait_group_add(wg, bus.irq, irq_signals, bus.irq as usize) {
            pw_log::error!("wait_group_add irq failed");
        }
    }

    let mut request_buf = [0u8; MAX_BUF_SIZE];
    let mut response_buf = [0u8; MAX_BUF_SIZE];
    let mut slave_event_buf = [0u8; MAX_PAYLOAD_SIZE];
    let wait_mask = Signals::READABLE | irq_signals;

    loop {
        let Ok(w) = syscall::object_wait(wg, wait_mask, Instant::MAX) else {
            continue;
        };

        // ---- hardware slave IRQ: drain the armed bus, ack, wake client ----
        // Each bus registered its IRQ with user_data = bus.irq, so check if this
        // signal came from an IRQ (vs. a client channel READABLE event).
        if w.pending_signals.contains(irq_signals) {
            let irq = w.user_data as u32;
            let acked = w.pending_signals & irq_signals;
            if let Some(bus) = buses.iter_mut().find(|b| b.irq == irq) {
                if bus.notif_enabled {
                    match bus.driver.try_next_slave_event() {
                        Ok(Some((kind, _))) => {
                            match kind {
                                I2cIsrEvent::SlaveWrRecvd
                                | I2cIsrEvent::SlaveWrRecvdStop => {
                                    // Plain offset-addressed mailbox: RX is
                                    // [offset, data...] with no MCTP source header
                                    // (AST_I2CC_SLAVE_PKT_SAVE_ADDR is cleared). No
                                    // source to extract; the client defaults to port 0.
                                    let stop_after_data =
                                        kind == I2cIsrEvent::SlaveWrRecvdStop;
                                    slave_event_buf.fill(0);
                                    match bus.driver.read_slave_buffer(&mut slave_event_buf) {
                                        Ok(n) if n > 0 => bus.push_or_warn(
                                            SlaveEvent::DataReceived,
                                            0xFF,
                                            &slave_event_buf[..n],
                                        ),
                                        Ok(_) => {}
                                        Err(_) => pw_log::error!("read_slave_buffer failed"),
                                    }
                                    if stop_after_data {
                                        bus.push_or_warn(SlaveEvent::Stop, 0xFF, &[]);
                                    }
                                }
                                I2cIsrEvent::SlaveRdReq | I2cIsrEvent::SlaveRdProc => {
                                    bus.push_or_warn(SlaveEvent::ReadRequest, 0xFF, &[]);
                                }
                                I2cIsrEvent::SlaveStop => {
                                    bus.push_or_warn(SlaveEvent::Stop, 0xFF, &[]);
                                }
                                I2cIsrEvent::SlaveWrReq => {}
                            }
                        }
                        Ok(None) => {
                            pw_log::debug!(
                                "slave IRQ fired but no data ready — spurious or non-data event"
                            );
                        }
                        Err(_) => {
                            pw_log::error!("try_next_slave_event failed");
                        }
                    }
                }
                if let Err(_) = syscall::interrupt_ack(irq, acked) {
                    pw_log::error!("interrupt_ack failed");
                }
                if bus.notif_enabled && bus.has_slave_event() {
                    // ORs USER onto the bus channel without disturbing READABLE.
                    if let Err(_) = syscall::object_set_peer_user_signal(bus.channel, true) {
                        pw_log::error!("object_set_peer_user_signal failed");
                    }
                }
            }
            continue;
        }

        if !w.pending_signals.contains(Signals::READABLE) {
            continue;
        }
        let channel = w.user_data as u32;
        let Some(bus) = buses.iter_mut().find(|b| b.channel == channel) else {
            continue;
        };
        let Ok(req_len) = syscall::channel_read(channel, 0, &mut request_buf) else {
            continue;
        };
        let req = &request_buf[..req_len];

        let resp_len = match header(req) {
            Some((I2cOp::Transaction, _)) => {
                let n = dispatch(&mut bus.driver, req, &mut response_buf);
                // After a bus-level error, attempt hardware recovery so the
                // next transaction starts clean. The client already has the
                // encoded error; recovery is best-effort (ignore its result).
                if n >= I2cResponseHeader::SIZE {
                    if let Some(rhdr) = zerocopy::Ref::<_, I2cResponseHeader>::from_bytes(
                        &response_buf[..I2cResponseHeader::SIZE],
                    )
                    .ok()
                    {
                        use i2c_api::I2cError;
                        match rhdr.error_code() {
                            I2cError::Bus | I2cError::ArbitrationLoss | I2cError::Timeout => {
                                let _ = bus.driver.recover_bus();
                            }
                            _ => {}
                        }
                    }
                }
                n
            }
            Some((I2cOp::ConfigureSlave | I2cOp::EnableSlave | I2cOp::DisableSlave, _)) => {
                dispatch_slave(&mut bus.driver, req, &mut response_buf)
            }
            Some((I2cOp::EnableSlaveNotification, _)) => {
                bus.notif_enabled = true;
                encode_ok(&mut response_buf, 0)
            }
            Some((I2cOp::DisableSlaveNotification, _)) => {
                bus.notif_enabled = false;
                bus.clear_slave_events();
                encode_ok(&mut response_buf, 0)
            }
            Some((I2cOp::SlaveReceive, max_len)) => {
                if !bus.has_slave_event() {
                    encode_error(&mut response_buf, I2cError::NoData)
                } else {
                    // Response payload: [kind (1), source_addr (1), data (0..max_len)]
                    let cap = response_buf.len() - I2cResponseHeader::SIZE;
                    let metadata_size = 2; // kind + source
                    if cap < metadata_size {
                        encode_error(&mut response_buf, I2cError::BufferTooSmall)
                    } else {
                        let slot = bus.event_head;
                        let data_cap = cap - metadata_size;
                        let n = bus.event_len[slot].min(max_len).min(data_cap);
                        let payload_offset = I2cResponseHeader::SIZE;
                        response_buf[payload_offset] = bus.event_kind[slot] as u8;
                        response_buf[payload_offset + 1] = bus.event_source[slot];
                        if n > 0 {
                            response_buf[payload_offset + 2..payload_offset + 2 + n]
                                .copy_from_slice(&bus.event_data[slot][..n]);
                        }
                        bus.pop_slave_event();
                        if bus.has_slave_event() {
                            let _ = syscall::object_set_peer_user_signal(bus.channel, true);
                        }
                        encode_ok(&mut response_buf, metadata_size + n)
                    }
                }
            }
            // NOTE: not needed for MCTP (master-write only). For testing slave-TX patterns.
            Some((I2cOp::SlaveSetResponse, _)) => {
                let hdr_end = I2cRequestHeader::SIZE;
                if req.len() < hdr_end {
                    encode_error(&mut response_buf, I2cError::InvalidOperation)
                } else {
                    let payload = &req[hdr_end..];
                    match bus.driver.write_slave_response(payload) {
                        Ok(()) => encode_ok(&mut response_buf, 0),
                        Err(_) => encode_error(&mut response_buf, I2cError::InternalError),
                    }
                }
            }
            Some((_, _)) | None => encode_error(&mut response_buf, I2cError::InvalidOperation),
        };
        if let Err(_) = syscall::channel_respond(channel, &response_buf[..resp_len]) {
            pw_log::error!("channel_respond failed");
        }
    }
}
