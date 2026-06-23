//! Counter-based Philox4x32-10 RNG with a reproducibility contract.
//!
//! The headline is not the algorithm (Philox is well known) but the contract:
//! **bit-identical output across hosts and thread counts, KAT-pinned, `no_std`,
//! no float in the core, drop-in for `rand_chacha`.** Every word is a pure
//! function of `(key, counter)` with no carried entropy, so re-running any draw
//! reproduces it exactly — independent of how many draws ran before it or on
//! which thread. That is the slot the `rand` ecosystem leaves empty.
//!
//! # Layers
//!
//! - [`philox4x32_10`] — the block function, the KAT-frozen primitive.
//! - [`splitmix64`] / [`u32_to_unit_f32`] — generic leaf helpers (seed avalanche;
//!   word → open-interval `f32`). The single float touch in the crate.
//! - [`Philox`] — counter-addressable RNG: `(key, 128-bit counter)` → reproducible
//!   words at any counter position, with buffered [`Philox::next_u32`] /
//!   [`Philox::next_u64`], [`Philox::bounded`], and [`Philox::seek`].
//! - `rand_core` impls ([`rand_core::TryRng`] / [`rand_core::SeedableRng`], hence
//!   `Rng` / `RngCore`) — the ecosystem drop-in. Behind the default-on
//!   `rand_core` feature.
//!
//! The block function, the helpers, and [`Philox`] are `no_std`, zero-dependency,
//! and allocation-free — the WASM-clean surface. The `rand_core` feature adds the
//! one small `no_std` dependency.
//!
//! # Example
//!
//! ```
//! use rand_philox::Philox;
//!
//! // Same (key, counter) reproduces the same stream, anywhere.
//! let mut a = Philox::from_u64_seed(42);
//! let mut b = Philox::from_u64_seed(42);
//! assert_eq!(a.next_u32(), b.next_u32());
//!
//! // Jump to any counter position and read deterministically.
//! a.seek(1_000_000);
//! let x = a.next_u32();
//! b.seek(1_000_000);
//! assert_eq!(x, b.next_u32());
//! ```
#![no_std]

mod philox;

pub use philox::philox4x32_10;

/// David Stafford's "Mix13" SplitMix64 finalizer — the avalanche function also
/// used by Java's `SplittableRandom`. Mixes a raw `u64` seed into well-separated
/// bits so low-entropy standalone seeds (0, 1, 2, …) still produce independent
/// streams.
///
/// Validated against Stafford's published Mix13 constants; byte-identical to the
/// finalizer in MCPower and CommonStats (this is the shared copy they extract).
/// Callers needing a multi-input seed combine their inputs first (xor+rotate,
/// Weyl add, …) and pass the result through this finalizer.
///
/// `z`: the raw seed word. Returns the avalanched word.
#[inline]
#[must_use]
pub fn splitmix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Map a Philox 32-bit word to an `f32` uniform on the **open** interval (0, 1).
///
/// Convention: 23-bit mantissa (`word >> 9`) centred by +0.5 so the result is
/// never exactly 0 or 1 — an inverse-CDF fed by this never sees a saturating
/// argument. Floor ≈ 2⁻²⁴, cap ≈ 1 − 2⁻²⁴. Byte-identical to MCPower's
/// `u32_to_unit_f32`.
///
/// 23 bits rather than 24: in `f32`, values ≥ 2²³ = 8_388_608 have ULP ≥ 1.0, so
/// `(max_24bit as f32) + 0.5` would round to 2²⁴ and the result would hit 1.0
/// exactly. In `[2²², 2²³)` the ULP is 0.5, so every half-integer is exactly
/// representable and the +0.5 centering is guaranteed to stay below 1.0.
///
/// `word`: any Philox output word. Returns a value strictly inside (0, 1).
#[inline]
#[must_use]
pub fn u32_to_unit_f32(word: u32) -> f32 {
    ((word >> 9) as f32 + 0.5) * (1.0 / 8_388_608.0) // (x+0.5) · 2⁻²³
}

/// Counter-addressable Philox4x32-10 RNG.
///
/// Holds a 64-bit `key` and a 128-bit block `counter`. Each
/// [`philox4x32_10`] call consumes one counter block and yields four `u32`
/// words; [`next_u32`](Self::next_u32) serves them from an internal buffer and
/// advances the counter on refill. Because every word is a pure function of
/// `(key, counter, position-in-block)`, the stream is reproducible across hosts
/// and thread counts, and [`seek`](Self::seek) jumps to any block in O(1).
///
/// **Counter convention (frozen):** the `counter` argument to
/// [`new`](Self::new) / [`seek`](Self::seek) is a **block index** — Philox's
/// native 128-bit counter, mapped to its four `u32` words little-endian. One
/// increment advances by one block = four output words. `seek(c)` positions the
/// stream at the start of block `c`.
#[derive(Debug, Clone)]
pub struct Philox {
    key: [u32; 2],
    counter: u128, // next block to generate
    buf: [u32; 4],
    buf_pos: usize, // 0..=4; 4 = exhausted, refill on next draw
}

/// Split a 128-bit block counter into Philox's four little-endian counter words.
#[inline]
fn counter_words(c: u128) -> [u32; 4] {
    [
        c as u32,
        (c >> 32) as u32,
        (c >> 64) as u32,
        (c >> 96) as u32,
    ]
}

impl Philox {
    /// Open a stream at block `counter` with an explicit 64-bit `key` (two `u32`
    /// words). For a single `u64` seed without a prepared key, use
    /// [`from_u64_seed`](Self::from_u64_seed).
    ///
    /// `key`: the two Philox key words, used verbatim (no mixing). `counter`: the
    /// starting block index (see the counter convention on [`Philox`]).
    #[inline]
    #[must_use]
    pub fn new(key: [u32; 2], counter: u128) -> Self {
        Self {
            key,
            counter,
            buf: [0; 4],
            buf_pos: 4,
        }
    }

    /// Open a stream from a single `u64` seed: the seed is run through
    /// [`splitmix64`] to fill the two key words, and the counter starts at 0.
    ///
    /// Use this for low-entropy or sequential seeds — the avalanche keeps streams
    /// from adjacent seeds independent.
    ///
    /// `seed`: any `u64`. Returns a stream positioned at block 0.
    #[inline]
    #[must_use]
    pub fn from_u64_seed(seed: u64) -> Self {
        let k = splitmix64(seed);
        Self::new([k as u32, (k >> 32) as u32], 0)
    }

    /// Reposition the stream to the start of block `counter`, discarding any
    /// buffered words.
    ///
    /// `counter`: the block index to jump to (see the counter convention on
    /// [`Philox`]). After `seek(c)`, the next [`next_u32`](Self::next_u32) returns
    /// the first word of block `c`.
    #[inline]
    pub fn seek(&mut self, counter: u128) {
        self.counter = counter;
        self.buf_pos = 4;
    }

    /// Next pseudo-random 32-bit word. Refills a four-word Philox block when the
    /// buffer is exhausted and advances the block counter; the refill is keyed by
    /// `(key, counter)`, not by carried state, so the stream reproduces across a
    /// block boundary exactly.
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        if self.buf_pos == 4 {
            self.buf = philox4x32_10(counter_words(self.counter), self.key);
            self.counter = self.counter.wrapping_add(1);
            self.buf_pos = 0;
        }
        let w = self.buf[self.buf_pos];
        self.buf_pos += 1;
        w
    }

    /// Next pseudo-random 64-bit word.
    ///
    /// Convention (frozen): two consecutive stream words, low word first —
    /// `lo | (hi << 32)`. Consumes two words from the [`next_u32`](Self::next_u32)
    /// stream.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let lo = u64::from(self.next_u32());
        let hi = u64::from(self.next_u32());
        lo | (hi << 32)
    }

    /// Unbiased uniform integer in `[0, n)` via Lemire's method (no modulo bias,
    /// unlike `floor(uniform * n)`). Draws extra words only in the rare rejection
    /// zone, so it stays integer-only and reproducible.
    ///
    /// Lemire (2019), "Fast Random Integer Generation in an Interval": form the
    /// 64-bit product `m = word · n`; its high 32 bits are the candidate. Reject
    /// only when the low 32 bits fall below the threshold `t = 2³² mod n`, exactly
    /// the set that would otherwise bias the result. Byte-identical to
    /// CommonStats' `bounded`.
    ///
    /// `n`: the exclusive upper bound; must be ≥ 1. `n == 1` always returns 0.
    /// Returns a value in `[0, n)`.
    #[inline]
    pub fn bounded(&mut self, n: u32) -> u32 {
        debug_assert!(n >= 1, "bounded: n must be >= 1");
        let n64 = u64::from(n);
        let mut m = u64::from(self.next_u32()) * n64;
        let mut lo = m as u32; // low 32 bits of the product
        if lo < n {
            let t = (1u64 << 32).wrapping_rem(n64) as u32; // 2^32 mod n
            while lo < t {
                m = u64::from(self.next_u32()) * n64;
                lo = m as u32;
            }
        }
        (m >> 32) as u32
    }
}

#[cfg(feature = "rand_core")]
mod rand_core_impl {
    use crate::Philox;
    use rand_core::{Infallible, SeedableRng, TryRng};

    /// Drop-in `rand` support. `TryRng` with `Error = Infallible` confers
    /// `Rng` / `RngCore` / `CryptoRng`-free blanket impls in `rand_core` 0.10.
    impl TryRng for Philox {
        type Error = Infallible;

        #[inline]
        fn try_next_u32(&mut self) -> Result<u32, Infallible> {
            Ok(self.next_u32())
        }

        #[inline]
        fn try_next_u64(&mut self) -> Result<u64, Infallible> {
            Ok(self.next_u64())
        }

        #[inline]
        fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
            let mut chunks = dst.chunks_exact_mut(4);
            for chunk in &mut chunks {
                chunk.copy_from_slice(&self.next_u32().to_le_bytes());
            }
            let rem = chunks.into_remainder();
            if !rem.is_empty() {
                // A partial tail still consumes one whole word — the standard
                // fill_bytes behaviour; reproducible because it is deterministic.
                let bytes = self.next_u32().to_le_bytes();
                rem.copy_from_slice(&bytes[..rem.len()]);
            }
            Ok(())
        }
    }

    /// Seed layout (frozen): 24 bytes = `key[0] ‖ key[1] ‖ counter`, all
    /// little-endian (8-byte key + 16-byte counter). A transparent bijection onto
    /// [`Philox::new`] — no hidden mixing, so a seed round-trips to exactly its
    /// stream. For low-entropy `u64` seeds use [`Philox::from_u64_seed`] (or
    /// `SeedableRng::seed_from_u64`), which avalanches.
    impl SeedableRng for Philox {
        type Seed = [u8; 24];

        #[inline]
        fn from_seed(seed: [u8; 24]) -> Self {
            let k0 = u32::from_le_bytes([seed[0], seed[1], seed[2], seed[3]]);
            let k1 = u32::from_le_bytes([seed[4], seed[5], seed[6], seed[7]]);
            let mut cb = [0u8; 16];
            cb.copy_from_slice(&seed[8..24]);
            Philox::new([k0, k1], u128::from_le_bytes(cb))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix64_distinct_for_adjacent_seeds() {
        // Adjacent seeds avalanche to well-separated words — the property that
        // keeps low-entropy sequential seeds (0, 1, 2, …) independent. (Note the
        // Mix13 finalizer fixes 0 → 0, so don't assert 0 maps off itself.)
        assert_ne!(splitmix64(0), splitmix64(1));
        assert_ne!(splitmix64(1), splitmix64(2));
        assert_ne!(splitmix64(2), splitmix64(3));
    }

    #[test]
    fn unit_f32_in_open_interval() {
        // Field extremes map strictly inside (0,1), and ordered.
        assert!(u32_to_unit_f32(0) > 0.0);
        assert!(u32_to_unit_f32(u32::MAX) < 1.0);
        assert!(u32_to_unit_f32(0) < u32_to_unit_f32(u32::MAX));
        // Dense scan stays in range.
        for k in 0..10_000u32 {
            let w = k.wrapping_mul(0x9E37_79B9);
            let u = u32_to_unit_f32(w);
            assert!(u > 0.0 && u < 1.0, "u={u} out of (0,1)");
        }
    }

    #[test]
    fn same_key_counter_reproduces_stream() {
        let mut a = Philox::new([0xdead_beef, 0x1234_5678], 0);
        let mut b = Philox::new([0xdead_beef, 0x1234_5678], 0);
        for _ in 0..1000 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }

    #[test]
    fn reproduces_across_block_boundaries() {
        // 37 words crosses several 4-word block refills; the keyed refill must
        // not depend on carried state.
        let mut a = Philox::from_u64_seed(1);
        let mut first = [0u32; 37];
        for w in &mut first {
            *w = a.next_u32();
        }
        let mut b = Philox::from_u64_seed(1);
        for &w in &first {
            assert_eq!(b.next_u32(), w);
        }
    }

    #[test]
    fn seek_addresses_blocks_deterministically() {
        let key = [42, 7];
        // Read the 5th block (words 16..20) sequentially...
        let mut seq = Philox::new(key, 0);
        let mut want = [0u32; 4];
        for _ in 0..16 {
            seq.next_u32();
        }
        for w in &mut want {
            *w = seq.next_u32();
        }
        // ...and by seeking straight to block 4.
        let mut jumped = Philox::new(key, 0);
        jumped.seek(4);
        for &w in &want {
            assert_eq!(jumped.next_u32(), w);
        }
    }

    #[test]
    fn different_keys_and_counters_diverge() {
        // Different keys.
        let mut a = Philox::new([42, 0], 0);
        let mut b = Philox::new([42, 1], 0);
        let mut diff = 0usize;
        for _ in 0..100 {
            if a.next_u32() != b.next_u32() {
                diff += 1;
            }
        }
        assert!(diff > 90, "different keys must give independent streams");
        // Different starting counters.
        let mut c = Philox::new([7, 7], 0);
        let mut d = Philox::new([7, 7], 1_000);
        let mut diff2 = 0usize;
        for _ in 0..100 {
            if c.next_u32() != d.next_u32() {
                diff2 += 1;
            }
        }
        assert!(
            diff2 > 90,
            "different counters must give independent streams"
        );
    }

    #[test]
    fn next_u64_is_two_words_low_first() {
        let mut a = Philox::from_u64_seed(99);
        let lo = u64::from(a.next_u32());
        let hi = u64::from(a.next_u32());
        let mut b = Philox::from_u64_seed(99);
        assert_eq!(b.next_u64(), lo | (hi << 32));
    }

    #[test]
    fn bounded_one_is_always_zero() {
        let mut r = Philox::from_u64_seed(3);
        for _ in 0..1000 {
            assert_eq!(r.bounded(1), 0);
        }
    }

    #[test]
    fn bounded_stays_in_range() {
        let mut r = Philox::from_u64_seed(11);
        for &n in &[2u32, 3, 7, 10, 100, 1000] {
            for _ in 0..5000 {
                assert!(r.bounded(n) < n, "bounded({n}) out of range");
            }
        }
    }

    // No modulo bias: over many draws every bucket in [0, n) is hit with roughly
    // equal frequency. A biased floor(u*n) would over-fill the low buckets; this
    // χ²-style spread check catches gross deviation.
    #[test]
    fn bounded_is_approximately_uniform() {
        let n = 7u32;
        let draws = 700_000usize;
        let mut counts = [0u64; 7];
        let mut r = Philox::from_u64_seed(2024);
        for _ in 0..draws {
            counts[r.bounded(n) as usize] += 1;
        }
        let expected = draws as f64 / f64::from(n);
        for (i, &c) in counts.iter().enumerate() {
            // Manual abs: `f64::abs` is std-only, unavailable in this no_std crate.
            let d = c as f64 - expected;
            let rel = (if d < 0.0 { -d } else { d }) / expected;
            assert!(
                rel < 0.02,
                "bucket {i} count {c} deviates {rel:.4} from uniform"
            );
        }
    }
}
