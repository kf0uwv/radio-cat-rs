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

//! Release lines (ADR 0022) must not leak a file descriptor.
//!
//! Its own test binary, with exactly one test, because it counts
//! `/proc/self/fd`: any other test running in parallel in the same process
//! would open and close fds underneath the count.

#![cfg(target_os = "linux")]

use cat_transport_core::Transport;
use cat_transport_serial::{SerialConfig, SerialPort};

fn open_fds() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .expect("read /proc/self/fd")
        .count()
}

#[test]
fn release_lines_hold_one_fd_and_give_it_back_when_dropped() {
    use nix::fcntl::OFlag;
    use nix::pty::{grantpt, posix_openpt, ptsname_r, unlockpt};

    let master = posix_openpt(OFlag::O_RDWR | OFlag::O_NOCTTY).expect("posix_openpt");
    grantpt(&master).expect("grantpt");
    unlockpt(&master).expect("unlockpt");
    let slave = ptsname_r(&master).expect("ptsname_r");

    let mut runtime = monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
        .build()
        .expect("monoio runtime");
    runtime.block_on(async {
        let port = SerialPort::open(&slave, SerialConfig::default()).expect("open");

        let before = open_fds();
        let lines = port.release_lines().expect("release lines");
        assert_eq!(open_fds(), before + 1, "the duplicate is one fd");

        // A second one is a second fd, and both go back.
        let second = port.release_lines().expect("release lines");
        assert_eq!(open_fds(), before + 2);
        drop(second);
        drop(lines);
        assert_eq!(open_fds(), before, "dropping the duplicates closes them");

        // Outliving the primary, it still closes on drop.
        let lines = port.release_lines().expect("release lines");
        drop(port);
        let with_only_duplicate = open_fds();
        drop(lines);
        assert_eq!(open_fds(), with_only_duplicate - 1);
    });
}
