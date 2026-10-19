// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS elevated execution.
//!
//! Nothing runs with admin rights here yet (that needs an authorization
//! prompt via a helper), so starting an elevated step fails up front instead
//! of pretending it ran and finished with exit code 0.

use std::path::Path;

use crate::i18n::tr;

pub enum Error {
    Declined,
    Other(String),
}

pub struct Process;

impl Process {
    /// No process is ever started, so there is no exit code to report.
    pub fn exit_code(&self) -> Option<u32> {
        None
    }
}

pub fn start(_program: &Path, _params: &str) -> Result<Process, Error> {
    Err(Error::Other(tr!("Setup", "Steps that need administrator rights aren't available on macOS yet.")))
}
