// Copyright 2026 Matt Franklin
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Shared in-crate fake `CommandId`/`CommandTable` for `cat-server`'s own
//! `tcp`/`udp` listener test modules — never a real radio crate, mirroring
//! how `cat-client`'s and `cat-framework`'s own tests build a small fake
//! table rather than importing a concrete radio's command set.
//!
//! `crate::broker`'s own test module keeps a separate, slightly richer
//! inline copy (it needs a couple of extra selector/action forms to
//! exercise `CommandTable::parse`'s operation classification directly) —
//! this one is intentionally minimal, sized for the `tcp`/`udp` end-to-end
//! tests that only need query/set/action round trips.

use cat_framework::{CommandDefinition, CommandForm, CommandOperation, CommandTable};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakeCommand {
    Frequency,
    Information,
    Transmit,
}

const QUERY: &[CommandForm] = &[CommandForm::fixed(CommandOperation::Query, 0)];
const SET_11: &[CommandForm] = &[CommandForm::fixed(CommandOperation::Set, 11)];
const ACTION: &[CommandForm] = &[CommandForm::fixed(CommandOperation::Action, 0)];
const NONE: &[CommandForm] = &[];

static DEFINITIONS: &[CommandDefinition<FakeCommand>] = &[
    CommandDefinition {
        id: FakeCommand::Frequency,
        code: "FA",
        name: "Frequency",
        description: "Test frequency",
        query_forms: QUERY,
        set_forms: SET_11,
        action_forms: NONE,
        response_forms: NONE,
        readable: true,
        writable: true,
    },
    CommandDefinition {
        id: FakeCommand::Information,
        code: "IF",
        name: "Information",
        description: "Test read-only information",
        query_forms: QUERY,
        set_forms: NONE,
        action_forms: NONE,
        response_forms: NONE,
        readable: true,
        writable: false,
    },
    CommandDefinition {
        id: FakeCommand::Transmit,
        code: "TX",
        name: "Transmit",
        description: "Test parameterless action",
        query_forms: NONE,
        set_forms: NONE,
        action_forms: ACTION,
        response_forms: NONE,
        readable: false,
        writable: true,
    },
];

pub static TABLE: CommandTable<FakeCommand> = CommandTable::new(DEFINITIONS);

// ---------------------------------------------------------------------
// Release-lines fixtures (ADR 0022), shared by the Linux and Windows
// broker tests so both backends are held to the same script.
// ---------------------------------------------------------------------

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cat_framework::ResponseDisposition;
use cat_transport_core::{CatSession, ModemControlLines, TransportError};

/// What happened, in order: CAT exchanges beginning/ending and every line
/// change, tagged with which handle made it (`primary` = the session's own
/// borrowed lines, `release` = the owned duplicate).
pub type EventLog = Arc<Mutex<Vec<String>>>;

/// Modem lines that record into an [`EventLog`].
pub struct RecordingLines {
    tag: &'static str,
    log: EventLog,
    /// Incremented when this value is dropped -- how a test sees the
    /// broker close its duplicate.
    drops: Arc<AtomicUsize>,
}

impl ModemControlLines for RecordingLines {
    fn set_rts(&self, asserted: bool) -> Result<(), TransportError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("{} rts={asserted}", self.tag));
        Ok(())
    }
    fn set_dtr(&self, asserted: bool) -> Result<(), TransportError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("{} dtr={asserted}", self.tag));
        Ok(())
    }
    fn read_cts(&self) -> Result<bool, TransportError> {
        Ok(false)
    }
    fn read_dsr(&self) -> Result<bool, TransportError> {
        Ok(false)
    }
    fn read_dcd(&self) -> Result<bool, TransportError> {
        Ok(false)
    }
}

impl Drop for RecordingLines {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

/// A session whose `FA;` exchange never answers -- it hangs until the
/// broker's request timeout cancels it, exactly like a radio that has
/// stopped talking mid-read. Every other request answers at once.
pub struct StallingSession {
    log: EventLog,
    primary: RecordingLines,
    with_release_lines: bool,
    release_drops: Arc<AtomicUsize>,
}

impl StallingSession {
    pub fn new(with_release_lines: bool) -> Self {
        let log: EventLog = Arc::default();
        Self {
            primary: RecordingLines {
                tag: "primary",
                log: Arc::clone(&log),
                drops: Arc::default(),
            },
            log,
            with_release_lines,
            release_drops: Arc::default(),
        }
    }

    pub fn log(&self) -> EventLog {
        Arc::clone(&self.log)
    }

    /// How many release-line duplicates handed out have since been dropped.
    pub fn release_drops(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.release_drops)
    }
}

struct ExchangeOver(EventLog, String);

impl Drop for ExchangeOver {
    fn drop(&mut self) {
        self.0.lock().unwrap().push(format!("cat over {}", self.1));
    }
}

pub fn logged(log: &EventLog) -> Vec<String> {
    log.lock().unwrap().clone()
}

#[async_trait(?Send)]
impl CatSession for StallingSession {
    type Error = TransportError;

    async fn execute(
        &mut self,
        request: &[u8],
        response: &mut Vec<u8>,
    ) -> Result<ResponseDisposition, Self::Error> {
        let req = String::from_utf8_lossy(request).into_owned();
        self.log.lock().unwrap().push(format!("cat begin {req}"));
        // Logs "cat over" however the exchange ends -- answered, or
        // cancelled by the broker's timeout -- so a test can see whether
        // something happened during the exchange or after it.
        let _over = ExchangeOver(Arc::clone(&self.log), req);
        if request == b"FA;" {
            std::future::pending::<()>().await;
        }
        response.extend_from_slice(b"IF017;");
        Ok(ResponseDisposition::ResponseWritten)
    }

    fn modem_lines(&self) -> Option<&dyn ModemControlLines> {
        Some(&self.primary)
    }

    fn release_lines(&self) -> Option<Box<dyn ModemControlLines + Send + Sync>> {
        self.with_release_lines.then(|| {
            Box::new(RecordingLines {
                tag: "release",
                log: Arc::clone(&self.log),
                drops: Arc::clone(&self.release_drops),
            }) as Box<dyn ModemControlLines + Send + Sync>
        })
    }
}
