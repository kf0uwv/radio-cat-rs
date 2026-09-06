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

//! Ask a running server what signal hardware its machine can see.
//!
//! ```text
//! cargo run -p cat-native --example ask_devices -- 127.0.0.1:7400
//! cargo run -p cat-native --example ask_devices -- 127.0.0.1:7400 rtl:0
//! ```
//!
//! With a spec, it attaches that device instead of listing -- the other
//! half of what a picker does, and the half whose failures carry the
//! host's own words.
//!
//! A diagnostic for the question a graphical console asks silently on
//! every connection. When a picker comes up empty, this says whether the
//! server declined, found nothing, or could not look -- three answers a
//! console deliberately renders differently and which are easy to confuse
//! from the outside.

fn main() {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7400".to_string());

    let mut client =
        match cat_native::Connection::connect(addr.as_str(), cat_native::Streams::none()) {
            Ok(client) => client,
            Err(e) => {
                eprintln!("could not reach {addr}: {e}");
                std::process::exit(1);
            }
        };
    println!("connected to {addr} ({})", client.capabilities().model);

    if let Some(spec) = std::env::args().nth(2) {
        let lists = client.read_devices().ok().flatten().unwrap_or_default();
        let Some(device) = lists
            .iter()
            .flat_map(|l| l.devices.iter())
            .find(|d| d.spec == spec)
        else {
            eprintln!("the server does not offer {spec:?}");
            std::process::exit(1);
        };
        match client.attach_device(device.kind, &spec) {
            Ok(()) => println!("attached {spec}"),
            Err(e) => {
                eprintln!("the server refused: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    match client.read_devices() {
        Ok(None) => println!("this server does not offer device selection"),
        Ok(Some(lists)) => {
            for list in lists {
                println!("\n{}", list.kind.heading());
                match (&list.error, list.devices.is_empty()) {
                    (Some(why), _) => println!("  unavailable: {why}"),
                    (None, true) => println!("  nothing attached"),
                    (None, false) => {
                        for d in &list.devices {
                            let mark = if d.is_default { " (default)" } else { "" };
                            println!("  {}{mark}", d.label);
                            println!("      {} {}", d.kind.flag(), d.spec);
                        }
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("asking failed: {e}");
            std::process::exit(1);
        }
    }
}
