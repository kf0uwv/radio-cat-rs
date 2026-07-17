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
