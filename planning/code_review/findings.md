# Findings — code_review

(none yet — no extraction work has started)

## 2026-10-06 — feat/cat-morse review

Verification: crate tests 85+53+… all pass; clippy -D warnings clean (all
features). Probes in a scratch crate outside the repo (path dep on cat-morse).

1. should-fix CONFIRMED — decoder loses the first ~word when tone is present
   during warm-up (decoder.rs step(): floor trained as plain mean for 25
   windows while !on, incl. tone). "PARIS PARIS" starting at t=0 → "RIS PARIS"
   (15/20 wpm), "PARIS" (30 wpm); joining a 25 wpm stream exactly at the first
   mark → first word lost. All roundtrip tests start at 300 ms.
2. should-fix CONFIRMED — Classifier histories poisoned by outliers
   (two_means over HISTORY=24 marks/gaps, no outlier rejection for marks).
   3 s (or 1 s) carrier then PARIS×4 → "T*S HISIS PARIS PARIS". A 20 ms
   full-scale crash on a −20 dB signal → "PARISE TRISPARISPARISPARISPARIS"
   (word spaces gone for the rest of the stream).
3. should-fix CONFIRMED — PitchMode::Auto with a narrow range never locks
   (median of a 3–4 bin bank is the tone's neighbour); validate() accepts
   ≥20 Hz spans. [690,720]/705, [690,720]/700, [690,710]/700 → pitch None, "".
4. should-fix — PlaybackQueue::render takes a blocking Mutex::lock on the
   audio thread; docs/ADR 0017 §16 say it "never blocks". Writer holds the
   lock across an up-to-capacity clamp+extend; a preempted writer stalls the
   RT callback. Use try_lock → silence on contention, clamp before locking.
5. should-fix CONFIRMED — Signal::new default start=0 puts the first ramp's
   half-amplitude point at t=0: first sample is 0.124 (= ½·0.25), a key click
   contradicting synth's "click-free". band_channel QRM start can be 0 too.
6. plan deviation (recorded in cat_morse/progress.md, needs architect ack) —
   auto-pitch never re-acquires (plan: "re-acquire after the squelch has been
   closed for a while"). Also API drift: TimingError::InvalidWpm vs plan's
   NonPositiveWpm, Difficulty.wpm (u32,u32) vs (f64,f64).
7. nit CONFIRMED — pitch near 0/Nyquist accepted but undecodable (image
   within the Hann bin): Fixed(3990) at 8 kHz → "3JN EH A*E AT TB".
8. nit CONFIRMED — Speed accepts any wpm in (0,100]; Farnsworth(100,1e-9)
   gives a letter gap that saturates secs() to u64::MAX ns; rendering such
   keying aborts on allocation. Suggest a floor (≥1 or ≥3 wpm, as decoder).
9. nit — a huge finite input (1e30) deafens the decoder for ~100 s (peak
   release 6 dB/s); real ±1 audio is fine; NaN/inf handled fine (CONFIRMED).
10. nit — Callsign derives Deserialize on the tuple, bypassing parse();
    band_channel does not check pitch+QRM offset < Nyquist (render errors
    later); align_unordered is O((n+m)^3) on unbounded operator input;
    render with frames==0 clears `playing`.
11. test gap — no golden-value test pinning a keying/script across
    platforms; the ADR 0021 bit-identical promise is only checked
    same-process. Windows CI would catch drift if one existed.

Checked and fine: Hungarian matches brute force on 400 random non-square
cases (0–4 expected × 0–5 copied, incl. empty) and is deterministic; 13.5k
scenarios across odd my_calls (VE3/W1XYZ, 9A2AB, N0A, 1A1, W1AW/P) all
generate; timing/scenario use no transcendentals or hash iteration; Rng
pinned; host_lock never nested (no re-entrant deadlock), poison-tolerant,
not held by callbacks; DeviceKind is #[non_exhaustive]; zero-length feed ok.

## 2026-10-07 — Review: feat/morse-world-calls (feat/cat-morse..HEAD, 18 commits)

Verified in a scratch copy (probe tests outside the repo). Tests 157/157 green, clippy clean.
No panics/hangs: 13,500 `generate` runs (DxChase/DxPileup/RagChew x 15 edge operator calls incl.
4U1UN, BS7H, 3Y0J, MM/DL1ABC, W1ABC/MM, 3D2AB/R, hf_band 160m). station_call round-trips 0/136,000
misses across all 340 entities. No HashMap/HashSet outside tests. Downstream (ts570d src/cw,
cw-trainer) neither matches ScenarioKind exhaustively nor builds Difficulty by literal: no breakage.

1. callsign.rs:260-266 — MODIFIERS/NO_ENTITY applied in prefix position. `M/DL1ABC` -> GERMANY
   (England), `B/DL1ABC` -> GERMANY (China), `MM/DL1ABC` -> None (Scotland, the standard CEPT
   form). Also makes designator() (356) silently waste England's `M`/Scotland's `MM` draws.
   Fix: only treat pieces after the base as modifiers / MM / AM.
2. callsign.rs:100 base_part tie -> last piece. `W1A/VE3` -> USA, `N0A/KH6` -> USA,
   `K1AB/VP2E` -> USA, `F5AB/VP2E` -> FRANCE; `N0A/QRP` fails Callsign::parse (base "QRP").
   Pre-existing rule, now load-bearing for designator precedence. Fix: base = longest piece
   containing a digit and not a known modifier, tie -> the piece that isn't a pure prefix.
   Generator is safe (licence.len() > d.len() guard, no 3-char calls generated).
3. Region by first digit (callsign.rs:274) for 7K-7N/7J/8J: `7K3A` -> JA area 7, `8J1A` -> area 8.
   morse_world findings claim 7K3ABC reads as area 3 — it doesn't. Zones same (25/45), QTH wrong.
4. Data gaps: CT9 -> PORTUGAL (Madeira); FT4Y/FT5Y -> FRANCE (Antarctica); ZS7 -> SOUTH AFRICA
   (Antarctica); R1AN -> EU RUSSIA (Antarctica); 4U1VIC -> None (Austria); China 3H-3U/XS
   missing (3H1A -> None); `DL1ABC/KG4` -> USA (EXACT_SUFFIX applied to a designator piece).
5. qso.rs place() fallback: random_dx excludes the operator's entity but not the chased DX's, so
   a DxChase caller whose one-call home is exhausted can be the DX's countryman. Very rare.
Top-30 entity spot-check (prefix blocks, continent, CQ/ITU) otherwise correct.

## 2026-10-07 — feat/cw-capability (ADR 0023) TX-safety review

`cargo test --workspace`: 1262 passed, 0 failed.

1. HIGH — BufferFlag `ready()` is "buffer space", not "idle" (TS-570D manual
   KY: 0 = buffer space available). Supervisor treats Ready(true) as idle:
   chunks stack in the radio (breaks 3 s drain bound), job "completes" and
   Stopping's WaitReady passes while keying -> restore writes full power
   under a keyed chunk. Scratch test: all 14 chunks of a 10 wpm job handed
   over at t=0; after abort+disarm power=100 at t=0 with ~25 s still
   buffered. cw.rs:1013-1025, 1098-1099; native_bridge.rs:130 doc.
2. HIGH — raw TCP/UDP CAT listeners are not gated while armed (TX;, PC100;,
   KS, KY). lib.rs:393-418 (Linux), Windows equivalent. ADR only gates rigctl.
3. MED — keyer speed can change mid-job (SetKeyerSpeed TOCTOU
   native_bridge.rs:796 vs queue applied after drive_cw at 1208-1234; also
   raw KS / front knob); chunks sized at old wpm, drain bound exceeded.
4. MED — owed ID (abort before ID) is dropped on disarm/detach/idle.
5. LOW — cw_blocks_external_ptt fails open on poisoned mutex (624-626);
   AbortCw acked-and-dropped for consoles that handshook before enable_cw.
6. LOW — PARIS 3:1 estimate vs TS-570D auto/locked weighting up to 4:1
   (+~26% on dash-heavy text); ID split mid-callsign at slow wpm.

## 2026-10-07 — re-review of fixes e4cec18..bc32cd0

`cargo test --workspace`: 1283 passed, 0 failed.

- F1 fixed: 10 wpm buffer-space case now hands the radio "C" only at t=0;
  abort at 0.5 s, power restored at 1.85 s (= 4:1 est of C + 250 ms margin).
  WaitKeyed is time-only (no livelock); in-flight Send counted on completion.
- F2 fixed: BrokerHandle::gated on submit(), both platforms, all four raw
  listeners; raw uses only submit(); gate closed from Arming (no window).
- F3 fixed for SetKeyerSpeed (refused while armed, re-checked at pump apply).
  NEW low: replan (cw.rs on_speed->replan) compares only *remaining* time to
  the cap; front-panel slowdown mid-job lets the job exceed 60 s (scratch:
  54.8 s job, knob 20->10 wpm at 31 s, completed at 63 s; worst ~1.5x cap).
- F4 fixed as decided (IdOwed notice + CwState.id_owed, error log).
- F5 fixed (fail-closed cw_query; serves_cw per message). NEW low: drive_cw
  still uses with_cw (lock().ok()), so after poisoning an abort is recorded
  but the pump never issues Stop (matters for RadioStop radios).
- F6 fixed (4:1 dash; '0' at 10 wpm 2.88 s < 3 s bound).
