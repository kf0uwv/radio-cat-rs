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

//! What SDRs does this machine see?
//!
//! `cargo run -p cat-signal-rtlsdr --features device --example enumerate`
//!
//! A diagnostic for the question "is it my dongle or my software", answered
//! without a console in the way. It prints the same `DeviceList` a picker
//! is handed, so what it shows is what an operator would have been offered.

fn main() {
    let list = cat_signal_rtlsdr::device::devices();

    match &list.error {
        // Not "no devices": the enumeration itself could not be performed.
        Some(why) => println!("cannot enumerate SDRs: {why}"),
        None if list.devices.is_empty() => {
            println!("no SDRs attached (the enumeration worked; nothing is plugged in)")
        }
        None => {
            println!("{} SDR(s):", list.devices.len());
            for d in &list.devices {
                println!("  {:<10} {}", d.spec, d.label);
                if let Some(detail) = &d.detail {
                    println!("             {detail}");
                }
            }
            println!("\npass one to a console as: --if-out <spec>");
        }
    }
}
