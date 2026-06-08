// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![no_std]

pub mod smbus_mailbox_client;
pub mod swmbx_ctrl;

pub use smbus_mailbox_client::{I2cPfrClientError, I2cPfrSmbusClient, Source, SourceAddressMap};
pub use swmbx_ctrl::{
	SwmbxCtrl, SwmbxError, SWMBX_BUF_BASE, SWMBX_DEV_COUNT, SWMBX_FIFO, SWMBX_FIFO_COUNT,
	SWMBX_FIFO_DEPTH, SWMBX_FIFO_NOTIFY_START, SWMBX_FIFO_NOTIFY_STOP,
	SWMBX_NODE_COUNT, SWMBX_NOTIFY, SWMBX_PROTECT,
};
