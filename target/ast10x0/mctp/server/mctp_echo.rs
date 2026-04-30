// Licensed under the Apache-2.0 license

#![no_main]
#![no_std]

use openprot_mctp_api::stack::Stack;
use openprot_mctp_api::{MctpError, MctpListener, MctpRespChannel};
use openprot_mctp_client::IpcMctpClient;
use userspace::entry;

use app_mctp_echo::handle;

const OWN_EID: u8 = 8;
const ECHO_MSG_TYPE: u8 = 0x7e;
const MAX_ECHO_PAYLOAD: usize = 1023;

fn mctp_echo_loop() -> Result<(), MctpError> {
    let stack = Stack::new(IpcMctpClient::new(handle::MCTP));
    stack.set_eid(OWN_EID)?;

    let mut listener = stack.listener(ECHO_MSG_TYPE, 0)?;
    let mut recv_buf = [0u8; MAX_ECHO_PAYLOAD];

    loop {
        let (_, msg, mut resp) = listener.recv(&mut recv_buf)?;
        resp.send(msg)?;
    }
}

#[entry]
fn entry() -> ! {
    if let Err(e) = mctp_echo_loop() {
        ast10x0_userspace_runtime::fail_stop("mctp_echo", e.code as u32);
    }
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
