// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

//! Kernel-tagged IPC loop for the OTP service.
//!
//! Wraps the host-buildable [`otp_server::dispatch`] in the Pigweed WaitGroup
//! wait/respond loop. The runtime owns no policy: it is generic over the HAL
//! backend and the board traits, and its sole added responsibility is to anchor
//! trust to the channel — each bound channel carries a fixed [`CallerId`], so a
//! request's authority comes from where it arrived, never from the wire.

#![no_std]

use hal_otp_driver::{OtpProgramBytes, OtpReadBytes};
use otp_api::RegionId;
use otp_server::{dispatch, AccessPolicy, CallerId, FieldMap, SvnCodec, MAX_BUF_SIZE};
use userspace::syscall::{self, Signals};
use userspace::time::Instant;

/// One IPC channel bound to the identity of the caller it serves. The runtime
/// assigns each channel its `caller` from configuration; the wire cannot
/// forge it.
pub struct Binding {
    /// IPC channel handle (`channel_handler`) this server answers on.
    pub channel: u32,
    /// Fixed identity attributed to every request on `channel`.
    pub caller: CallerId,
}

impl Binding {
    /// Bind `channel` to `caller`.
    pub const fn new(channel: u32, caller: CallerId) -> Self {
        Self { channel, caller }
    }
}

/// Serve the OTP service forever over `bindings`.
///
/// Registers every channel with `wg` for `READABLE`, then loops: read a
/// request, dispatch it under the bound caller's identity, and reply. `otp` is
/// the single OTP backend; `policy`/`codec`/`field_map` are the board traits.
pub fn run<D, P, C, M>(
    wg: u32,
    otp: &mut D,
    policy: &P,
    codec: &C,
    field_map: &M,
    bindings: &[Binding],
) -> !
where
    D: OtpReadBytes<Region = RegionId> + OtpProgramBytes<Region = RegionId>,
    P: AccessPolicy,
    C: SvnCodec,
    M: FieldMap,
{
    for b in bindings {
        if syscall::wait_group_add(wg, b.channel, Signals::READABLE, b.channel as usize).is_err() {
            pw_log::error!("otp: wait_group_add failed");
        }
    }

    let mut request_buf = [0u8; MAX_BUF_SIZE];
    let mut response_buf = [0u8; MAX_BUF_SIZE];

    loop {
        let Ok(w) = syscall::object_wait(wg, Signals::READABLE, Instant::MAX) else {
            continue;
        };
        if !w.pending_signals.contains(Signals::READABLE) {
            continue;
        }
        let channel = w.user_data as u32;
        let Some(binding) = bindings.iter().find(|b| b.channel == channel) else {
            continue;
        };
        let Ok(req_len) = syscall::channel_read(channel, 0, &mut request_buf) else {
            continue;
        };
        let resp_len = dispatch(
            otp,
            policy,
            codec,
            field_map,
            binding.caller,
            &request_buf[..req_len],
            &mut response_buf,
        );
        if syscall::channel_respond(channel, &response_buf[..resp_len]).is_err() {
            pw_log::error!("otp: channel_respond failed");
        }
    }
}
