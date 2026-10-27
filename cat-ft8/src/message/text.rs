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

//! Fixed-capacity ASCII strings, so the core needs no allocator.

use core::fmt;
use core::ops::Deref;

/// A fixed-capacity ASCII string of at most `N` bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AsciiBuf<const N: usize> {
    buf: [u8; N],
    len: u8,
}

impl<const N: usize> AsciiBuf<N> {
    /// The empty string.
    pub const fn new() -> Self {
        AsciiBuf {
            buf: [0; N],
            len: 0,
        }
    }

    /// Copies `s`; `None` if it is longer than `N` or not ASCII.
    pub fn from_ascii(s: &str) -> Option<Self> {
        let mut b = Self::new();
        b.push_str(s).ok()?;
        Some(b)
    }

    /// The contents.
    pub fn as_str(&self) -> &str {
        // Only ASCII bytes are ever stored, so this cannot fail.
        core::str::from_utf8(&self.buf[..usize::from(self.len)]).unwrap_or("")
    }

    /// Appends `s`, or leaves the buffer unchanged if it would not fit or
    /// is not ASCII.
    pub fn push_str(&mut self, s: &str) -> Result<(), fmt::Error> {
        let start = usize::from(self.len);
        let end = start + s.len();
        if end > N || end > usize::from(u8::MAX) || !s.is_ascii() {
            return Err(fmt::Error);
        }
        self.buf[start..end].copy_from_slice(s.as_bytes());
        self.len = end as u8;
        Ok(())
    }

    /// Appends one ASCII byte.
    pub fn push(&mut self, c: u8) -> Result<(), fmt::Error> {
        let mut tmp = [0u8; 4];
        self.push_str((c as char).encode_utf8(&mut tmp))
    }
}

impl<const N: usize> Default for AsciiBuf<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Deref for AsciiBuf<N> {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl<const N: usize> fmt::Write for AsciiBuf<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_str(s)
    }
}

impl<const N: usize> fmt::Display for AsciiBuf<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<const N: usize> fmt::Debug for AsciiBuf<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl<const N: usize> PartialEq<str> for AsciiBuf<N> {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl<const N: usize> PartialEq<&str> for AsciiBuf<N> {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

/// An unpacked message, at most 64 characters (the longest FT8 message
/// is about 50).
pub type MessageText = AsciiBuf<64>;

/// A callsign of at most 11 characters (the longest FT8 can carry).
pub type Call = AsciiBuf<11>;
