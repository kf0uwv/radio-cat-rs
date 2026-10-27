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

//! CRC-14 (QEX 2020 §3): polynomial 0x6757, initial value zero, computed
//! over the 77-bit payload zero-extended to 82 bits, MSB first. No final
//! XOR and no reflection.
//!
//! Bit vectors in this crate are `u128` values holding their bits in the
//! low end, first-transmitted bit most significant: a 91-bit word `a` has
//! its bit 0 at `a >> 90`.

use crate::message::Payload;

/// The generator polynomial, x^14 + x^13 + x^10 + x^9 + x^8 + x^6 + x^4 + x^2 + x + 1.
pub const POLYNOMIAL: u16 = 0x6757;
/// Mask of the 91 message-plus-CRC bits.
pub const A91_MASK: u128 = (1u128 << 91) - 1;

/// Byte table for the direct form: entry `i` is the register after eight
/// feedback steps starting from `i` in its top eight bits. 512 bytes,
/// built at compile time.
pub(crate) static CRC_TABLE: [u16; 256] = {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut reg = (i as u16) << 6;
        let mut k = 0;
        while k < 8 {
            let top = (reg >> 13) & 1;
            reg = (reg << 1) & 0x3fff;
            if top == 1 {
                reg ^= POLYNOMIAL & 0x3fff;
            }
            k += 1;
        }
        t[i] = reg;
        i += 1;
    }
    t
};

/// CRC-14 of `payload`: the 77 bits zero-extended to 82, a byte at a time.
/// Leading zero bits do not change a CRC that starts from zero, so the 82
/// bits are read as 11 bytes (6 leading zeros). A test checks it against
/// the bit-at-a-time definition on 20 000 payloads.
pub fn crc14(payload: &Payload) -> u16 {
    let bits = payload.bits() << 5; // 82 bits in the low end of 88
    let mut reg: u16 = 0;
    for k in (0..11).rev() {
        let byte = (bits >> (8 * k)) as u8;
        let idx = ((reg >> 6) as u8) ^ byte;
        reg = ((reg << 8) & 0x3fff) ^ CRC_TABLE[usize::from(idx)];
    }
    reg
}

/// CRC-14 over the low `n` bits of `bits` (MSB first), `n <= 128`: the
/// reference definition the table form is tested against.
///
/// The direct bitwise form: each message bit is XOR-ed into the register's
/// top bit, which gives the remainder of M(x)·x^14 mod P(x) without
/// shifting 14 flushing zeros through.
#[cfg(test)]
fn crc14_bits(bits: u128, n: u32) -> u16 {
    let mut reg: u16 = 0;
    for i in (0..n).rev() {
        let inbit = ((bits >> i) & 1) as u16;
        let feedback = ((reg >> 13) & 1) ^ inbit;
        reg = (reg << 1) & 0x3fff;
        if feedback == 1 {
            reg ^= POLYNOMIAL & 0x3fff;
        }
    }
    reg
}

/// The 91-bit message-plus-CRC word: payload followed by its CRC-14.
pub fn append_crc(payload: &Payload) -> u128 {
    (payload.bits() << 14) | u128::from(crc14(payload))
}

/// True when the last 14 bits of `a91` are the CRC of its first 77.
pub fn check(a91: u128) -> bool {
    // The shifted value has at most 77 bits, so `from_bits` always succeeds.
    Payload::from_bits((a91 & A91_MASK) >> 14).is_some_and(|p| crc14(&p) == (a91 & 0x3fff) as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_form_equals_bitwise_form() {
        let mut x: u64 = 0x1234_5678_9abc_def1;
        for _ in 0..20_000 {
            // xorshift64 for test payloads
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let hi = u128::from(x);
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let bits = ((hi << 64) | u128::from(x)) & ((1 << 77) - 1);
            let p = Payload::from_bits(bits).unwrap();
            assert_eq!(crc14(&p), crc14_bits(bits << 5, 82));
        }
        for bits in [0u128, (1 << 77) - 1, 1, 1 << 76] {
            let p = Payload::from_bits(bits).unwrap();
            assert_eq!(crc14(&p), crc14_bits(bits << 5, 82));
        }
    }
}
