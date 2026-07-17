//! Generic, radio-independent CAT command engine.
//!
//! Extracted from `ts570d`'s `framework/src/cat.rs` (commit `1585e1e`,
//! `refactor/generic-cat-framework` branch), per
//! `docs/adr/0001-scope-and-crate-boundaries.md`.
//!
//! This crate provides the **generic CAT engine**: command table modeling,
//! syntactic parsing, structural validation, the dispatch lifecycle, and
//! response construction — all generic over a radio-defined [`cat::CommandId`].
//!
//! `cat-framework` contains **no radio-specific** command definitions, modes,
//! frequencies, state, or handlers. A radio crate supplies those and
//! implements [`cat::CatRadio`] to receive parsed commands.

pub mod cat;

pub use cat::{
    CatCommandCatalog, CatFramework, CatFrameworkError, CatRadio, CommandDefinition, CommandForm,
    CommandId, CommandOperation, CommandOutcome, CommandRequest, CommandTable,
    ParameterAccessError, ParameterValues, ParseError, ProtocolErrorKind, ResponseBuildError,
    ResponseBuilder, ResponseDisposition,
};
