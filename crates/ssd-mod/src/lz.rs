//! The encoder of the LZ stream the game's decoder reads (`00:C559`, `docs/re/data/graphics.md`): a `u16` size, then
//! groups of a flag byte and up to eight items, the flags read from the least significant bit. A set bit is a literal
//! byte; a clear bit is a copy of 2 bytes, `first` (the length minus 3 in its low nibble, the low nibble of the distance
//! in its high nibble) and `second` (the distance's high byte), a distance from 1 to 4095.
//!
//! The encoder is greedy: at each position it takes the longest earlier run of at least 3 bytes (at most 18 long,
//! at most 4095 back). It is the inverse of `ssd_catalogue::graphics::lz_decode`, which the tests of
//! `ssd-catalogue` check it against.

use std::collections::HashMap;

/// The longest run a copy can name (length nibble 15 plus 3).
const MAX_RUN: usize = 18;
/// The shortest run worth a copy (a copy costs 2 bytes, a literal 1, plus its flag bit).
const MIN_RUN: usize = 3;
/// The farthest back a copy can reach (a 12-bit distance).
const MAX_DISTANCE: usize = 4095;
/// The most earlier positions with the same 3 bytes that are looked at for a run.
const CANDIDATES: usize = 256;

/// Encodes `data` as an LZ stream. `None` when `data` is empty (the stream of an empty set has no size the decoder
/// accepts) or longer than the `u16` size allows (65 535 bytes).
#[must_use]
pub fn encode(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() || data.len() > usize::from(u16::MAX) {
        return None;
    }
    let items = tokenize(data);
    let mut out = Vec::with_capacity(data.len() + data.len() / 8 + 4);
    out.extend_from_slice(&(data.len() as u16).to_le_bytes());
    for group in items.chunks(8) {
        let mut flags = 0_u8;
        let flag_at = out.len();
        out.push(0);
        for (bit, item) in group.iter().enumerate() {
            match *item {
                Item::Literal(byte) => {
                    flags |= 1 << bit;
                    out.push(byte);
                }
                Item::Copy { length, distance } => {
                    let first = ((length - MIN_RUN) as u8) | (((distance & 0x0F) as u8) << 4);
                    out.push(first);
                    out.push((distance >> 4) as u8);
                }
            }
        }
        out[flag_at] = flags;
    }
    Some(out)
}

/// One item of the stream, before the flag bytes are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Literal(u8),
    Copy { length: usize, distance: usize },
}

/// The greedy parse of `data`: the longest run at each position, or a literal.
fn tokenize(data: &[u8]) -> Vec<Item> {
    let mut seen: HashMap<[u8; 3], Vec<usize>> = HashMap::new();
    let mut items = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let mut best = (0_usize, 0_usize);
        if pos + MIN_RUN <= data.len()
            && let Some(earlier) = seen.get(&[data[pos], data[pos + 1], data[pos + 2]])
        {
            for &from in earlier.iter().rev().take(CANDIDATES) {
                let distance = pos - from;
                if distance > MAX_DISTANCE {
                    break;
                }
                let limit = MAX_RUN.min(data.len() - pos);
                let length = (0..limit).take_while(|&k| data[from + k] == data[pos + k]).count();
                if length > best.0 {
                    best = (length, distance);
                }
            }
        }
        let step = if best.0 >= MIN_RUN {
            items.push(Item::Copy { length: best.0, distance: best.1 });
            best.0
        } else {
            items.push(Item::Literal(data[pos]));
            1
        };
        for at in pos..pos + step {
            if at + MIN_RUN <= data.len() {
                seen.entry([data[at], data[at + 1], data[at + 2]]).or_default().push(at);
            }
        }
        pos += step;
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A xorshift generator: the tests need no random crate and the same bytes every run.
    fn noise(len: usize, mut state: u32) -> Vec<u8> {
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state >> 8) as u8
            })
            .collect()
    }

    #[test]
    fn an_empty_input_has_no_stream() {
        assert_eq!(encode(&[]), None);
    }

    #[test]
    fn a_single_byte_is_one_flag_and_one_literal() {
        assert_eq!(encode(&[0x5A]), Some(vec![0x01, 0x00, 0x01, 0x5A]));
    }

    #[test]
    fn a_run_of_one_byte_is_one_literal_and_one_copy_of_fifteen() {
        // Flags 0b01: a literal, then a copy (bit clear). The copy's first byte is the length nibble 12 (15 bytes)
        // with the distance's low nibble 1 above it; its second byte is the distance's high byte, 0.
        let stream = encode(&[0x77; 16]).unwrap();
        assert_eq!(stream, vec![0x10, 0x00, 0x01, 0x77, 0x1C, 0x00]);
    }

    #[test]
    fn the_size_is_the_length_of_the_input() {
        let data = noise(300, 7);
        let stream = encode(&data).unwrap();
        assert_eq!(u16::from_le_bytes([stream[0], stream[1]]) as usize, 300);
    }
}
