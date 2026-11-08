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

//! Encoder CPU and memory profile over the test-vector messages.
//!
//! ```text
//! cargo run --release -p cat-ft8 --features profile --example encode_bench -- [REPEATS] [SLOTS]
//! encode_bench --stage NAME [REPEATS]
//! ```
//!
//! `--stage` runs only one stage's batch (`pack`, `pack_nohashes`,
//! `remember_hit`, `remember_miss`, `crc`, `ldpc`, `symbols` or `waveform`)
//! and prints nothing else, so its instruction count can be measured on its
//! own, for example with `perf stat -e instructions:u` (stable to about
//! 1e-6, where cycles vary by about ±1%); subtract a run with REPEATS = 0
//! for the setup cost.
//!
//! Times each stage as one batch over every message in
//! `tests/data/encoder-vectors.jsonl`, REPEATS times (default 200), and
//! prints the mean per message (one `Instant::now()` pair can cost more
//! than a stage, so stages are never timed one call at a time). Then
//! synthesizes SLOTS waveforms (default 50) into a 15 s buffer and in
//! 480-sample blocks, and prints the mean per slot. A counting global
//! allocator (in this example only, never in the library) checks that the
//! core allocates nothing.
//!
//! The last line is one `{"profile": {...}}` JSON object: keys ending in
//! `_bytes` are memory, every other key is seconds.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use cat_ft8::ldpc::Codeword;
use cat_ft8::message::{
    pack, CallHashes, CallTable, MessageText, NoHashes, Payload, PACK_SCRATCH_BYTES,
};
use cat_ft8::waveform::{synthesize, Placement, Synth};
use cat_ft8::{crc, encode, ldpc, symbols, SLOT_SAMPLES, STATIC_TABLE_BYTES, SYMBOL_COUNT};

/// Counts heap allocations and bytes, delegating to the system allocator.
struct Counting;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

// SAFETY: every call is forwarded unchanged to `System`, which upholds the
// `GlobalAlloc` contract; the counters are plain atomics.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn heap() -> (u64, u64) {
    (
        ALLOCS.load(Ordering::Relaxed),
        BYTES.load(Ordering::Relaxed),
    )
}

/// Mean time per message of each encode stage.
struct StageTimes {
    /// `pack` with a 64-entry call table (what a receiver keeps).
    pack: Duration,
    /// `pack` with `NoHashes`: the parse and bit packing alone.
    pack_nohashes: Duration,
    /// `CallTable<64>::remember` of a call already present.
    remember_hit: Duration,
    /// `CallTable<64>::remember` of a new call into a full table.
    remember_miss: Duration,
    crc: Duration,
    ldpc: Duration,
    symbols: Duration,
}

/// Each stage runs all its repeats under one timer.
fn mean_stage_times(messages: &[&str], repeats: u32, only: Option<&str>) -> (StageTimes, usize) {
    let mut table: CallTable<64> = CallTable::new();
    let used: Vec<&str> = messages
        .iter()
        .copied()
        .filter(|m| pack(m, &mut table).is_ok())
        .collect();
    let payloads: Vec<Payload> = used
        .iter()
        .filter_map(|m| pack(m, &mut table).ok())
        .collect();
    let a91s: Vec<u128> = payloads.iter().map(crc::append_crc).collect();
    let cws: Vec<Codeword> = a91s.iter().map(|&a| ldpc::encode(a)).collect();

    // With `only`, every other stage is skipped (its time reads zero).
    let time = |name: &str, f: &mut dyn FnMut()| {
        if only.is_some_and(|o| o != name) {
            return Duration::ZERO;
        }
        let s = Instant::now();
        for _ in 0..repeats {
            f();
        }
        s.elapsed()
    };
    let pack_t = time("pack", &mut || {
        for m in &used {
            let _ = black_box(pack(black_box(m), &mut table));
        }
    });
    let pack_nohashes_t = time("pack_nohashes", &mut || {
        for m in &used {
            let _ = black_box(pack(black_box(m), &mut NoHashes));
        }
    });
    // A full table, then the same 64 calls again (hits), then a pool of
    // 4 × 64 calls cycled through (every one a miss that evicts).
    let pool: Vec<String> = (0..256)
        .map(|i| format!("K{}A{}", i % 10, (b'A' + (i / 10) as u8) as char))
        .collect();
    let mut full: CallTable<64> = CallTable::new();
    for c in &pool[..64] {
        full.remember(c);
    }
    let hit_t = time("remember_hit", &mut || {
        for c in &pool[..64] {
            full.remember(black_box(c));
        }
    });
    let miss_t = time("remember_miss", &mut || {
        for c in &pool {
            full.remember(black_box(c));
        }
    });
    let crc_t = time("crc", &mut || {
        for p in &payloads {
            black_box(crc::append_crc(black_box(p)));
        }
    });
    let ldpc_t = time("ldpc", &mut || {
        for &a in &a91s {
            black_box(ldpc::encode(black_box(a)));
        }
    });
    let sym_t = time("symbols", &mut || {
        for cw in &cws {
            black_box(symbols::map(black_box(cw)));
        }
    });
    let n = (used.len() as u32).saturating_mul(repeats).max(1);
    (
        StageTimes {
            pack: pack_t / n,
            pack_nohashes: pack_nohashes_t / n,
            remember_hit: hit_t / (64 * repeats.max(1)),
            remember_miss: miss_t / (256 * repeats.max(1)),
            crc: crc_t / n,
            ldpc: ldpc_t / n,
            symbols: sym_t / n,
        },
        used.len(),
    )
}

/// `--stage NAME`: runs one stage's batch and nothing else.
fn run_one_stage(stage: &str, messages: &[&str], repeats: u32) {
    if stage == "waveform" {
        let symbols: Vec<[u8; SYMBOL_COUNT]> = messages
            .iter()
            .filter_map(|m| encode(m, &mut NoHashes).ok())
            .map(|e| e.symbols)
            .take(50)
            .collect();
        let mut block = [0.0f32; 480];
        for _ in 0..repeats {
            for sym in &symbols {
                let mut synth = Synth::new(sym, 1500.0, 1.0).expect("synth");
                while synth.fill(black_box(&mut block)) == block.len() {}
            }
        }
        return;
    }
    const STAGES: [&str; 7] = [
        "pack",
        "pack_nohashes",
        "remember_hit",
        "remember_miss",
        "crc",
        "ldpc",
        "symbols",
    ];
    assert!(STAGES.contains(&stage), "unknown stage {stage}");
    let _ = black_box(mean_stage_times(messages, repeats, Some(stage)));
}

fn main() {
    let mut args = std::env::args().skip(1).peekable();
    let only: Option<String> = if args.peek().map(String::as_str) == Some("--stage") {
        args.next();
        Some(args.next().expect("--stage needs a stage name"))
    } else {
        None
    };
    let repeats: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(200);
    let slots: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(50);

    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/encoder-vectors.jsonl"
    );
    let text = std::fs::read_to_string(path).expect("read vectors");
    let owned: Vec<String> = text
        .lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).expect("json");
            v["message"].as_str().expect("message").to_string()
        })
        .collect();
    let messages: Vec<&str> = owned.iter().map(String::as_str).collect();

    if let Some(stage) = only.as_deref() {
        run_one_stage(stage, &messages, repeats);
        return;
    }
    let (t, used) = mean_stage_times(&messages, repeats, None);
    let ns = |d: Duration| d.as_nanos() as f64;
    println!(
        "{} messages ({used} packable) x {repeats} repeats, each stage timed as a batch",
        messages.len()
    );
    println!("stage      mean ns/message");
    println!("pack       {:10.1}  (CallTable<64>)", ns(t.pack));
    println!(
        "  pack     {:10.1}  (NoHashes: parse and packing only)",
        ns(t.pack_nohashes)
    );
    println!(
        "  remember {:10.1}  per call, already in the table",
        ns(t.remember_hit)
    );
    println!(
        "  remember {:10.1}  per call, new into a full table (evicts)",
        ns(t.remember_miss)
    );
    println!("crc        {:10.1}", ns(t.crc));
    println!("ldpc       {:10.1}", ns(t.ldpc));
    println!("symbols    {:10.1}", ns(t.symbols));
    println!(
        "total      {:10.1}",
        ns(t.pack + t.crc + t.ldpc + t.symbols)
    );

    // Everything the waveform pass needs is allocated before counting.
    let all_symbols: Vec<[u8; SYMBOL_COUNT]> = messages
        .iter()
        .filter_map(|m| encode(m, &mut NoHashes).ok())
        .map(|e| e.symbols)
        .take(slots.max(1))
        .collect();
    let mut slot = vec![0.0f32; SLOT_SAMPLES];
    let placement = Placement::slot(1500.0, 0.0).expect("placement");
    let count = all_symbols.len() as u32;

    // Heap check: a full pass of the core with the counter running.
    let (a0, b0) = heap();
    let mut table: CallTable<64> = CallTable::new();
    for m in &messages {
        let _ = black_box(encode(m, &mut table));
    }
    let s = Instant::now();
    for sym in &all_symbols {
        slot.fill(0.0);
        synthesize(sym, &placement, &mut slot).expect("synthesize");
    }
    let waveform = s.elapsed() / count;
    // Block rendering, as a small target would do it.
    let mut block = [0.0f32; 480];
    let s = Instant::now();
    for sym in &all_symbols {
        let mut synth = Synth::new(sym, 1500.0, 1.0).expect("synth");
        while synth.fill(black_box(&mut block)) == block.len() {}
    }
    let waveform_blocks = s.elapsed() / count;
    let (a1, b1) = heap();

    println!(
        "waveform          {:10.3} ms/slot (whole slot, mean of {count})",
        waveform.as_secs_f64() * 1e3
    );
    println!(
        "waveform          {:10.3} ms/slot (480-sample blocks)",
        waveform_blocks.as_secs_f64() * 1e3
    );

    // Memory account: the core never allocates; these are its buffers.
    let tables = STATIC_TABLE_BYTES;
    let working = PACK_SCRATCH_BYTES
        + size_of::<Payload>()
        + size_of::<u128>()
        + size_of::<Codeword>()
        + size_of::<[u8; SYMBOL_COUNT]>()
        + size_of::<MessageText>();
    let synth = size_of::<Synth>();
    let call_table = size_of::<CallTable<64>>();
    let block_buffer = size_of::<[f32; 480]>();
    let slot_buffer = SLOT_SAMPLES * size_of::<f32>();
    let peak = tables + working + synth + call_table + slot_buffer;
    println!(
        "memory (bytes): static tables {tables}, working {working} (stack), synth {synth}, \
         call table<64> {call_table}, 480-sample block {block_buffer}, \
         15 s slot buffer {slot_buffer}"
    );
    println!(
        "heap during {} encodes and {count} slots (whole + blocks): {} allocations, {} bytes",
        messages.len(),
        a1 - a0,
        b1 - b0
    );
    println!(
        "{{\"profile\": {{\"pack\": {:.3e}, \"pack_nohashes\": {:.3e}, \"remember_hit\": {:.3e}, \
         \"remember_miss\": {:.3e}, \"crc\": {:.3e}, \"ldpc\": {:.3e}, \
         \"symbols\": {:.3e}, \"waveform\": {:.3e}, \"waveform_blocks\": {:.3e}, \
         \"tables_bytes\": {tables}, \"working_bytes\": {working}, \"synth_bytes\": {synth}, \
         \"call_table_bytes\": {call_table}, \"block_buffer_bytes\": {block_buffer}, \
         \"slot_buffer_bytes\": {slot_buffer}, \"peak_bytes\": {peak}, \"heap_bytes\": {}}}}}",
        t.pack.as_secs_f64(),
        t.pack_nohashes.as_secs_f64(),
        t.remember_hit.as_secs_f64(),
        t.remember_miss.as_secs_f64(),
        t.crc.as_secs_f64(),
        t.ldpc.as_secs_f64(),
        t.symbols.as_secs_f64(),
        waveform.as_secs_f64(),
        waveform_blocks.as_secs_f64(),
        b1 - b0
    );
}
