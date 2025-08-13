//! Increasingly non-basic GPIO keyboard matrix scanner.
//!
//! # Scanning
//!
//! This scans a rectangular matrix by setting one "row" pin high at a time,
//! waiting for a bit, and then reading the state of all "column" pins. These
//! states are fed through per-key debouncing logic to produce a model of the
//! state of all physical switches.
//!
//! Scanning is performed under control of a hardware timer (currently TIM2) to
//! get higher interrupt rates than the kernel timer can provide. The timing is
//! set up to scan at roughly 16 kHz, so that we can handle a 16-row matrix with
//! a 1 kHz update rate.
//!
//! Currently, debouncing is performed in terms of scan passes, not time.
//! Because scanning is pretty regular, this doesn't seem like a problem yet,
//! but may bear rethinking down the road.
//!
//!
//! # Layers and layouts
//!
//! The physical switches are combined with a _layer set_ to determine what
//! logical keys are being pressed. Logical keys include both "real" keys
//! defined in the USB HID spec, and also _meta_ keys that affect the operation
//! of the keyboard, such as layer shift keys.
//!
//!
//! # Report production
//!
//! We produce reports when poked by the USB driver via the `USB_REPORT_NEEDED`
//! notification. To produce a report, we process the state of the meta keys
//! (since they affect the other keys) and then generate a HID report describing
//! the "real" keys.

#![no_std]
#![no_main]

use core::{mem::MaybeUninit, num::NonZeroU8, ptr::addr_of_mut, sync::atomic::{AtomicBool, Ordering}};

use hubris_task_slots::SLOTS;
use drv_stm32xx_sys_api::{Stm32Sys as Sys, Port, Pull, PeripheralName};
use drv_stm32l4_usb_api::{UsbHid, UsbEvent};
use idyll_runtime::{NotificationHandler, Meta};
use userlib::{sys_enable_irq_and_clear_pending, ReplyFaultReason};

const INTERVAL: u32 = 62 - 15; // microseconds; fudge factor adjusts for code
                               // execution time

const QUEUE_DEPTH: usize = 16;

#[export_name = "main"]
fn main() -> ! {
    let sys = Sys::from(SLOTS.sys);
    let usb = UsbHid::from(SLOTS.usbhid);

    sys.enable_clock(PeripheralName::Tim2);

    let phys_table = claim_physical_table();

    for (port, pin) in config::ROWS {
        sys.set_pin_output(port, pin);
    }

    for (port, pin) in config::COLS {
        sys.set_pin_pull(port, pin, Some(Pull::Down));
        sys.set_pin_input(port, pin);
    }

    // Alright, we're done using SYS for performance reasons, start poking the
    // GPIO directly.
    for (port, pin) in config::ROWS {
        set_pin_low(port, pin);
    }

    let tim = stm32_metapac::TIM2;
    // By default the timer is receiving 80 MHz from the bus fabric. Divide that
    // by 80 to give a microsecond timebase.
    tim.psc().write(|w| w.set_psc(80 - 1));
    // Let's wake up every INTERVAL us.
    tim.arr().write(|w| w.set_arr(INTERVAL - 1));
    // Trigger an update event so those registers get applied.
    tim.egr().write(|w| w.set_ug(true));
    // Clear any status bits that just got set by that. All writable bits in
    // this register are write-zero-to-clear.
    tim.sr().write(|w| w.0 = 0);
    // Enable interrupt on timer overflow (update).
    tim.dier().write(|w| w.set_uie(true));
    // Start the timer!
    tim.cr1().write(|w| {
        // One-pulse mode causes the timer to interrupt once and turn itself
        // off, which gets us our "at least" sleep behavior more effectively
        // than a cyclic repeating interrupt would ... but it does mean we have
        // to remember to turn it back on!
        w.set_opm(true);
        w.set_cen(true);
    });

    // Turn on our IRQ. It'll fire real soon now but we won't notice until we
    // recv.
    sys_enable_irq_and_clear_pending(hubris_notifications::TIM_IRQ);

    // Event loop!
    let mut buffer = [MaybeUninit::uninit(); SCANNER_BUFFER_SIZE];
    let mut server = Server {
        tim,
        scan_row: 0,
        phys_table,
        queue: heapless::Deque::new(),
        usb,
        config: None,
        layers: LayerSet {
            layers: &mut [
                (&CANNED_LAYER_0, true),
                (&CANNED_LAYER_1, false),
            ],
        },
    };
    loop {
        idyll_runtime::dispatch_or_event(
            &mut server,
            hubris_notifications::TIM_IRQ
                | hubris_notifications::USB_EVENT_READY
                | hubris_notifications::USB_REPORT_NEEDED,
            &mut buffer,
        );
    }
}

fn claim_physical_table() -> &'static mut [[PhysKey; config::COL_COUNT]; config::ROW_COUNT]  {
    static PHYSICAL_TAKEN: AtomicBool = AtomicBool::new(false);

    if PHYSICAL_TAKEN.swap(true, Ordering::Relaxed) {
        panic!();
    }

    static mut PHYSICAL: [[PhysKey; config::COL_COUNT]; config::ROW_COUNT] = 
        [[PhysKey::DEFAULT; config::COL_COUNT]; config::ROW_COUNT];
    unsafe { &mut *addr_of_mut!(PHYSICAL) }
}

struct Server<'a> {
    tim: stm32_metapac::timer::TimGp32,
    scan_row: usize,
    phys_table: &'static mut [[PhysKey; config::COL_COUNT]; config::ROW_COUNT] ,

    queue: heapless::Deque<KeyEvent, QUEUE_DEPTH>,

    usb: UsbHid,
    config: Option<Config>,

    layers: LayerSet<'a>,
}

impl Scanner for Server<'_> {
    fn pop_event(
        &mut self,
        _: Meta,
    ) -> Result<Option<KeyEvent>, ReplyFaultReason> {
        Ok(self.queue.pop_front())
    }
}

impl NotificationHandler for Server<'_> {
    fn handle_notification(&mut self, bits: u32) {
        if bits & hubris_notifications::TIM_IRQ != 0 {
            // Advance key scanning, but only if time has really elapsed. (This
            // check is me being pedantic: it's possible for other tasks to post
            // our timer notification instead of the kernel doing it, so, I
            // check the time.)
            if self.tim.sr().read().uif() {
                // A real timer event! Clear the interrupt condition at the
                // timer...
                self.tim.sr().write(|w| {
                    // write 1 to preserve
                    w.0 = !0;
                    // ...except the update flag
                    w.set_uif(false);
                });
                // ...and re-enable in the kernel.
                sys_enable_irq_and_clear_pending(hubris_notifications::TIM_IRQ);

                // Read column inputs
                let row = &mut self.phys_table[self.scan_row];
                for (i, (port, pin)) in config::COLS.into_iter().enumerate() {
                    let gpio = get_port(port);
                    let observed_state = if gpio.idr().read().0 & (1 << pin) != 0 {
                        PhysState::Closed(self.layers.get(self.scan_row, i))
                    } else {
                        PhysState::Open
                    };

                    row[i].step(observed_state);
                }

                // Turn off this row and turn on the next.
                set_pin_low(config::ROWS[self.scan_row].0, config::ROWS[self.scan_row].1);
                let next = self.scan_row + 1;
                if next == config::ROW_COUNT {
                    // TODO if we wanted to do anything special at end-of-scan
                    // it'd happen here.
                    self.scan_row = 0;
                }  else {
                    self.scan_row = next;
                }
                set_pin_high(config::ROWS[self.scan_row].0, config::ROWS[self.scan_row].1);

                // Restart our timer. We do this _after_ turning on the GPIO
                // above so that it acts as a propagation delay before we read.
                self.tim.cr1().modify(|w| w.set_cen(true));
            }
        }

        let mut deliver_report_now = false;

        if bits & hubris_notifications::USB_EVENT_READY != 0 {
            if let Some(event) = self.usb.get_event() {
                match event {
                    UsbEvent::Reset => {
                        self.config = None;
                    }
                    UsbEvent::Configured => {
                        self.config = Some(Config::BootProtocol);
                        deliver_report_now = true;
                    }
                    UsbEvent::ReportDescriptorNeeded { length } => {
                        // In this case, we do not deliver a report on EP1.
                        // Instead, we're going to deposit a descriptor on EP0.
                        let n = BOOT_KBD_DESC.len().min(usize::from(length));
                        self.usb.enqueue_report(0, &BOOT_KBD_DESC[..n]).ok();
                    }
                }
            }
        }
        if bits & hubris_notifications::USB_REPORT_NEEDED != 0 {
            deliver_report_now = true;
        }
        if deliver_report_now {
            self.process_meta();
            let report = self.generate_boot_report();
            self.usb.enqueue_report(1, &report).ok();
        }
    }
}

impl Server<'_> {
    pub fn process_meta(&mut self) {
        // Make a first pass handling all meta-keys, since they affect the
        // results of the next pass.
        for row in self.phys_table.iter_mut() {
            for key in row {
                let (before, after) = (key.get_handled_sym(), key.get_sym());
                match (before, after) {
                    (Some(KeySym::Layer { index, action }), None) => {
                        // Layer-related key is released.
                        match action {
                            LayerAction::EnableHold => {
                                self.layers.set_enable(usize::from(index), false);
                            }
                        }
                        key.mark_as_handled();
                    }
                    (None, Some(KeySym::Layer { index, action })) => {
                        // Layer-related key is pressed.
                        match action {
                            LayerAction::EnableHold => {
                                self.layers.set_enable(usize::from(index), true);
                            }
                        }
                        key.mark_as_handled();
                    }
                    (_, _) => (),
                }
            }
        }
    }

    pub fn generate_boot_report(&self) -> [u8; 8] {
        let mut non_modifier_key_count = 0;
        let mut report = [0; 8];

        // In the second pass, we handle all reporting keys subject to the
        // effects of the previous pass.
        'generate_loop:
        for row in &*self.phys_table {
            for key in row {
                if let Some(sym) = key.get_sym() {
                    if let Some(modifier) = sym.as_standard_modifier() {
                        // Standard modifiers get packed into byte zero of the
                        // report, rather than reported as keys.
                        report[0] |= 1 << (modifier as u32);
                    } else {
                        match sym {
                            KeySym::HidStd(usage) => {
                                if let Some(spot) = report.get_mut(2 + non_modifier_key_count) {
                                    *spot = usage;
                                    non_modifier_key_count += 1;
                                } else {
                                    // Welp, we've filled the whole dang thing
                                    report[2..].fill(0x01);
                                    break 'generate_loop;
                                }
                            }

                            // Handled above:
                            KeySym::Layer { .. } => (),
                        }
                    }
                }
            }
        }

        report
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Config {
    BootProtocol,
}

fn set_pin_low(port: Port, pin: u8) {
    let gpio = get_port(port);
    let pin = pin as usize & 0xF;
    gpio.bsrr().write(|w| w.set_br(pin, true));
}

fn set_pin_high(port: Port, pin: u8) {
    let gpio = get_port(port);
    let pin = pin as usize & 0xF;
    gpio.bsrr().write(|w| w.set_bs(pin, true));
}

fn get_port(port: Port) -> stm32_metapac::gpio::Gpio {
    match port {
        Port::A => stm32_metapac::GPIOA,
        Port::B => stm32_metapac::GPIOB,
        Port::C => stm32_metapac::GPIOC,
        Port::D => stm32_metapac::GPIOD,
        Port::H => stm32_metapac::GPIOH,

        _ => unreachable!(),
    }
}

#[derive(Copy, Clone, Debug, Default)]
struct PhysKey {
    /// The current official state of the key, after debouncing.
    state: PhysState,
    /// Debouncing timer; when `None`, the key is thought stable. When
    /// `Some(t)`, there are `t` scans remaining before we flip it.
    transition: Option<NonZeroU8>,
    /// The state of the key that was last observed by the higher level
    /// processing loop. This can be used to determine when a particular key is
    /// _newly_ pressed or released, which we mostly need for internally handled
    /// function keys.
    handled_state: PhysState,
}

impl PhysKey {
    const DEFAULT: Self = Self {
        state: PhysState::Open,
        transition: None,
        handled_state: PhysState::Open,
    };
    const INTERVAL: NonZeroU8 = unsafe { NonZeroU8::new_unchecked(5) };

    /// Advances the key state machine in response to seeing the key in
    /// `observed_state`.
    ///
    /// Note that in the `Closed` case, `observed_state` carries the current
    /// keysym from the layout. This has the slightly odd side effect that if
    /// the keysym changes _during the key down debounce interval_ we'll take
    /// the last one.
    fn step(&mut self, observed_state: PhysState) {
        match (self.state, observed_state) {
            (PhysState::Closed(_), PhysState::Closed(_)) | (PhysState::Open, PhysState::Open) => {
                // No change huh. Cancel any debouncing.
                self.transition.take();

                // In the closed state: we don't actually care what keysym the
                // caller _thinks_ is current, because we're latching ours. (The
                // layout may have changed while this key is being held.)
            }
            (PhysState::Closed(_), PhysState::Open) | (PhysState::Open, PhysState::Closed(_)) => {
                // This key appears to have changed, though we may not believe
                // it yet.
                match self.transition {
                    Some(t) => {
                        // The transition timer is running...
                        self.transition = NonZeroU8::new(u8::from(t) - 1);
                        if self.transition.is_none() {
                            // The timer has just elapsed!
                            self.state = observed_state;
                        }
                    }
                    None => {
                        // The timer is just starting.
                        self.transition = Some(Self::INTERVAL);
                    }
                }
            }
        }
    }

    /// Returns the active keysym if if this key is currently considered to be
    /// down, `None` otherwise.
    fn get_sym(&self) -> Option<KeySym> {
        self.state.as_closed()
    }

    fn get_handled_sym(&self) -> Option<KeySym> {
        self.handled_state.as_closed()
    }

    fn mark_as_handled(&mut self) {
        self.handled_state = self.state;
    }
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
enum PhysState {
    #[default]
    Open,
    Closed(Option<KeySym>),
}

impl PhysState {
    fn as_closed(self) -> Option<KeySym> {
        if let PhysState::Closed(sym) = self {
            sym
        } else {
            None
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum KeySym {
    HidStd(u8),
    Layer {
        index: u8,
        action: LayerAction,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum LayerAction {
    EnableHold,
}

impl KeySym {
    pub const fn as_standard_modifier(&self) -> Option<StandardMod> {
        match self {
            Self::HidStd(0xE0) => Some(StandardMod::LeftControl),
            Self::HidStd(0xE1) => Some(StandardMod::LeftShift),
            Self::HidStd(0xE2) => Some(StandardMod::LeftAlt),
            Self::HidStd(0xE3) => Some(StandardMod::LeftGui),
            Self::HidStd(0xE4) => Some(StandardMod::RightControl),
            Self::HidStd(0xE5) => Some(StandardMod::RightShift),
            Self::HidStd(0xE6) => Some(StandardMod::RightAlt),
            Self::HidStd(0xE7) => Some(StandardMod::RightGui),
            _ => None,
        }
    }
}

#[derive(Copy, Clone, Debug)]
enum StandardMod {
    LeftControl,
    LeftShift,
    LeftAlt,
    LeftGui,
    RightControl,
    RightShift,
    RightAlt,
    RightGui,
}

/// Canned report descriptor that describes a report equivalent to the Boot
/// Keyboard Protocol report.
static BOOT_KBD_DESC: [u8; 62] = [
    0x05, 0x01,       //  Usage Page (Desktop),
    0x09, 0x06,       //  Usage (Keyboard),
    0xA1, 0x01,       //  Collection (Application),
    0x05, 0x07,       //      Usage Page (Keyboard),
    0x19, 0xE0,       //      Usage Minimum (KB Leftcontrol),
    0x29, 0xE7,       //      Usage Maximum (KB Right GUI),
    0x15, 0x00,       //      Logical Minimum (0),
    0x25, 0x01,       //      Logical Maximum (1),
    0x75, 0x01,       //      Report Size (1),
    0x95, 0x08,       //      Report Count (8),
    0x81, 0x02,       //      Input (Variable),
    0x95, 0x01,       //      Report Count (1),
    0x75, 0x08,       //      Report Size (8),
    0x81, 0x01,       //      Input (Constant),
    0x95, 0x03,       //      Report Count (3),
    0x75, 0x01,       //      Report Size (1),
    0x05, 0x08,       //      Usage Page (LED),
    0x19, 0x01,       //      Usage Minimum (01h),
    0x29, 0x03,       //      Usage Maximum (03h),
    0x91, 0x02,       //      Output (Variable),
    0x95, 0x05,       //      Report Count (5),
    0x75, 0x01,       //      Report Size (1),
    0x91, 0x01,       //      Output (Constant),
    0x95, 0x06,       //      Report Count (6),
    0x75, 0x08,       //      Report Size (8),
    0x26, 0xFF, 0x00, //      Logical Maximum (255),
    0x05, 0x07,       //      Usage Page (Keyboard),
    0x19, 0x00,       //      Usage Minimum (None),
    0x29, 0x91,       //      Usage Maximum (KB LANG2),
    0x81, 0x00,       //      Input,
    0xC0              //  End Collection
];

enum LayerSym {
    DeadKey,
    Sym(KeySym),
    Transparent,
}

struct Layer {
    syms: &'static [[LayerSym; config::COL_COUNT]; config::ROW_COUNT],
}

struct LayerSet<'a> {
    layers: &'a mut [(&'static Layer, bool)],
}

impl LayerSet<'_> {
    pub fn get(&self, row: usize, col: usize) -> Option<KeySym> {
        for (layer, enable) in self.layers.iter().rev() {
            if !*enable {
                continue;
            }

            match layer.syms[row][col] {
                LayerSym::DeadKey => return None,
                LayerSym::Sym(sym) => return Some(sym),
                LayerSym::Transparent => (),
            }
        }

        None
    }

    pub fn set_enable(&mut self, index: usize, flag: bool) {
        self.layers[index].1 = flag;
    }
}

static CANNED_LAYER_0: Layer = Layer {
    syms: &[
        [
            LayerSym::Sym(KeySym::Layer { index: 1, action: LayerAction::EnableHold }),
            LayerSym::Sym(KeySym::HidStd(0x04)),
        ],
        [
            LayerSym::Sym(KeySym::HidStd(0x05)),
            LayerSym::Sym(KeySym::HidStd(0x06)),
        ],
    ],
};

static CANNED_LAYER_1: Layer = Layer {
    syms: &[
        [
            LayerSym::Transparent,
            LayerSym::Sym(KeySym::HidStd(0x1E)),
        ],
        [
            LayerSym::Sym(KeySym::HidStd(0x1F)),
            LayerSym::Sym(KeySym::HidStd(0x20)),
        ],
    ],
};

include!(concat!(env!("OUT_DIR"), "/generated_server.rs"));
include!(concat!(env!("OUT_DIR"), "/config.rs"));
