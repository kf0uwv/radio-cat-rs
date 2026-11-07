# Task: payload length arithmetic overflows on 32-bit targets

## Goal
`cat-native`'s binary decoders never overflow `usize` on 32-bit targets
(armv7, i686), whatever length a peer puts on the wire. A bad length is
refused (`None` or `FrameError::TooLarge`); it never panics or yields a
frame built from the wrong bytes.

## The defect
Lengths arrive as `u32` and are widened to `usize`. On a 32-bit target
`usize` is 32 bits, so:
- `decode_spectrum_payload`: `28 + count * 4` overflows for
  `count >= 2^30 - 7`.
- `decode_audio_payload`: `28 + samples * 4 + bins * 4` overflows the same
  way, and `28 + samples * 4` again as the bins' start.
- `decode_frame`: `5 + len` overflows for `len > usize::MAX - 5`
  (Spectrum and Audio frames have no length cap; only Control does).

In a debug build that panics on arithmetic overflow; in release it wraps,
the length check passes against a small number, and the slice then panics
(`start > end`) or decodes the wrong span. One case is quietly wrong rather
than a crash: a 28-byte spectrum header with `count = 1 << 30` wraps to 28,
passes, and decodes as `Some` frame with no bins. Either way a peer can crash a
32-bit client or server with one header. 64-bit targets are not affected
(`u32::MAX * 4 + 28` fits in 64 bits).

## Phases
1. [x] Worktree `fix/payload-lengths` off origin/main (b821bf4); read
       CLAUDE.md. `cat-native` is identical on `feat/cat-ft8`, so no
       conflict with that branch.
2. [x] Plan reviewed. The user asked for this fix ("Fix the overflow for
       sure"); the architect asked for changes 1-3 below, adopted.
3. [x] Tests first:
       - `floats_end(start, count)` returns `None` on either overflow, on
         any host: the multiply (`count = usize::MAX / 4 + 1`) and the add
         (`count = usize::MAX / 4`, `start = 28`).
       - A header-only spectrum payload with `count = 1 << 30`, and an audio
         one with `samples = bins = 1 << 29`, decode to `None`.
       - `decode_frame` on a Spectrum header with `len = u32::MAX` is
         `Incomplete` (same answer on every target).
       On 64-bit the decoder tests only guard against regression (they are
       `None` because the payload is short); the 32-bit proof is the helper
       test.
4. [x] Fix: checked arithmetic in the payload decoders, each range computed
       once (audio: scope end, then bins end, passed to the reader).
       `decode_frame` compares `buf.len() - 5 < len` instead of adding. No change to the
       wire format, to the encoders, or to what any valid payload decodes
       to. No new cap on bin or sample counts (that would be a protocol
       decision).
5. [x] `cargo test -p cat-native`, `cargo clippy --all-targets
       -D warnings`, `cargo fmt --check`; `cargo check -p cat-native
       --target armv7-linux-androideabi` and `i686-linux-android` to
       show it compiles for 32-bit. Commit on the branch.

## Decisions
- `decode_frame` compares without adding (architect option a), so a huge
  length is `Incomplete` on every target, as it already is on 64-bit. The
  same bytes never decode differently per target.
- No upper bound on counts or frame lengths is added here. Two follow-up
  issues, separate from this fix:
  1. Spectrum and Audio frames have no length cap, so a peer can make
     either side buffer up to 4 GiB (32- and 64-bit alike). Likely a cap
     like `MAX_CONTROL_BYTES`.
  2. The server loop (`server.rs`, `let Ok(..) = decode_frame(..) else
     { break }`) treats `TooLarge` and `UnknownKind` like `Incomplete` and
     keeps growing `pending`; it should drop the connection on any error
     other than `Incomplete`, as the client does.

## Verification limits
No 32-bit host to run the tests on (no multilib libgcc, no qemu), so the
32-bit behaviour is shown through the helper's boundary test plus a
32-bit check, not by running the decoders on armv7. Optional: Miri
(`cargo +nightly miri test -p cat-native --target i686-unknown-linux-gnu
--lib <decoder filter>`) would run them with 32-bit `usize`; it needs the
miri component installed, which the owner has not yet approved.
