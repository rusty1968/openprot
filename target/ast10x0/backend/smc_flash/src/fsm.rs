// Licensed under the Apache-2.0 license
// SPDX-License-Identifier: Apache-2.0

#![cfg_attr(not(test), no_std)]

#[cfg(test)]
extern crate std;

use core::convert::TryFrom;

use util_error::{self as error, ErrorCode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceAccess {
    ReadOnly,
    ReadWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LongOpKind {
    Program { page_size: usize },
    Erase { sector_size: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LongOpReq {
    pub access: DeviceAccess,
    pub kind: LongOpKind,
    pub offset: u32,
    pub len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpEvent {
    Start,
    Tick,
    HwReady,
    HwBusy,
    HwError(ErrorCode),
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpCmd {
    ProgramChunk { offset: u32, len: usize },
    EraseChunk { offset: u32, len: usize },
    Poll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpStep {
    None,
    Command(OpCmd),
    Complete,
    Failed(ErrorCode),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpPhase {
    Init,
    Issue,
    Poll,
    Done,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LongOpFsm {
    phase: OpPhase,
    req: LongOpReq,
    next_offset: u32,
    remaining: usize,
    in_flight_len: usize,
    busy_retries: u8,
    max_busy_retries: u8,
    failure: ErrorCode,
}

impl LongOpFsm {
    pub const fn new(req: LongOpReq, max_busy_retries: u8) -> Self {
        let offset = req.offset;
        let len = req.len;
        Self {
            phase: OpPhase::Init,
            req,
            next_offset: offset,
            remaining: len,
            in_flight_len: 0,
            busy_retries: 0,
            max_busy_retries,
            failure: error::KERNEL_ERROR_ABORTED,
        }
    }

    pub fn step(&mut self, event: OpEvent) -> OpStep {
        if matches!(event, OpEvent::Cancel) {
            self.phase = OpPhase::Failed;
            self.failure = error::KERNEL_ERROR_CANCELLED;
            return OpStep::Failed(self.failure);
        }

        match self.phase {
            OpPhase::Init => self.step_init(event),
            OpPhase::Issue => self.step_issue(event),
            OpPhase::Poll => self.step_poll(event),
            OpPhase::Done => OpStep::Complete,
            OpPhase::Failed => OpStep::Failed(self.failure),
        }
    }

    fn step_init(&mut self, event: OpEvent) -> OpStep {
        if !matches!(event, OpEvent::Start) {
            return OpStep::None;
        }

        if self.remaining == 0 {
            self.phase = OpPhase::Done;
            return OpStep::Complete;
        }

        if matches!(self.req.access, DeviceAccess::ReadOnly) {
            self.phase = OpPhase::Failed;
            self.failure = error::FLASH_SMC_WRITE_PROTECTED;
            return OpStep::Failed(self.failure);
        }

        match self.req.kind {
            LongOpKind::Program { page_size } => {
                if page_size == 0 {
                    self.phase = OpPhase::Failed;
                    self.failure = error::FLASH_GENERIC_INVALID_SIZE;
                    return OpStep::Failed(self.failure);
                }
            }
            LongOpKind::Erase { sector_size } => {
                if sector_size == 0 {
                    self.phase = OpPhase::Failed;
                    self.failure = error::FLASH_GENERIC_INVALID_SIZE;
                    return OpStep::Failed(self.failure);
                }
            }
        }

        self.phase = OpPhase::Issue;
        self.issue_chunk_or_complete()
    }

    fn step_issue(&mut self, event: OpEvent) -> OpStep {
        if matches!(event, OpEvent::Tick | OpEvent::Start) {
            return self.issue_chunk_or_complete();
        }
        OpStep::None
    }

    fn step_poll(&mut self, event: OpEvent) -> OpStep {
        match event {
            OpEvent::HwReady => {
                if self.advance_after_completion().is_err() {
                    self.phase = OpPhase::Failed;
                    self.failure = error::FLASH_GENERIC_ADDR_OUT_OF_BOUNDS;
                    return OpStep::Failed(self.failure);
                }

                if self.remaining == 0 {
                    self.phase = OpPhase::Done;
                    return OpStep::Complete;
                }

                self.phase = OpPhase::Issue;
                self.failure = error::KERNEL_ERROR_ABORTED;
                OpStep::None
            }
            OpEvent::HwBusy | OpEvent::Tick => {
                if self.busy_retries >= self.max_busy_retries {
                    self.phase = OpPhase::Failed;
                    self.failure = error::FLASH_SMC_TIMEOUT;
                    return OpStep::Failed(self.failure);
                }
                self.busy_retries = self.busy_retries.saturating_add(1);
                OpStep::Command(OpCmd::Poll)
            }
            OpEvent::HwError(code) => {
                self.phase = OpPhase::Failed;
                self.failure = code;
                OpStep::Failed(code)
            }
            OpEvent::Start | OpEvent::Cancel => OpStep::None,
        }
    }

    fn issue_chunk_or_complete(&mut self) -> OpStep {
        if self.remaining == 0 {
            self.phase = OpPhase::Done;
            return OpStep::Complete;
        }

        let cmd = match self.req.kind {
            LongOpKind::Program { page_size } => {
                let chunk = self.program_chunk_len(page_size);
                self.in_flight_len = chunk;
                OpCmd::ProgramChunk {
                    offset: self.next_offset,
                    len: chunk,
                }
            }
            LongOpKind::Erase { sector_size } => {
                let chunk = core::cmp::min(self.remaining, sector_size);
                self.in_flight_len = chunk;
                OpCmd::EraseChunk {
                    offset: self.next_offset,
                    len: chunk,
                }
            }
        };

        self.busy_retries = 0;
        self.phase = OpPhase::Poll;
        OpStep::Command(cmd)
    }

    fn program_chunk_len(&self, page_size: usize) -> usize {
        let off_in_page = (self.next_offset as usize) % page_size;
        let page_remaining = page_size.saturating_sub(off_in_page);
        core::cmp::min(self.remaining, page_remaining)
    }

    fn advance_after_completion(&mut self) -> Result<(), ErrorCode> {
        self.next_offset = self
            .next_offset
            .checked_add(
                u32::try_from(self.in_flight_len)
                    .map_err(|_| error::FLASH_GENERIC_ADDR_OUT_OF_BOUNDS)?,
            )
            .ok_or(error::FLASH_GENERIC_ADDR_OUT_OF_BOUNDS)?;
        self.remaining = self
            .remaining
            .checked_sub(self.in_flight_len)
            .ok_or(error::FLASH_GENERIC_ADDR_OUT_OF_BOUNDS)?;
        self.in_flight_len = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fsm_rejects_writes_on_read_only() {
        let req = LongOpReq {
            access: DeviceAccess::ReadOnly,
            kind: LongOpKind::Program { page_size: 256 },
            offset: 0,
            len: 16,
        };
        let mut fsm = LongOpFsm::new(req, 3);
        let step = fsm.step(OpEvent::Start);
        assert_eq!(step, OpStep::Failed(error::FLASH_SMC_WRITE_PROTECTED));
    }

    #[test]
    fn fsm_program_splits_at_page_boundary() {
        let req = LongOpReq {
            access: DeviceAccess::ReadWrite,
            kind: LongOpKind::Program { page_size: 256 },
            offset: 0x10,
            len: 300,
        };
        let mut fsm = LongOpFsm::new(req, 3);

        let first = fsm.step(OpEvent::Start);
        assert_eq!(
            first,
            OpStep::Command(OpCmd::ProgramChunk {
                offset: 0x10,
                len: 240
            })
        );

        let progressed = fsm.step(OpEvent::HwReady);
        assert_eq!(progressed, OpStep::None);

        let second = fsm.step(OpEvent::Tick);
        assert_eq!(
            second,
            OpStep::Command(OpCmd::ProgramChunk {
                offset: 0x100,
                len: 60
            })
        );
    }

    #[test]
    fn fsm_erase_emits_multiple_chunks() {
        let req = LongOpReq {
            access: DeviceAccess::ReadWrite,
            kind: LongOpKind::Erase { sector_size: 4096 },
            offset: 0,
            len: 8192,
        };
        let mut fsm = LongOpFsm::new(req, 3);

        let first = fsm.step(OpEvent::Start);
        assert_eq!(
            first,
            OpStep::Command(OpCmd::EraseChunk {
                offset: 0,
                len: 4096
            })
        );

        let progressed = fsm.step(OpEvent::HwReady);
        assert_eq!(progressed, OpStep::None);

        let second = fsm.step(OpEvent::Tick);
        assert_eq!(
            second,
            OpStep::Command(OpCmd::EraseChunk {
                offset: 4096,
                len: 4096
            })
        );

        let progressed2 = fsm.step(OpEvent::HwReady);
        assert_eq!(progressed2, OpStep::Complete);
    }

    #[test]
    fn fsm_times_out_after_busy_retries() {
        let req = LongOpReq {
            access: DeviceAccess::ReadWrite,
            kind: LongOpKind::Erase { sector_size: 4096 },
            offset: 0,
            len: 4096,
        };
        let mut fsm = LongOpFsm::new(req, 2);

        let started = fsm.step(OpEvent::Start);
        assert_eq!(
            started,
            OpStep::Command(OpCmd::EraseChunk {
                offset: 0,
                len: 4096
            })
        );

        let poll1 = fsm.step(OpEvent::HwBusy);
        assert_eq!(poll1, OpStep::Command(OpCmd::Poll));

        let poll2 = fsm.step(OpEvent::HwBusy);
        assert_eq!(poll2, OpStep::Command(OpCmd::Poll));

        let timed_out = fsm.step(OpEvent::HwBusy);
        assert_eq!(timed_out, OpStep::Failed(error::FLASH_SMC_TIMEOUT));
    }

    #[test]
    fn fsm_aborts_on_hardware_error() {
        let req = LongOpReq {
            access: DeviceAccess::ReadWrite,
            kind: LongOpKind::Program { page_size: 256 },
            offset: 0,
            len: 64,
        };
        let mut fsm = LongOpFsm::new(req, 3);

        let started = fsm.step(OpEvent::Start);
        assert_eq!(
            started,
            OpStep::Command(OpCmd::ProgramChunk { offset: 0, len: 64 })
        );

        let failed = fsm.step(OpEvent::HwError(error::FLASH_SMC_HARDWARE_ERROR));
        assert_eq!(failed, OpStep::Failed(error::FLASH_SMC_HARDWARE_ERROR));
    }
}
