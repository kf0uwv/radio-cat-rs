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

//! Linux (`monoio`) rigctld TCP accept loop, generic over
//! [`crate::RigctlRadio`]. The wire protocol itself — command dispatch,
//! `\dump_state`, and line-buffering — lives in [`crate::protocol`], shared
//! with [`crate::rigctl_windows`]'s `std`-based accept loop; this module
//! owns only the `monoio`-specific I/O around it.
//!
//! Ported from `ft991a`'s current `server/src/rigctl.rs`, which was
//! validated against a real Hamlib client (`rigctl -m 2`, model 2 = "NET
//! rigctl" — the same `netrigctl.c` backend WSJT-X's "Hamlib NET rigctl"
//! rig type uses) after a real WSJT-X instance reported a connection
//! timeout against an earlier version of this module. See
//! [`crate::protocol`]'s module doc for the two protocol bugs found and
//! fixed along the way; both fixes live there now, not here.
//!
//! Delegates to a radio-specific [`crate::RigctlRadio`] implementation —
//! this module never constructs a raw radio wire frame itself; that is
//! `cat_server::BrokerCatSession` plus a radio crate's own typed client's
//! job, entirely below this abstraction.

use std::io;

use monoio::io::{AsyncReadRent, AsyncWriteRentExt};
use monoio::net::{TcpListener, TcpStream};

use cat_server::{BrokerCatSession, BrokerHandle, ClientId};

use crate::protocol::{self, LineSplitter};
use crate::RigctlRadio;

/// Accept loop, mirroring `cat_server::tcp::serve`'s shape: binding is the
/// caller's responsibility, one task per accepted connection, runs until
/// `accept()` itself fails. `make_radio` builds one `R: RigctlRadio` per
/// accepted connection from a fresh [`BrokerCatSession`] tagged with that
/// connection's [`ClientId`] — this is the one seam where a caller's
/// concrete radio type plugs into otherwise fully generic dispatch logic.
pub(crate) async fn serve<R, F>(
    listener: TcpListener,
    handle: BrokerHandle,
    make_radio: F,
) -> io::Result<()>
where
    R: RigctlRadio + 'static,
    F: Fn(BrokerCatSession) -> R + Clone + 'static,
{
    let mut next_client_id: u64 = 0;
    loop {
        let (stream, _peer_addr) = listener.accept().await?;
        let client_id = ClientId::from_raw(next_client_id);
        next_client_id = next_client_id.wrapping_add(1);
        let handle = handle.clone();
        let make_radio = make_radio.clone();
        monoio::spawn(handle_connection(stream, handle, client_id, make_radio));
    }
}

async fn handle_connection<R, F>(
    mut stream: TcpStream,
    handle: BrokerHandle,
    client_id: ClientId,
    make_radio: F,
) where
    R: RigctlRadio,
    F: Fn(BrokerCatSession) -> R,
{
    let mut radio = make_radio(BrokerCatSession::new(handle, client_id));
    let mut splitter = LineSplitter::new();

    loop {
        let line = match read_line(&mut stream, &mut splitter).await {
            Ok(Some(line)) => line,
            Ok(None) | Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.eq_ignore_ascii_case("q") {
            break;
        }

        let response = protocol::dispatch(&mut radio, trimmed).await;
        let (result, _buf) = stream.write_all(response.into_bytes()).await;
        if result.is_err() {
            break;
        }
    }
}

/// Read one complete line via `monoio`'s owned-buffer
/// `AsyncReadRentExt::read`, feeding bytes through the shared
/// [`LineSplitter`]. `Ok(None)` on a clean disconnect with no partial line
/// pending; `Err` on any I/O failure, including a line that exceeds
/// [`protocol::MAX_LINE_LEN`] bytes without a `\n`.
async fn read_line(
    stream: &mut TcpStream,
    splitter: &mut LineSplitter,
) -> io::Result<Option<String>> {
    loop {
        if let Some(line) = splitter.try_take_line() {
            return Ok(Some(line));
        }

        if splitter.is_over_limit() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "rigctl line exceeded maximum length of {} bytes without a newline",
                    protocol::MAX_LINE_LEN
                ),
            ));
        }

        let chunk = vec![0u8; 4096];
        let (result, chunk) = stream.read(chunk).await;
        let n = result?;
        if n == 0 {
            return Ok(splitter.take_final_partial_line());
        }
        splitter.feed(&chunk[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard, ported from this module's pre-`protocol`-extraction
    /// version: verifies the `monoio`-specific `read_line`/`LineSplitter`
    /// wiring end-to-end over a real loopback socket, rather than
    /// `protocol::LineSplitter`'s own pure-buffering unit tests (which
    /// don't touch I/O at all).
    #[monoio::test(driver = "legacy")]
    async fn read_line_rejects_a_line_longer_than_the_maximum_without_a_newline() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind failed");
        let addr = listener.local_addr().expect("local_addr failed");

        let server_task = monoio::spawn(async move {
            let (mut stream, _peer_addr) = listener.accept().await.expect("accept failed");
            let mut splitter = LineSplitter::new();
            read_line(&mut stream, &mut splitter).await
        });

        let mut client = TcpStream::connect(addr).await.expect("connect failed");
        // One byte over the cap, with no '\n' anywhere in it.
        let oversized = vec![b'x'; protocol::MAX_LINE_LEN + 1];
        let (result, _buf) = client.write_all(oversized).await;
        result.expect("write_all failed");

        let outcome = server_task.await;
        let err = outcome.expect_err("a line over MAX_LINE_LEN with no newline must be an Err");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        drop(client);
    }
}
