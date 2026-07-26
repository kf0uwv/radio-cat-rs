# 5. `cat-rigctl`: a generic rigctld bridge behind a `RigctlRadio` trait

Date: 2026-07-26

## Status

Accepted

## Context

`ft991a` and `ts570d` each independently hand-wrote a Hamlib
rigctld-compatible TCP bridge (for WSJT-X's "Hamlib NET rigctl" rig type) in
their own `server` crate: a `broker_session.rs` (`BrokerCatSession`, a
`CatSession` adapter over `cat_server::BrokerHandle`), a `rigctl.rs`
(dispatch/`\dump_state`/line framing), and a `lib.rs::run()` (listener
orchestration). `broker_session.rs` was 100% duplicated with zero
radio-specific logic. `rigctl.rs` and `run()` were ~90% duplicated, and each
had independently accumulated one bugfix the other lacked: `ft991a`'s
`rigctl.rs` fixed two real interop bugs found against a live Hamlib client
(`\dump_state`'s capability-tail field count; `F`'s `%f`-formatted float
parsing); `ts570d`'s `run()` fixed a real error-propagation bug
(`select_all` results silently discarded, `run()` always returning `Ok(())`
regardless of a listener's actual failure).

This is exactly the kind of drift this repo's whole `radio-cat-rs`
extraction effort exists to prevent — divergent bugfixes are strictly worse
than either app lacking a fix outright, because they are silent: nothing
signals that the sibling app is missing a fix the other already has.

## Decision

Move both pieces into this repo:

1. `BrokerCatSession` moves into `cat-server` itself
   (`cat-server/src/broker_session.rs`, `pub use`d from `cat-server`'s
   root) — it never depended on anything FT-991A/TS-570D-specific to begin
   with, so this is pure relocation, not new design.

2. A new crate, `cat-rigctl`, holds the dispatch/`\dump_state`/line-framing
   logic and the `run()` orchestration, generic over a new trait:

   ```rust
   #[async_trait::async_trait(?Send)]
   pub trait RigctlRadio {
       type Mode: Copy;
       type Error;

       async fn get_vfo_a_hz(&mut self) -> Result<u64, Self::Error>;
       async fn set_vfo_a_hz(&mut self, hz: u64) -> Result<(), Self::Error>;
       async fn get_mode(&mut self) -> Result<Self::Mode, Self::Error>;
       async fn set_mode(&mut self, mode: Self::Mode) -> Result<(), Self::Error>;
       async fn get_transmitting(&mut self) -> Result<bool, Self::Error>;
       async fn transmit(&mut self) -> Result<(), Self::Error>;
       async fn receive(&mut self) -> Result<(), Self::Error>;

       fn hamlib_mode_name(mode: Self::Mode) -> &'static str;
       fn hamlib_mode_from_name(name: &str) -> Option<Self::Mode>;
       fn freq_range_hz() -> (u64, u64);
   }
   ```

   Each app implements this once for its own typed radio client
   (`ft991a::radio::Ft991a<S>`, `ts570d::radio::Ts570d<S>`). `run<C, S, R,
   F>(session, table, config, make_radio)` takes a `make_radio: F` closure
   (`BrokerCatSession -> R`) as the one seam where a caller's concrete
   radio type plugs into otherwise fully generic listener orchestration —
   the same shape `cat-server::build` already uses for
   `CommandTable<C>`/`CatSession` genericity, extended one level further to
   cover the radio's *typed* client, not just its wire-level session.

## Why this crosses the ADR bar

Every crate boundary recorded in ADR 0001 is a strict layering: each crate
either sits below the ones that depend on it (generic engine → transport →
client → server) or is depended on, never both. `cat-rigctl` introduces a
different shape: a generic crate that *calls back into* per-app code via a
trait (`RigctlRadio`), rather than being called by it or hiding it behind a
type parameter alone. This is a new kind of dependency edge in this
repo's design vocabulary — worth naming explicitly, the way ADR 0003 named
`ModemControlLines` as "a separate, additive capability trait" for a
similarly novel-shaped decision.

## Consequences

- `ft991a`'s and `ts570d`'s own `server` crates shrink to: implement
  `RigctlRadio` once, define their own mode-name tables (the one piece that
  really is per-radio — an FT-991A implementation has C4FM/data-mode
  fallbacks with no exact Hamlib counterpart, a TS-570D implementation is a
  clean 1:1 table over a smaller mode set), and call `cat_rigctl::run`.
  Neither app's `server` crate needs its own `broker_session.rs`,
  `rigctl.rs`, or `run()` orchestration anymore. (Not done in this
  session — both apps are read-only ground truth here, per this task's
  charter; that migration is the next phase, in each app's own working
  copy.)
- `ft991a`'s already-verified interop fixes (dump_state field count, `F`
  float parsing) and `ts570d`'s already-verified error-propagation fix are
  now both present exactly once, for both apps, by construction — the
  drift this ADR exists to close cannot recur for this subsystem.
- `RigctlRadio::Error` is deliberately unbounded (no `Display`/
  `std::error::Error` requirement): rigctld's `RPRT -1` convention carries
  no error text on the wire, and dispatch never displays the error value —
  confirmed by rereading every arm of the ported `dispatch` logic before
  writing the trait.
- `run`'s `R: RigctlRadio` bound also requires `R: 'static` (needed for
  `monoio::spawn`'s owned-future requirement) and `F: Clone + 'static` (one
  `make_radio` call per accepted rigctl connection) — a mechanical addition
  beyond the trait's own definition, not a design choice.
