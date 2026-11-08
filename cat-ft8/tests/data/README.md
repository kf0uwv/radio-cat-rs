# cat-ft8 test data

Black-box reference vectors generated from WSJT-X 3.0.0-rc1 `ft8sim`
output. One JSON object per line:

| Key | Meaning |
|-----|---------|
| `message` | Text given to `ft8sim` |
| `decoded` | `ft8sim`'s own unpack of that message (`null` if it printed none) |
| `i3`, `n3` | Message type it chose |
| `bits77` | The 77 payload bits it printed |
| `symbols` | The 79 channel symbols (tones 0–7) it printed |

- `encoder-vectors.jsonl` — 1 020 messages: 20 fixed cases covering each
  message type, then 1 000 random standard messages (CQ with grid, grid,
  report, R+report, RR73, RRR, 73).
- `encoder-vectors-extra.jsonl` — 89 edge cases: type 4 nonstandard calls,
  CQ modifiers, report limits, telemetry/free-text boundaries and bracketed
  callsign combinations.
- `encoder-vectors-random.jsonl` — 420 random messages, 60 per class: type
  4, free text, telemetry, DXpedition, `/P`, type 5, and type 1 with `/R`,
  hashed calls, CQ modifiers and reports over −50…+50.

Known oracle quirks are handled explicitly in `tests/vectors.rs`: one
message `ft8sim` packs but cannot unpack (`*** bad message ***`, with
all-zero symbols), inputs it prints nothing for (counted), and the inputs
where it silently truncates or drops part of the message (listed there as
deviations).
