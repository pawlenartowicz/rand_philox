//! `rand_core` conformance (feature `rand_core`, default on): the `SeedableRng`
//! / `Rng` drop-in surface behaves and reproduces.
#![cfg(feature = "rand_core")]

use rand_core::{Rng, SeedableRng};
use rand_philox::Philox;

// The frozen seed layout: 24 bytes = key[0] ‖ key[1] ‖ counter, all LE, a
// transparent bijection onto `Philox::new`.
#[test]
fn from_seed_matches_new() {
    let mut seed = [0u8; 24];
    for (i, b) in seed.iter_mut().enumerate() {
        *b = i as u8 + 1;
    }
    let k0 = u32::from_le_bytes([1, 2, 3, 4]);
    let k1 = u32::from_le_bytes([5, 6, 7, 8]);
    let mut cb = [0u8; 16];
    cb.copy_from_slice(&seed[8..24]);
    let counter = u128::from_le_bytes(cb);

    let mut via_seed = Philox::from_seed(seed);
    let mut via_new = Philox::new([k0, k1], counter);
    for _ in 0..16 {
        assert_eq!(via_seed.next_u32(), via_new.next_u32());
    }
}

// `fill_bytes` is the little-endian word stream: whole words, then a partial
// tail that consumes one whole word and keeps its leading bytes. The lengths
// straddle the 256-byte bulk chunk, from a block-aligned and a mid-block start.
#[test]
fn fill_bytes_is_the_le_word_stream() {
    for skip in [0, 1] {
        for len in [0usize, 1, 6, 16, 255, 256, 257, 1029] {
            let mut a = Philox::from_seed([7u8; 24]);
            for _ in 0..skip {
                a.next_u32();
            }
            let mut b = a.clone();
            let mut buf = vec![0u8; len];
            a.fill_bytes(&mut buf);
            let mut expect = Vec::with_capacity(len + 4);
            while expect.len() < len {
                expect.extend_from_slice(&b.next_u32().to_le_bytes());
            }
            assert_eq!(buf, expect[..len], "skip {skip} len {len}");
            assert_eq!(
                a.next_u32(),
                b.next_u32(),
                "end state, skip {skip} len {len}"
            );
        }
    }
}
