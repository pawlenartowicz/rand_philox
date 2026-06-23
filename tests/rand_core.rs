//! `rand_core` conformance (feature `rand_core`, default on): the `SeedableRng`
//! / `Rng` drop-in surface behaves and reproduces.
#![cfg(feature = "rand_core")]

use rand_core::{Rng, SeedableRng};
use rand_philox::Philox;

#[test]
fn seedable_reproduces_from_same_seed() {
    let seed = [9u8; 24];
    let mut a = Philox::from_seed(seed);
    let mut b = Philox::from_seed(seed);
    for _ in 0..64 {
        assert_eq!(a.next_u32(), b.next_u32());
    }
}

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

#[test]
fn fill_bytes_agrees_with_next_u32() {
    let seed = [7u8; 24];
    let mut a = Philox::from_seed(seed);
    let mut buf = [0u8; 16];
    a.fill_bytes(&mut buf);

    let mut b = Philox::from_seed(seed);
    let mut expect = [0u8; 16];
    for chunk in expect.chunks_mut(4) {
        chunk.copy_from_slice(&b.next_u32().to_le_bytes());
    }
    assert_eq!(buf, expect);
}

// A partial tail consumes one whole word and takes its leading bytes.
#[test]
fn fill_bytes_partial_tail() {
    let seed = [3u8; 24];
    let mut a = Philox::from_seed(seed);
    let mut buf = [0u8; 6];
    a.fill_bytes(&mut buf);

    let mut b = Philox::from_seed(seed);
    let w0 = b.next_u32().to_le_bytes();
    let w1 = b.next_u32().to_le_bytes();
    assert_eq!(&buf[..4], &w0);
    assert_eq!(&buf[4..6], &w1[..2]);
}

#[test]
fn seed_from_u64_reproduces() {
    let mut a = Philox::seed_from_u64(12_345);
    let mut b = Philox::seed_from_u64(12_345);
    for _ in 0..32 {
        assert_eq!(a.next_u32(), b.next_u32());
    }
}
