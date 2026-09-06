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

//! What sound cards does this machine have, and what would a console print?
//!
//! ```text
//! cargo run -p cat-signal-audio --example enumerate --features device
//! ```
//!
//! Deliberately builds **without** the `device` feature too, and that is
//! the interesting half: it is how an operator finds out that their build
//! cannot see hardware, rather than concluding that nothing is plugged in.
//! `cat-signal-rtlsdr`'s equivalent example needs `required-features`
//! because its whole device module vanishes; this one does not, because
//! `input_devices` is always there and always answers.

use cat_signal::DeviceKind;

fn main() {
    let list = cat_signal_audio::input_devices();
    println!("{}", DeviceKind::AudioInput.heading());

    if let Some(why) = &list.error {
        // "Cannot ask", which is not the same answer as "nothing found".
        println!("  unavailable: {why}");
        println!("\n  (that is not an empty list -- see cat_signal::DeviceList)");
        return;
    }

    if list.devices.is_empty() {
        println!("  none found. The enumeration worked; nothing is plugged in.");
        return;
    }

    for device in &list.devices {
        let marker = if device.is_default { "*" } else { " " };
        println!("{marker} {}", device.label);
        println!("    spec:   {}", device.spec);
        if let Some(detail) = &device.detail {
            println!("    detail: {detail}");
        }
    }
    println!(
        "\n  * = this host's default input. Any spec above is what \
         `{}` takes.",
        DeviceKind::AudioInput.flag()
    );
}
