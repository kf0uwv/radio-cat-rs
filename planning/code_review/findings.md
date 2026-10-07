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
