// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::env;
use std::str;
use std::time::Duration;

use mctp::{Eid, MsgType, ReqChannel};
use mctp_linux::MctpLinuxReq;

const DEFAULT_REMOTE_EID: u8 = 8;
const DEFAULT_MSG_TYPE: u8 = 1;
const TIMEOUT_SECS: u64 = 5;

#[derive(Debug)]
enum AppError {
    ParseEnv {
        var: &'static str,
        value: String,
        source: std::num::ParseIntError,
    },
    Mctp(mctp::Error),
    Io(std::io::Error),
    Utf8(std::str::Utf8Error),
    EchoMismatch,
}

impl core::fmt::Display for AppError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ParseEnv { var, value, source } => {
                write!(f, "failed to parse {var}='{value}': {source}")
            }
            Self::Mctp(e) => write!(f, "mctp error: {e}"),
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Utf8(e) => write!(f, "utf8 decode error: {e}"),
            Self::EchoMismatch => write!(f, "echo response does not match request"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<mctp::Error> for AppError {
    fn from(value: mctp::Error) -> Self {
        Self::Mctp(value)
    }
}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<std::str::Utf8Error> for AppError {
    fn from(value: std::str::Utf8Error) -> Self {
        Self::Utf8(value)
    }
}

fn parse_u8_env(var_name: &'static str, default: u8) -> Result<u8, AppError> {
    match env::var(var_name) {
        Ok(v) => v.parse::<u8>().map_err(|source| AppError::ParseEnv {
            var: var_name,
            value: v,
            source,
        }),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Ok(default),
    }
}

/// Sends a "Hello, World!" MCTP message and verifies echoed payload.
fn run() -> Result<(), AppError> {
    let eid = parse_u8_env("REMOTE_EID", DEFAULT_REMOTE_EID)?;
    let msg_type = parse_u8_env("MSG_TYPE", DEFAULT_MSG_TYPE)?;

    let mut req = MctpLinuxReq::new(Eid(eid), None)?;
    req.as_socket()
        .set_read_timeout(Some(Duration::from_secs(TIMEOUT_SECS)))?;

    let data = b"Hello, World!";
    req.send(MsgType(msg_type), data)?;

    println!("Sent message to EID {eid}");

    let mut buf = [0u8; 255];
    let (_, _, resp) = req.recv(&mut buf)?;

    let resp_str = str::from_utf8(resp)?;
    println!("Received echo: '{resp_str}'");

    if data != resp {
        return Err(AppError::EchoMismatch);
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
