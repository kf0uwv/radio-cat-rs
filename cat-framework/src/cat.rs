//! Generic CAT command table and frame-processing lifecycle.
//!
//! This module is radio-independent. It knows how to look up command codes,
//! classify wire frames as query/set/action operations, perform basic
//! structural validation, and delegate command semantics to a radio-specific
//! [`CatRadio`] implementation.

use thiserror::Error;

/// Marker trait for radio-owned command identifiers.
pub trait CommandId: Copy + Clone + Eq + core::fmt::Debug + Send + Sync + 'static {}

impl<T> CommandId for T where T: Copy + Clone + Eq + core::fmt::Debug + Send + Sync + 'static {}

/// CAT command operation identified from a wire frame.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandForm {
    /// Operation represented by this form.
    pub operation: CommandOperation,
    /// Minimum payload width after the command code.
    pub min_len: usize,
    /// Maximum payload width after the command code.
    pub max_len: usize,
}

impl CommandForm {
    /// Create a fixed-width form.
    pub const fn fixed(operation: CommandOperation, len: usize) -> Self {
        Self {
            operation,
            min_len: len,
            max_len: len,
        }
    }

    /// Create a variable-width form.
    pub const fn variable(operation: CommandOperation, min_len: usize, max_len: usize) -> Self {
        Self {
            operation,
            min_len,
            max_len,
        }
    }

    fn matches(&self, operation: CommandOperation, len: usize) -> bool {
        self.operation == operation && (self.min_len..=self.max_len).contains(&len)
    }
}

/// Radio-specific command definition stored in a generic table.
#[derive(Debug, Clone, Copy)]
pub struct CommandDefinition<C: CommandId> {
    /// Radio-owned identifier.
    pub id: C,
    /// Wire command code, normally two ASCII characters.
    pub code: &'static str,
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

impl<C: CommandId> CommandDefinition<C> {
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
}

/// Static command table generic over a radio-defined command identifier.
#[derive(Debug)]
pub struct CommandTable<C: CommandId> {
    definitions: &'static [CommandDefinition<C>],
}

impl<C: CommandId> CommandTable<C> {
    /// Create a table from static command definitions.
    pub const fn new(definitions: &'static [CommandDefinition<C>]) -> Self {
        Self { definitions }
    }

    /// Return all command definitions.
    pub fn definitions(&self) -> &'static [CommandDefinition<C>] {
        self.definitions
    }

    /// Find a command definition by wire code.
    pub fn find(&self, code: &str) -> Option<&'static CommandDefinition<C>> {
        self.definitions
            .iter()
            .find(|definition| definition.code == code)
    }

    /// Parse one complete CAT frame into a generic request.
    pub fn parse<'a>(&'static self, frame: &'a str) -> Result<CommandRequest<'a, C>, ParseError> {
        let frame = frame
            .strip_suffix(';')
            .ok_or(ParseError::MissingTerminator)?;
        if frame.len() < 2 {
            return Err(ParseError::InvalidSyntax);
        }

        let (code, parameters) = frame.split_at(2);
        let definition = self
            .find(code)
            .ok_or_else(|| ParseError::UnknownCommand(code.to_string()))?;

        let operation = if parameters.is_empty() && definition.supports(CommandOperation::Query, 0)
        {
            CommandOperation::Query
        } else if parameters.is_empty() && definition.supports(CommandOperation::Action, 0) {
            CommandOperation::Action
        } else if definition.supports(CommandOperation::Set, parameters.len()) {
            CommandOperation::Set
        } else if parameters.is_empty() {
            return Err(ParseError::UnsupportedOperation(code.to_string()));
        } else {
            return Err(ParseError::InvalidParameterWidth {
                code: code.to_string(),
                len: parameters.len(),
            });
        };

        Ok(CommandRequest {
            id: definition.id,
            code: definition.code,
            operation,
            parameters: ParameterValues { raw: parameters },
        })
    }
}

/// Parsed generic CAT request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandRequest<'a, C: CommandId> {
    /// Radio-owned identifier.
    pub id: C,
    /// Static wire command code.
    pub code: &'static str,
    /// Parsed operation.
    pub operation: CommandOperation,
    /// Borrowed raw parameter payload.
    pub parameters: ParameterValues<'a>,
}

/// Borrowed parameter payload with convenience accessors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParameterValues<'a> {
    raw: &'a str,
}

impl<'a> ParameterValues<'a> {
    /// Return the raw parameter payload after the command code.
    pub fn raw(&self) -> &'a str {
        self.raw
    }

    /// Parse the full payload as an unsigned integer.
    pub fn unsigned(&self) -> Result<u64, ParameterAccessError> {
        self.raw
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
pub struct ResponseBuilder<'a> {
    output: &'a mut Vec<u8>,
    finished: bool,
}

impl<'a> ResponseBuilder<'a> {
    /// Create a new response builder.
    pub fn new(output: &'a mut Vec<u8>) -> Self {
        Self {
            output,
            finished: false,
        }
    }

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
pub trait CatCommandCatalog {
    /// Radio-owned command identifier.
    type CommandId: CommandId;

    /// Return the static command table.
    fn command_table(&self) -> &'static CommandTable<Self::CommandId>;
}

/// Radio-specific CAT state machine delegated to by the generic framework.
pub trait CatRadio: CatCommandCatalog {
    /// Radio-specific event type.
    type Event;
    /// Radio-specific error type.
    type Error;

    /// Execute one parsed command request.
    fn handle_command(
        &mut self,
        request: CommandRequest<'_, Self::CommandId>,
        response: &mut ResponseBuilder<'_>,
    ) -> Result<CommandOutcome<Self::Event>, Self::Error>;

    /// Write a radio-specific protocol error response.
    fn write_protocol_error(
        &mut self,
        _kind: ProtocolErrorKind,
        response: &mut ResponseBuilder<'_>,
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
pub struct CatFramework<R> {
    radio: R,
}

impl<R> CatFramework<R> {
    /// Create a framework around a radio-specific state machine.
    pub fn new(radio: R) -> Self {
        Self { radio }
    }

    /// Access the underlying radio state immutably.
    pub fn radio(&self) -> &R {
        &self.radio
    }
}

impl<R> CatFramework<R>
where
    R: CatRadio,
{
    /// Process one complete CAT frame.
    pub fn process_frame(
        &mut self,
        frame: &str,
        output: &mut Vec<u8>,
    ) -> Result<CommandOutcome<R::Event>, CatFrameworkError<R::Error>> {
        match self.radio.command_table().parse(frame) {
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
        let request = TABLE.parse("FA;").unwrap();
        assert_eq!(request.operation, CommandOperation::Query);
        assert_eq!(request.parameters.raw(), "");
    }

    #[test]
    fn parses_set_form() {
        let request = TABLE.parse("FA00014230000;").unwrap();
        assert_eq!(request.operation, CommandOperation::Set);
        assert_eq!(request.parameters.raw(), "00014230000");
    }

    #[test]
    fn parses_action_form() {
        let request = TABLE.parse("PG;").unwrap();
        assert_eq!(request.operation, CommandOperation::Action);
    }

    #[test]
    fn rejects_missing_terminator() {
        assert!(matches!(
            TABLE.parse("FA"),
            Err(ParseError::MissingTerminator)
        ));
    }

    #[test]
    fn rejects_unknown_command() {
        assert!(matches!(
            TABLE.parse("ZZ;"),
            Err(ParseError::UnknownCommand(code)) if code == "ZZ"
        ));
    }

    #[test]
    fn rejects_wrong_width() {
        assert!(matches!(
            TABLE.parse("FA123;"),
            Err(ParseError::InvalidParameterWidth { code, len }) if code == "FA" && len == 3
        ));
    }

    #[test]
    fn response_builder_preserves_leading_zeroes() {
        let mut out = Vec::new();
        let mut response = ResponseBuilder::new(&mut out);
        response.push_wire_value("FA00014230000").unwrap();
        response.finish().unwrap();
        assert_eq!(out, b"FA00014230000;");
    }
}
