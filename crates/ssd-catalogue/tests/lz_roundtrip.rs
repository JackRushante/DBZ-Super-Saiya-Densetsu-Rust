//! The LZ encoder of `ssd_mod::lz` round-trips through the game's decoder (`ssd_catalogue::graphics::lz_decode`, the
//! `00:C559` format of `docs/re/data/graphics.md`): for random and for structured inputs, the decoded bytes are the
//! encoded ones. No ROM is read.

use ssd_catalogue::graphics::lz_decode;
use ssd_mod::lz::encode;

/// A xorshift generator: the same bytes on every run.
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

/// Encodes `data`, decodes the stream at offset 0 and checks that it reads back as `data`.
fn round_trip(data: &[u8]) {
    let stream = encode(data).expect("a non-empty input of at most 65535 bytes encodes");
    let decoded = lz_decode(&stream, 0).expect("the decoder accepts the stream");
    assert_eq!(decoded.data, data, "{} bytes", data.len());
    assert_eq!(decoded.consumed, stream.len(), "the stream ends where the decoder stops");
}

#[test]
fn random_inputs_of_many_lengths_round_trip() {
    for (len, seed) in [1, 2, 3, 7, 8, 9, 17, 100, 255, 256, 257, 1000, 4096, 4097, 20_000, 65_535]
        .into_iter()
        .zip(1_u32..)
    {
        round_trip(&noise(len, seed.wrapping_mul(0x9E37_79B9) | 1));
    }
}

#[test]
fn structured_inputs_round_trip() {
    round_trip(&[0; 1]);
    round_trip(&[0xAB; 18]);
    round_trip(&[0xAB; 19]);
    round_trip(&[0; 5000]);
    // Periodic data: a copy of a long run from a distance of one period, up to the farthest distance of 4095.
    for period in [1_usize, 2, 3, 5, 16, 255, 4095] {
        let unit = noise(period, period as u32 + 77);
        let data: Vec<u8> = unit.iter().copied().cycle().take(12_000).collect();
        round_trip(&data);
    }
    // A mixture of runs, noise and repeats, as a tile set has.
    let mut mixed = Vec::new();
    for k in 0..200_u32 {
        mixed.extend(noise(usize::try_from(k % 13).unwrap() + 1, k + 3));
        mixed.extend(std::iter::repeat_n((k % 7) as u8, usize::try_from(k % 23).unwrap()));
        mixed.extend_from_slice(b"SSD-LZ-TEST");
    }
    round_trip(&mixed);
}

#[test]
fn an_empty_input_has_no_stream_and_an_oversized_one_has_none() {
    assert_eq!(encode(&[]), None);
    assert_eq!(encode(&vec![0; 65_536]), None);
}
