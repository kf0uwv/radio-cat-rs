//! Generic CAT command sender.
//!
//! [`CatClient`] wraps any [`CatSession`] implementation and provides
//! high-level `query`/`query_with_param`/`set` methods that validate command
//! codes against a radio-supplied `&'static CommandTable<C>` before placing
//! bytes on the wire — genericized from `ts570d`'s `radio/src/client.rs`
//! (`RadioClient<S: CatSession<Error = TransportError>>`, commit `1585e1e`),
//! which hardcoded `Ts570dCommandId`/`TS570D_COMMAND_TABLE` and returned the
//! TS-570D-specific `RadioError`. See
//! `planning/cat_framework/task_plan.md`'s Task 3 section for the full
//! design proposal.
//!
//! # Wire format
//!
//! - **Query**: `<CMD>;`          e.g. `FA;`
//! - **Set**:   `<CMD><params>;`  e.g. `FA00014250000;`
//! - **Response** (query only): `<CMD><data>;` read back from the radio

use cat_framework::{
    CommandDefinition, CommandId, CommandTable, ProtocolErrorKind, ResponseDisposition,
};
use cat_transport_core::CatSession;
use thiserror::Error;

/// Sends CAT commands over a [`CatSession`] and reads back responses.
///
/// Generic over `C`, the radio-supplied [`CommandId`], and `S`, the
/// [`CatSession`] implementation. Contains no radio-specific or
/// transport-specific types — a radio crate supplies `table` at
/// construction, and any `CatSession` implementation (serial, TCP, UDP, or a
/// test double) can be wrapped unchanged.
///
/// All methods validate the command code against `table` before touching
/// the session.
pub struct CatClient<C: CommandId, S: CatSession> {
    pub(crate) session: S,
    table: &'static CommandTable<C>,
}

impl<C, S> CatClient<C, S>
where
    C: CommandId,
    S: CatSession,
    S::Error: std::error::Error + 'static,
{
    /// Create a new `CatClient` wrapping `session`, validating commands
    /// against `table`.
    pub fn new(session: S, table: &'static CommandTable<C>) -> Self {
        Self { session, table }
    }

    /// Send a query command and return the radio's response string.
    ///
    /// Formats the wire bytes as `"<code>;"` and returns the response the
    /// session reports back for that exchange.
    ///
    /// # Errors
    ///
    /// - [`ClientError::UnknownCommand`] — `code` is not in the command table
    /// - [`ClientError::CommandNotReadable`] — command does not support read
    /// - [`ClientError::Transport`] — I/O error on the underlying session
    pub async fn query(&mut self, code: &str) -> Result<String, ClientError<S::Error>> {
        let meta = self.validate_code(code)?;
        if !meta.is_readable() {
            return Err(ClientError::CommandNotReadable(code.to_string()));
        }

        let wire = format!("{};", code);
        self.execute_query(wire.as_bytes()).await
    }

    /// Send a query command with a parameter prefix and return the radio's
    /// response string.
    ///
    /// Formats the wire bytes as `"<code><params>;"`. Use this when the
    /// command requires a selector or sub-address appended directly to the
    /// command code before the semicolon, e.g. `"SM0;"` or `"RM1;"`.
    ///
    /// # Errors
    ///
    /// - [`ClientError::UnknownCommand`] — `code` is not in the command table
    /// - [`ClientError::CommandNotReadable`] — command does not support read
    /// - [`ClientError::Transport`] — I/O error on the underlying session
    pub async fn query_with_param(
        &mut self,
        code: &str,
        params: &str,
    ) -> Result<String, ClientError<S::Error>> {
        let meta = self.validate_code(code)?;
        if !meta.is_readable() {
            return Err(ClientError::CommandNotReadable(code.to_string()));
        }

        let wire = format!("{}{};", code, params);
        self.execute_query(wire.as_bytes()).await
    }

    /// Send a set command with parameters.
    ///
    /// Formats the wire bytes as `"<code><params>;"` and does not wait for a
    /// response — delegates to [`CatSession::send`], which real sessions
    /// implement without blocking on a read.
    ///
    /// # Errors
    ///
    /// - [`ClientError::UnknownCommand`] — `code` is not in the command table
    /// - [`ClientError::CommandNotWritable`] — command does not support write
    /// - [`ClientError::Transport`] — I/O error on the underlying session
    pub async fn set(&mut self, code: &str, params: &str) -> Result<(), ClientError<S::Error>> {
        let meta = self.validate_code(code)?;
        if !meta.is_writable() {
            return Err(ClientError::CommandNotWritable(code.to_string()));
        }

        let wire = format!("{}{};", code, params);
        self.session.send(wire.as_bytes()).await?;
        Ok(())
    }

    // -------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------

    /// Look up `code` in the command table; return an error if not found.
    fn validate_code(
        &self,
        code: &str,
    ) -> Result<&'static CommandDefinition<C>, ClientError<S::Error>> {
        self.table
            .find(code)
            .ok_or_else(|| ClientError::UnknownCommand(code.to_string()))
    }

    /// Execute one query-shaped exchange through the session and decode the
    /// response bytes into a `String` matching the wire text the radio sent.
    async fn execute_query(&mut self, wire: &[u8]) -> Result<String, ClientError<S::Error>> {
        let mut response = Vec::new();
        let disposition = self.session.execute(wire, &mut response).await?;
        match disposition {
            ResponseDisposition::ProtocolError(kind) => Err(ClientError::ProtocolError(kind)),
            ResponseDisposition::ResponseWritten | ResponseDisposition::NoResponse => {
                Ok(String::from_utf8_lossy(&response).into_owned())
            }
        }
    }
}

/// Convenience `Result` alias for [`CatClient`] operations.
pub type ClientResult<T, E> = Result<T, ClientError<E>>;

/// Generic, radio-independent client-side errors.
///
/// Replaces `ts570d::RadioError` for this layer: keeps only the variants
/// that are actually generic and drops the radio-specific ones
/// (`InvalidMode`, `FrequencyOutOfRange`, `NotImplemented`, `Unsupported`) —
/// those stay in each radio crate's own error type, which wraps or converts
/// from `ClientError<E>` once a radio crate migrates onto this one.
#[derive(Debug, Error)]
pub enum ClientError<E>
where
    E: std::error::Error + 'static,
{
    /// `code` is not present in the radio's command table.
    #[error("unknown command: {0}")]
    UnknownCommand(String),
    /// Command exists but does not support query (read).
    #[error("command {0} does not support read (query)")]
    CommandNotReadable(String),
    /// Command exists but does not support set (write).
    #[error("command {0} does not support write (set)")]
    CommandNotWritable(String),
    /// The session reported a protocol-level error for this exchange.
    #[error("session reported a protocol error: {0:?}")]
    ProtocolError(ProtocolErrorKind),
    /// I/O error from the underlying [`CatSession`].
    #[error("transport error: {0}")]
    Transport(#[from] E),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use cat_framework::{CommandForm, CommandOperation};
    use cat_transport_core::test_support::{Exchange, ScriptedCatSession};
    use cat_transport_core::TransportError;

    use super::*;

    // -----------------------------------------------------------------
    // In-crate fake CommandId / CommandTable (never a real radio crate).
    // -----------------------------------------------------------------

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FakeCommand {
        Frequency,
        Information,
        Transmit,
        SignalMeter,
        RitMeter,
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
            description: "Test write-only action",
            query_forms: NONE,
            set_forms: NONE,
            action_forms: ACTION,
            response_forms: NONE,
            readable: false,
            writable: true,
        },
        CommandDefinition {
            id: FakeCommand::SignalMeter,
            code: "SM",
            name: "Signal meter",
            description: "Test selector-parameter read",
            query_forms: QUERY,
            set_forms: NONE,
            action_forms: NONE,
            response_forms: NONE,
            readable: true,
            writable: false,
        },
        CommandDefinition {
            id: FakeCommand::RitMeter,
            code: "RM",
            name: "RIT meter",
            description: "Test selector-parameter read",
            query_forms: QUERY,
            set_forms: NONE,
            action_forms: NONE,
            response_forms: NONE,
            readable: true,
            writable: false,
        },
    ];

    static TABLE: CommandTable<FakeCommand> = CommandTable::new(DEFINITIONS);

    /// Build a `CatClient` around a `ScriptedCatSession` pre-loaded with
    /// `script`.
    fn client_with_script<I: IntoIterator<Item = Exchange>>(
        script: I,
    ) -> CatClient<FakeCommand, ScriptedCatSession> {
        CatClient::new(ScriptedCatSession::with_script(script), &TABLE)
    }

    // -----------------------------------------------------------------
    // A minimal fake `CatSession` that always reports a protocol error,
    // for exercising `ClientError::ProtocolError` — `ScriptedCatSession`
    // never produces that disposition itself.
    // -----------------------------------------------------------------

    #[derive(Default)]
    struct ProtocolErrorSession {
        written: Vec<u8>,
    }

    #[async_trait(?Send)]
    impl CatSession for ProtocolErrorSession {
        type Error = TransportError;

        async fn execute(
            &mut self,
            request: &[u8],
            _response: &mut Vec<u8>,
        ) -> Result<ResponseDisposition, TransportError> {
            self.written.extend_from_slice(request);
            Ok(ResponseDisposition::ProtocolError(
                ProtocolErrorKind::UnknownCommand,
            ))
        }
    }

    // -----------------------------------------------------------------
    // Tests (ported 1:1 from ts570d's radio/src/client.rs test module)
    // -----------------------------------------------------------------

    #[monoio::test(driver = "legacy")]
    async fn test_query_fa_formats_correctly() {
        let mut client = client_with_script([Exchange::new("FA;", "FA00014250000;")]);

        let response = client.query("FA").await.unwrap();

        assert_eq!(client.session.written(), b"FA;");
        assert_eq!(response, "FA00014250000;");
    }

    #[monoio::test(driver = "legacy")]
    async fn test_set_fa_formats_correctly() {
        let mut client = client_with_script([Exchange::new("FA00014250000;", "")]);

        client.set("FA", "00014250000").await.unwrap();

        assert_eq!(client.session.written(), b"FA00014250000;");
    }

    #[monoio::test(driver = "legacy")]
    async fn test_query_unknown_command_returns_error() {
        let mut client = client_with_script([]);

        let result = client.query("ZZ").await;

        assert!(
            matches!(result, Err(ClientError::UnknownCommand(ref c)) if c == "ZZ"),
            "expected UnknownCommand(ZZ), got {:?}",
            result
        );
    }

    #[monoio::test(driver = "legacy")]
    async fn test_set_read_only_command_returns_error() {
        // IF is read-only (writable = false)
        let mut client = client_with_script([]);

        let result = client.set("IF", "").await;

        assert!(
            matches!(result, Err(ClientError::CommandNotWritable(ref c)) if c == "IF"),
            "expected CommandNotWritable(IF), got {:?}",
            result
        );
    }

    #[monoio::test(driver = "legacy")]
    async fn test_query_write_only_command_returns_error() {
        // TX is write-only (readable = false)
        let mut client = client_with_script([]);

        let result = client.query("TX").await;

        assert!(
            matches!(result, Err(ClientError::CommandNotReadable(ref c)) if c == "TX"),
            "expected CommandNotReadable(TX), got {:?}",
            result
        );
    }

    #[monoio::test(driver = "legacy")]
    async fn test_set_does_not_read_response() {
        // set() must call session.send(), never session.execute() with a
        // read-shaped expectation. Here we only confirm set() succeeds
        // against a single fire-and-forget exchange.
        let mut client = client_with_script([Exchange::new("FA00014250000;", "")]);

        client.set("FA", "00014250000").await.unwrap();
    }

    #[monoio::test(driver = "legacy")]
    async fn test_query_set_unknown_command_does_not_write() {
        let mut client = client_with_script([]);

        let _ = client.query("ZZ").await;

        assert!(
            client.session.written().is_empty(),
            "nothing should be written for unknown command"
        );
    }

    #[monoio::test(driver = "legacy")]
    async fn test_query_with_param_sm0_formats_correctly() {
        let mut client = client_with_script([Exchange::new("SM0;", "SM0015;")]);

        let response = client.query_with_param("SM", "0").await.unwrap();

        assert_eq!(client.session.written(), b"SM0;");
        assert_eq!(response, "SM0015;");
    }

    #[monoio::test(driver = "legacy")]
    async fn test_query_with_param_rm1_formats_correctly() {
        let mut client = client_with_script([Exchange::new("RM1;", "RM10023;")]);

        let response = client.query_with_param("RM", "1").await.unwrap();

        assert_eq!(client.session.written(), b"RM1;");
        assert_eq!(response, "RM10023;");
    }

    #[monoio::test(driver = "legacy")]
    async fn test_query_with_param_unknown_command_returns_error() {
        let mut client = client_with_script([]);

        let result = client.query_with_param("ZZ", "0").await;

        assert!(
            matches!(result, Err(ClientError::UnknownCommand(ref c)) if c == "ZZ"),
            "expected UnknownCommand(ZZ), got {:?}",
            result
        );
    }

    // -----------------------------------------------------------------
    // Additional tests for this crate's genericized error handling, not
    // present in ts570d's original suite (RadioError's InvalidProtocolString
    // and Transport(#[from] TransportError) variants had no direct test
    // there either — added here since ClientError is new work).
    // -----------------------------------------------------------------

    #[monoio::test(driver = "legacy")]
    async fn test_query_protocol_error_is_surfaced() {
        let mut client = CatClient::new(ProtocolErrorSession::default(), &TABLE);

        let result = client.query("FA").await;

        assert!(
            matches!(
                result,
                Err(ClientError::ProtocolError(
                    ProtocolErrorKind::UnknownCommand
                ))
            ),
            "expected ProtocolError(UnknownCommand), got {:?}",
            result
        );
    }

    #[monoio::test(driver = "legacy")]
    async fn test_query_transport_error_propagates() {
        let mut session = ScriptedCatSession::new();
        session.simulate_disconnect();
        let mut client = CatClient::new(session, &TABLE);

        let result = client.query("FA").await;

        assert!(
            matches!(
                result,
                Err(ClientError::Transport(TransportError::Other(_)))
            ),
            "expected Transport(Other(_)), got {:?}",
            result
        );
    }

    #[monoio::test(driver = "legacy")]
    async fn test_set_transport_error_propagates() {
        let mut session = ScriptedCatSession::new();
        session.simulate_timeout();
        let mut client = CatClient::new(session, &TABLE);

        let result = client.set("FA", "00014250000").await;

        assert!(
            matches!(
                result,
                Err(ClientError::Transport(TransportError::ReadTimeout))
            ),
            "expected Transport(ReadTimeout), got {:?}",
            result
        );
    }
}
