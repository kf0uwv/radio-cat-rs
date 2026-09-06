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

//! Show the console arrangement a server publishes.
//!
//! ```text
//! cargo run -p cat-native --example ask_layout -- 127.0.0.1:4532 [cols] [rows]
//! ```
//!
//! A diagnostic for the thing a console does silently at every handshake.
//! When two radios' consoles look alike and should not, this says whether
//! the layouts differ or the renderer is ignoring them.

fn main() {
    let mut args = std::env::args().skip(1);
    let addr = args.next().unwrap_or_else(|| "127.0.0.1:4532".to_string());
    let cols: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(120);
    let rows: u16 = args.next().and_then(|a| a.parse().ok()).unwrap_or(40);

    let client = match cat_native::Connection::connect(addr.as_str(), cat_native::Streams::none()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("could not reach {addr}: {e}");
            std::process::exit(1);
        }
    };
    let caps = client.capabilities();
    println!("{} — console layout at {cols}x{rows}", caps.model);

    let Some(spec) = &caps.layout else {
        println!("  this server publishes no layout; a console uses its own default");
        return;
    };

    let mut placed = spec.resolve(cat_layout::Area::new(0, 0, cols, rows));
    placed.sort_by_key(|p| (p.area.y, p.area.x));
    for p in placed {
        println!(
            "  {:<12} at {:>3},{:<3} {:>3}x{:<3}",
            format!("{:?}", p.kind),
            p.area.x,
            p.area.y,
            p.area.width,
            p.area.height
        );
    }
}
