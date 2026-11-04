# Findings

- `decode_spectrum_payload` (cat-native/src/lib.rs:1322),
  `decode_audio_payload` (:1376) and `decode_frame` (:163) widen `u32`
  lengths to `usize` and do unchecked `+`/`*`.
- `decode_frame` caps only Control frames (`MAX_CONTROL_BYTES`); Spectrum
  and Audio lengths are unbounded.
- The existing tests (`a_truncated_spectrum_payload_decodes_to_nothing`,
  `an_absurd_control_length_is_refused_before_allocating`) cover short
  payloads and the Control cap, not overflow.
- Found during a consumer's 32-bit Android build (armv7-linux-androideabi).
