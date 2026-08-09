//! Generic CAT command table and frame-processing lifecycle.
//!
//! This module is radio-independent, and — since
//! `docs/adr/0009-civ-engine-for-binary-addressed-protocols.md` — also
//! wire-protocol-independent: it is generic over a [`crate::wire_format::CatWireFormat`]
//! implementation (defaulting to [`crate::wire_format::AsciiLineFormat`], the
//! Kenwood/Yaesu-style ASCII shape this crate always supported). It knows how
//! to look up command codes, classify wire frames as query/set/action
//! operations, perform basic structural validation, and delegate command
//! semantics to a radio-specific [`CatRadio`] implementation.

use std::marker::PhantomData;

use thiserror::Error;

use crate::wire_format::{AsciiLineFormat, CatWireFormat};

/// Marker trait for radio-owned command identifiers.
pub trait CommandId: Copy + Clone + Eq + core::fmt::Debug + Send + Sync + 'static {}

impl<T> CommandId for T where T: Copy + Clone + Eq + core::fmt::Debug + Send + Sync + 'static {}

/// CAT command operation identified from a wire frame.
///
/// Format-agnostic — a pure classification of what a frame *means*
/// (read/write/act/answer), independent of how that frame is spelled on
/// the wire. Unchanged by the `CatWireFormat` generalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOperation {
    /// Query/read operation.
    Query,
    /// Set/write operation with parameters.
    Set,
    /// Parameterless action operation.
    Action,
    /// Response form metadata.
    Response,
}

/// Structural form accepted for a command operation.
///
/// Format-agnostic — `min_len`/`max_len` count bytes of the parameter/data
/// region after the command code, whatever that region's encoding turns
/// out to be (ASCII digits or binary BCD). Unchanged by the
/// `CatWireFormat` generalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandForm {
    /// Operation represented by this form.
    pub operation: CommandOperation,
    /// Minimum payload width after the command code.
    pub min_len: usize,
    /// Maximum payload width after the command code.
    pub max_len: usize,
    /// `true` if this form, despite being structurally [`CommandOperation::Set`]
    /// (a non-empty parameter was needed to select it — the generic
    /// `Query`/`Action` forms recognized by [`CommandTable::parse`] are
    /// always zero-width), is semantically a **read**: a "selector read"
    /// like the FT-991A's `MD0;` (query the current mode) or `EX047;`
    /// (query one settings-menu item) — a required selector parameter with
    /// no zero-width query form to express it. Always `false` for forms
    /// built via [`CommandForm::fixed`]/[`CommandForm::variable`]; only
    /// `true` for forms built via [`CommandForm::selector_read`].
    ///
    /// Consulted by generic dispatch code that needs to choose between a
    /// session's read-and-await-response path and its fire-and-forget
    /// write path for an otherwise-ambiguous `Set`-shaped request on a
    /// command that is both `readable` and `writable` (e.g.
    /// `cat-server::Broker::dispatch`) — a radio's own `CatRadio::
    /// handle_command` already disambiguates this by parameter width
    /// directly and does not need this field.
    pub is_selector_read: bool,
}

impl CommandForm {
    /// Create a fixed-width form.
    pub const fn fixed(operation: CommandOperation, len: usize) -> Self {
        Self {
            operation,
            min_len: len,
            max_len: len,
            is_selector_read: false,
        }
    }

    /// Create a variable-width form.
    pub const fn variable(operation: CommandOperation, min_len: usize, max_len: usize) -> Self {
        Self {
            operation,
            min_len,
            max_len,
            is_selector_read: false,
        }
    }

    /// Create a fixed-width, structurally-`Set` "selector read" form: a
    /// required selector parameter with no response-producing zero-width
    /// query form to express it (see [`CommandForm::is_selector_read`]).
    /// Fixed-width only — every known selector-read form across this
    /// workspace's radios is a single fixed width, distinct from its
    /// command's real write width(s).
    pub const fn selector_read(len: usize) -> Self {
        Self {
            operation: CommandOperation::Set,
            min_len: len,
            max_len: len,
            is_selector_read: true,
        }
    }

    fn matches(&self, operation: CommandOperation, len: usize) -> bool {
        self.operation == operation && (self.min_len..=self.max_len).contains(&len)
    }
}

/// Radio-specific command definition stored in a generic table.
///
/// Generic over `F: CatWireFormat` since [`Self::code`]'s shape is
/// protocol-specific (a 2-character ASCII string for
/// [`crate::wire_format::AsciiLineFormat`]; expected to be a
/// `(cmd, subcmd)` byte pair for a future CI-V implementation). Everything
/// else here — forms, readable/writable — is unchanged from before the
/// `CatWireFormat` generalization.
#[derive(Debug, Clone, Copy)]
pub struct CommandDefinition<C: CommandId, F: CatWireFormat = AsciiLineFormat> {
    /// Radio-owned identifier.
    pub id: C,
    /// Wire command code — `F::Code`'s shape depends on the protocol.
    pub code: F::Code,
    /// Human-readable command name.
    pub name: &'static str,
    /// Human-readable command description.
    pub description: &'static str,
    /// Legal query forms.
    pub query_forms: &'static [CommandForm],
    /// Legal set forms.
    pub set_forms: &'static [CommandForm],
    /// Legal action forms.
    pub action_forms: &'static [CommandForm],
    /// Legal response forms.
    pub response_forms: &'static [CommandForm],
    /// Whether a controller may query (read) this command.
    ///
    /// This is the documented controller-facing capability, which may differ
    /// from the wire-grammar forms: some reads take a selector parameter (e.g.
    /// an S-meter or memory read) that the query/set/action form model would
    /// otherwise classify as a `Set`.
    pub readable: bool,
    /// Whether a controller may set (write) this command, including
    /// parameterless action writes.
    pub writable: bool,
}

impl<C: CommandId, F: CatWireFormat> CommandDefinition<C, F> {
    /// Return true when any form for `operation` accepts `param_len`.
    pub fn supports(&self, operation: CommandOperation, param_len: usize) -> bool {
        let forms = match operation {
            CommandOperation::Query => self.query_forms,
            CommandOperation::Set => self.set_forms,
            CommandOperation::Action => self.action_forms,
            CommandOperation::Response => self.response_forms,
        };
        forms.iter().any(|form| form.matches(operation, param_len))
    }

    /// Whether a controller may query (read) this command.
    ///
    /// Returns the documented [`readable`](Self::readable) capability.
    pub fn is_readable(&self) -> bool {
        self.readable
    }

    /// Whether a controller may set (write) this command.
    ///
    /// Returns the documented [`writable`](Self::writable) capability, which
    /// includes parameterless action writes such as `TX`.
    pub fn is_writable(&self) -> bool {
        self.writable
    }

    /// Whether the `Set`-shaped form matching a `param_len`-byte parameter
    /// is actually a "selector read" (see [`CommandForm::is_selector_read`])
    /// rather than a write. `false` for any `param_len` that doesn't match
    /// one of this command's `set_forms` at all.
    pub fn is_selector_read(&self, param_len: usize) -> bool {
        self.set_forms
            .iter()
            .any(|form| form.matches(CommandOperation::Set, param_len) && form.is_selector_read)
    }
}

/// Static command table generic over a radio-defined command identifier
/// and, since ADR 0009, a wire protocol.
///
/// `F` defaults to [`AsciiLineFormat`], so every existing
/// `static TABLE: CommandTable<SomeCommandId> = CommandTable::new(...)`
/// declaration keeps compiling and behaving exactly as before —
/// `CommandTable` itself never owns a runtime `F` *value* (it stays a
/// plain `'static` definitions table, constructible in `const` context);
/// only [`CommandTable::parse`] borrows a caller-supplied `&F` instance,
/// for the one operation (splitting a raw frame apart) that can depend on
/// a protocol's own runtime configuration.
#[derive(Debug)]
pub struct CommandTable<C: CommandId, F: CatWireFormat = AsciiLineFormat> {
    definitions: &'static [CommandDefinition<C, F>],
}

impl<C: CommandId, F: CatWireFormat> CommandTable<C, F> {
    /// Create a table from static command definitions.
    pub const fn new(definitions: &'static [CommandDefinition<C, F>]) -> Self {
        Self { definitions }
    }

    /// Return all command definitions.
    pub fn definitions(&self) -> &'static [CommandDefinition<C, F>] {
        self.definitions
    }

    /// Find a command definition by its already-known code (as opposed to
    /// parsing one out of a raw frame — see [`Self::parse`]). Used by
    /// callers that already hold a code value, e.g. `cat-client`
    /// validating a command name a caller passed in directly, or
    /// `ts570d`'s/`ft991a`'s own UI code looking up a command's
    /// description by name.
    ///
    /// Generic over `Q` (rather than fixed to `F::Code`, which for
    /// `AsciiLineFormat` is `&'static str`) so a caller holding a
    /// *borrowed*, non-`'static` code — e.g. a `&str` of some shorter
    /// lifetime — can still look it up: `F::Code: PartialEq<Q>` is
    /// satisfied by `&'static str: PartialEq<&'a str>` for any `'a`
    /// (`std`'s blanket impl compares by content, independent of the two
    /// sides' lifetimes — the same mechanism `AsciiLineFormat::
    /// find_command` uses). This is a **restoration**, not new behavior:
    /// this crate's `find` accepted any-lifetime `&str` before the
    /// `CatWireFormat` generalization; an earlier revision of this method
    /// narrowed it to `F::Code` exactly and broke real callers (`ts570d`'s
    /// `emulator/src/tui.rs::lookup_description`) — caught by actually
    /// compiling `ts570d`/`ft991a` against this change, not by inspection.
    pub fn find<Q>(&self, code: Q) -> Option<&'static CommandDefinition<C, F>>
    where
        F::Code: PartialEq<Q>,
    {
        self.definitions
            .iter()
            .find(|definition| definition.code == code)
    }

    /// Parse one complete CAT frame into a generic request, using `format`
    /// to split the frame apart (protocol-specific) and this table to
    /// classify the resulting operation (protocol-agnostic — the same
    /// logic this crate always used).
    pub fn parse<'a>(
        &'static self,
        format: &F,
        frame: &'a [u8],
    ) -> Result<CommandRequest<'a, C, F>, ParseError> {
        let (definition, parameters) = format.find_command(self, frame)?;

        let operation = if parameters.is_empty() && definition.supports(CommandOperation::Query, 0)
        {
            CommandOperation::Query
        } else if parameters.is_empty() && definition.supports(CommandOperation::Action, 0) {
            CommandOperation::Action
        } else if definition.supports(CommandOperation::Set, parameters.len()) {
            CommandOperation::Set
        } else if parameters.is_empty() {
            return Err(ParseError::UnsupportedOperation(format!(
                "{:?}",
                definition.code
            )));
        } else {
            return Err(ParseError::InvalidParameterWidth {
                code: format!("{:?}", definition.code),
                len: parameters.len(),
            });
        };

        Ok(CommandRequest {
            id: definition.id,
            code: definition.code,
            operation,
            parameters: ParameterValues {
                raw: parameters,
                _format: PhantomData,
            },
        })
    }
}

/// Parsed generic CAT request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandRequest<'a, C: CommandId, F: CatWireFormat = AsciiLineFormat> {
    /// Radio-owned identifier.
    pub id: C,
    /// The command's wire code.
    pub code: F::Code,
    /// Parsed operation.
    pub operation: CommandOperation,
    /// Borrowed raw parameter payload.
    pub parameters: ParameterValues<'a, F>,
}

/// Borrowed parameter payload.
///
/// Holds raw bytes generically (`F::Code`/data encoding varies by
/// protocol), but see the `impl<'a> ParameterValues<'a, AsciiLineFormat>`
/// block below: the `AsciiLineFormat` instantiation gets a specialized
/// inherent `impl` reproducing this crate's original `&str`-based
/// `raw()`/`unsigned()` API, unchanged in name and behavior — so every
/// existing `ts570d`/`ft991a` command handler calling
/// `request.parameters.raw()` keeps compiling and behaving identically.
/// See `docs/adr/0009-civ-engine-for-binary-addressed-protocols.md`'s
/// "Verified against real call sites" section for why this shape was
/// chosen over a single generic `&[u8]`-returning method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParameterValues<'a, F: CatWireFormat = AsciiLineFormat> {
    raw: &'a [u8],
    _format: PhantomData<F>,
}

impl<'a, F: CatWireFormat> ParameterValues<'a, F> {
    /// Return the raw parameter/data bytes after the command code, in
    /// whatever encoding this protocol uses (ASCII digits, binary BCD,
    /// ...). Available for any format; see the `AsciiLineFormat`
    /// specialization below for the `&str`-typed convenience existing
    /// callers use.
    pub fn raw_bytes(&self) -> &'a [u8] {
        self.raw
    }
}

impl<'a> ParameterValues<'a, AsciiLineFormat> {
    /// Return the raw parameter payload after the command code, as text.
    ///
    /// Byte-identical behavior to this crate's pre-ADR-0009 `raw()`: every
    /// `AsciiLineFormat` frame is ASCII by construction (validated when the
    /// command code itself was split off — see
    /// `AsciiLineFormat::find_command`), so this is an invariant, not a
    /// possible-failure path a caller needs to handle.
    pub fn raw(&self) -> &'a str {
        core::str::from_utf8(self.raw)
            .expect("AsciiLineFormat parameter bytes are always valid UTF-8")
    }

    /// Parse the full payload as an unsigned integer.
    pub fn unsigned(&self) -> Result<u64, ParameterAccessError> {
        self.raw()
            .parse::<u64>()
            .map_err(|_| ParameterAccessError::InvalidUnsigned)
    }
}

/// Parameter accessor errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ParameterAccessError {
    /// Raw payload was not an unsigned integer.
    #[error("parameter is not an unsigned integer")]
    InvalidUnsigned,
}

/// Generic parse errors before radio-specific handling.
///
/// Format-agnostic. `MissingTerminator` reads as ASCII-flavored naming but
/// applies equally to a missing CI-V `FD` byte — kept as-is rather than
/// renamed, since renaming would be a gratuitous breaking change to
/// existing match arms for no behavior difference.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    /// Frame did not end in a CAT terminator.
    #[error("missing CAT frame terminator")]
    MissingTerminator,
    /// Frame did not contain a command code.
    #[error("invalid CAT frame syntax")]
    InvalidSyntax,
    /// Command code was not in the table.
    #[error("unknown command: {0}")]
    UnknownCommand(String),
    /// Command exists but the requested operation is not legal.
    #[error("unsupported operation for command: {0}")]
    UnsupportedOperation(String),
    /// Parameter payload width did not match any legal form.
    #[error("invalid parameter width for {code}: {len}")]
    InvalidParameterWidth { code: String, len: usize },
}

/// Generic protocol error categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolErrorKind {
    /// Unknown command code.
    UnknownCommand,
    /// Invalid frame syntax.
    InvalidSyntax,
    /// Invalid parameter shape or value.
    InvalidParameter,
    /// Unsupported operation.
    UnsupportedOperation,
}

impl From<&ParseError> for ProtocolErrorKind {
    fn from(value: &ParseError) -> Self {
        match value {
            ParseError::UnknownCommand(_) => ProtocolErrorKind::UnknownCommand,
            ParseError::UnsupportedOperation(_) => ProtocolErrorKind::UnsupportedOperation,
            ParseError::InvalidParameterWidth { .. } => ProtocolErrorKind::InvalidParameter,
            ParseError::MissingTerminator | ParseError::InvalidSyntax => {
                ProtocolErrorKind::InvalidSyntax
            }
        }
    }
}

/// Response disposition reported by radio handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseDisposition {
    /// A response was written to the output buffer.
    ResponseWritten,
    /// No response should be written.
    NoResponse,
    /// Protocol error response was written.
    ProtocolError(ProtocolErrorKind),
}

/// Outcome of one command dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome<E> {
    /// Response disposition.
    pub response: ResponseDisposition,
    /// Radio-specific events produced by state changes.
    pub events: Vec<E>,
}

impl<E> CommandOutcome<E> {
    /// Construct an outcome for a written response.
    pub fn response_written() -> Self {
        Self {
            response: ResponseDisposition::ResponseWritten,
            events: Vec::new(),
        }
    }

    /// Construct an outcome for a silent command.
    pub fn no_response() -> Self {
        Self {
            response: ResponseDisposition::NoResponse,
            events: Vec::new(),
        }
    }
}

/// Error while building a response.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ResponseBuildError {
    /// Response was finished more than once.
    #[error("response already finished")]
    AlreadyFinished,
}

/// Generic response builder over a caller-owned output buffer.
///
/// `new` is available for any `F: CatWireFormat` (construction never
/// needs to know the protocol's encoding). The actual writing methods —
/// [`push_wire_value`](Self::push_wire_value), [`finish`](Self::finish),
/// [`write_complete`](Self::write_complete) — are only defined on the
/// `AsciiLineFormat` specialization below, byte-identical to this crate's
/// pre-ADR-0009 `ResponseBuilder`: a future CI-V response builder needs a
/// structurally different API (it must assemble a full addressed
/// `FE FE...FD` frame, not "push text then append one terminator byte"),
/// so forcing one shared method set across both would misrepresent what
/// each protocol actually needs.
pub struct ResponseBuilder<'a, F: CatWireFormat = AsciiLineFormat> {
    output: &'a mut Vec<u8>,
    finished: bool,
    _format: PhantomData<F>,
}

impl<'a, F: CatWireFormat> ResponseBuilder<'a, F> {
    /// Create a new response builder.
    pub fn new(output: &'a mut Vec<u8>) -> Self {
        Self {
            output,
            finished: false,
            _format: PhantomData,
        }
    }
}

impl<'a> ResponseBuilder<'a, AsciiLineFormat> {
    /// Push raw wire text into the response buffer.
    pub fn push_wire_value(&mut self, value: &str) -> Result<(), ResponseBuildError> {
        if self.finished {
            return Err(ResponseBuildError::AlreadyFinished);
        }
        self.output.extend_from_slice(value.as_bytes());
        Ok(())
    }

    /// Finish the current response by appending the CAT terminator.
    pub fn finish(&mut self) -> Result<(), ResponseBuildError> {
        if self.finished {
            return Err(ResponseBuildError::AlreadyFinished);
        }
        self.output.push(b';');
        self.finished = true;
        Ok(())
    }

    /// Write a complete response string, preserving existing wire behavior.
    pub fn write_complete(&mut self, response: &str) -> Result<(), ResponseBuildError> {
        if self.finished {
            return Err(ResponseBuildError::AlreadyFinished);
        }
        self.output.extend_from_slice(response.as_bytes());
        self.finished = response.ends_with(';');
        Ok(())
    }
}

/// Generic command catalog available without mutable radio execution.
///
/// Generic over `F: CatWireFormat` (default [`AsciiLineFormat`]) since
/// `command_table`'s return type is. Existing `impl CatCommandCatalog for
/// SomeRadio` blocks (no explicit `<F>`) resolve against the default
/// unchanged.
pub trait CatCommandCatalog<F: CatWireFormat = AsciiLineFormat> {
    /// Radio-owned command identifier.
    type CommandId: CommandId;

    /// Return the static command table.
    fn command_table(&self) -> &'static CommandTable<Self::CommandId, F>;
}

/// Radio-specific CAT state machine delegated to by the generic framework.
pub trait CatRadio<F: CatWireFormat = AsciiLineFormat>: CatCommandCatalog<F> {
    /// Radio-specific event type.
    type Event;
    /// Radio-specific error type.
    type Error;

    /// Execute one parsed command request.
    fn handle_command(
        &mut self,
        request: CommandRequest<'_, Self::CommandId, F>,
        response: &mut ResponseBuilder<'_, F>,
    ) -> Result<CommandOutcome<Self::Event>, Self::Error>;

    /// Write a radio-specific protocol error response.
    fn write_protocol_error(
        &mut self,
        _kind: ProtocolErrorKind,
        response: &mut ResponseBuilder<'_, F>,
    ) -> Result<CommandOutcome<Self::Event>, Self::Error>;
}

/// Layered framework error.
#[derive(Debug, Error)]
pub enum CatFrameworkError<E> {
    /// Parse or structural validation failed.
    #[error("parse error: {0}")]
    Parse(ParseError),
    /// Radio-specific handler failed.
    #[error("radio error")]
    Radio(E),
}

/// Generic CAT processor for one radio state machine.
///
/// Owns one `format: F` instance, constructed once — see
/// `docs/adr/0009-civ-engine-for-binary-addressed-protocols.md`. Existing
/// `CatFramework::new(radio)` call sites keep compiling unchanged (`F`
/// defaults to [`AsciiLineFormat`], which is [`Default`]).
pub struct CatFramework<R, F: CatWireFormat = AsciiLineFormat> {
    radio: R,
    format: F,
}

impl<R, F: CatWireFormat + Default> CatFramework<R, F> {
    /// Create a framework around a radio-specific state machine, using
    /// `F`'s default format configuration (the only option for
    /// [`AsciiLineFormat`], which carries none).
    pub fn new(radio: R) -> Self {
        Self::with_format(radio, F::default())
    }
}

impl<R, F: CatWireFormat> CatFramework<R, F> {
    /// Create a framework around a radio-specific state machine and an
    /// explicit format instance — e.g. a CI-V format configured with a
    /// non-default bus address.
    pub fn with_format(radio: R, format: F) -> Self {
        Self { radio, format }
    }

    /// Access the underlying radio state immutably.
    pub fn radio(&self) -> &R {
        &self.radio
    }
}

impl<R, F> CatFramework<R, F>
where
    R: CatRadio<F>,
    F: CatWireFormat,
{
    /// Process one complete CAT frame.
    ///
    /// Takes `impl AsRef<[u8]>` rather than a concrete `&[u8]` specifically
    /// so existing `&str` call sites (`framework.process_frame("FA;", ...)`,
    /// used throughout `ts570d`'s/`ft991a`'s own test suites) keep
    /// compiling unchanged — `str` already implements `AsRef<[u8]>` in
    /// `std`, and a future CI-V caller passes a real `&[u8]` frame through
    /// the same bound.
    pub fn process_frame(
        &mut self,
        frame: impl AsRef<[u8]>,
        output: &mut Vec<u8>,
    ) -> Result<CommandOutcome<R::Event>, CatFrameworkError<R::Error>> {
        match self
            .radio
            .command_table()
            .parse(&self.format, frame.as_ref())
        {
            Ok(request) => {
                let mut response = ResponseBuilder::new(output);
                self.radio
                    .handle_command(request, &mut response)
                    .map_err(CatFrameworkError::Radio)
            }
            Err(err) => {
                let kind = ProtocolErrorKind::from(&err);
                let mut response = ResponseBuilder::new(output);
                self.radio
                    .write_protocol_error(kind, &mut response)
                    .map_err(CatFrameworkError::Radio)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum TestCommand {
        Frequency,
        Ping,
    }

    const QUERY: &[CommandForm] = &[CommandForm::fixed(CommandOperation::Query, 0)];
    const SET_11: &[CommandForm] = &[CommandForm::fixed(CommandOperation::Set, 11)];
    const ACTION: &[CommandForm] = &[CommandForm::fixed(CommandOperation::Action, 0)];
    const NONE: &[CommandForm] = &[];

    static DEFINITIONS: &[CommandDefinition<TestCommand>] = &[
        CommandDefinition {
            id: TestCommand::Frequency,
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
            id: TestCommand::Ping,
            code: "PG",
            name: "Ping",
            description: "Test action",
            query_forms: NONE,
            set_forms: NONE,
            action_forms: ACTION,
            response_forms: NONE,
            readable: false,
            writable: true,
        },
    ];

    static TABLE: CommandTable<TestCommand> = CommandTable::new(DEFINITIONS);

    #[test]
    fn command_lookup_finds_definition() {
        assert_eq!(TABLE.find("FA").unwrap().id, TestCommand::Frequency);
    }

    #[test]
    fn parses_query_form() {
        let request = TABLE.parse(&AsciiLineFormat, b"FA;").unwrap();
        assert_eq!(request.operation, CommandOperation::Query);
        assert_eq!(request.parameters.raw(), "");
    }

    #[test]
    fn parses_set_form() {
        let request = TABLE.parse(&AsciiLineFormat, b"FA00014230000;").unwrap();
        assert_eq!(request.operation, CommandOperation::Set);
        assert_eq!(request.parameters.raw(), "00014230000");
    }

    #[test]
    fn parses_action_form() {
        let request = TABLE.parse(&AsciiLineFormat, b"PG;").unwrap();
        assert_eq!(request.operation, CommandOperation::Action);
    }

    #[test]
    fn rejects_missing_terminator() {
        assert!(matches!(
            TABLE.parse(&AsciiLineFormat, b"FA"),
            Err(ParseError::MissingTerminator)
        ));
    }

    #[test]
    fn rejects_unknown_command() {
        assert!(matches!(
            TABLE.parse(&AsciiLineFormat, b"ZZ;"),
            Err(ParseError::UnknownCommand(code)) if code == "ZZ"
        ));
    }

    #[test]
    fn rejects_wrong_width() {
        assert!(matches!(
            TABLE.parse(&AsciiLineFormat, b"FA123;"),
            Err(ParseError::InvalidParameterWidth { code, len }) if code == "\"FA\"" && len == 3
        ));
    }

    #[test]
    fn response_builder_preserves_leading_zeroes() {
        let mut out = Vec::new();
        let mut response = ResponseBuilder::<AsciiLineFormat>::new(&mut out);
        response.push_wire_value("FA00014230000").unwrap();
        response.finish().unwrap();
        assert_eq!(out, b"FA00014230000;");
    }

    #[test]
    fn process_frame_accepts_str_literal_unchanged() {
        // Guards the `impl AsRef<[u8]>` choice on `CatFramework::process_frame`
        // — every existing `ts570d`/`ft991a` test/call site passes a bare
        // `&str` literal directly.
        struct EchoRadio;
        impl CatCommandCatalog for EchoRadio {
            type CommandId = TestCommand;
            fn command_table(&self) -> &'static CommandTable<Self::CommandId> {
                &TABLE
            }
        }
        impl CatRadio for EchoRadio {
            type Event = ();
            type Error = std::convert::Infallible;
            fn handle_command(
                &mut self,
                _request: CommandRequest<'_, Self::CommandId>,
                response: &mut ResponseBuilder<'_>,
            ) -> Result<CommandOutcome<Self::Event>, Self::Error> {
                response.write_complete("FA00014230000;").unwrap();
                Ok(CommandOutcome::response_written())
            }
            fn write_protocol_error(
                &mut self,
                _kind: ProtocolErrorKind,
                _response: &mut ResponseBuilder<'_>,
            ) -> Result<CommandOutcome<Self::Event>, Self::Error> {
                Ok(CommandOutcome::no_response())
            }
        }

        let mut framework = CatFramework::new(EchoRadio);
        let mut output = Vec::new();
        framework.process_frame("FA;", &mut output).unwrap();
        assert_eq!(output, b"FA00014230000;");
    }
}
