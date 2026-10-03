//! Philox4x32-10, the counter-based generator of Salmon et al. (2011).
//!
//! Every output word is a function of a 64-bit key and a 128-bit counter, with
//! no other state, so any draw can be recomputed on its own: the result does not
//! depend on how many draws ran before it, on which thread, or on which platform.
//!
//! # Contents
//!
//! - [`philox4x32_10`]: the block function, checked against the Random123
//!   known-answer vectors.
//! - [`splitmix64`]: seed mixing. [`u32_to_unit_f32`] and [`u32_to_unit_f64`]:
//!   a word to a float on the open interval (0, 1). These two are the only
//!   floating-point code in the crate.
//! - [`Philox`]: a generator over `(key, counter)` with [`Philox::next_u32`],
//!   [`Philox::next_u64`], [`Philox::fill_u32`], [`Philox::bounded`],
//!   [`Philox::seek`] and [`Philox::key`]; [`Philox::from_u64_seed_stream`] opens one of 2⁶⁴ streams
//!   per seed.
//! - With the default `rand_core` feature: [`rand_core::TryRng`] and
//!   [`rand_core::SeedableRng`], hence `Rng` / `RngCore`.
//!
//! Without features the crate is `no_std`, allocation-free and has no
//! dependencies.
//!
//! # Example
//!
//! ```
//! use rand_philox::Philox;
//!
//! // Same (key, counter) gives the same stream.
//! let mut a = Philox::from_u64_seed(42);
//! let mut b = Philox::from_u64_seed(42);
//! assert_eq!(a.next_u32(), b.next_u32());
//!
//! // Jump to a counter position and read from there.
//! a.seek(1_000_000);
//! let x = a.next_u32();
//! b.seek(1_000_000);
//! assert_eq!(x, b.next_u32());
//!
//! // One stream per parallel task, keyed by (seed, stream).
//! let mut task = Philox::from_u64_seed_stream(42, 7);
//! let u = rand_philox::u32_to_unit_f64(task.next_u32()); // f64 in (0, 1)
//! assert!(u > 0.0 && u < 1.0);
//!
//! // Many words at once (same words as a next_u32 loop, faster).
//! let mut words = [0u32; 1024];
//! task.fill_u32(&mut words);
//! ```
#![no_std]

mod philox;

pub use philox::philox4x32_10;
use philox::{philox4x32_10_lanes, LANES};

/// The SplitMix64 output function (David Stafford's "Mix13" finalizer, also used
/// by Java's `SplittableRandom`). Mixes a raw `u64` seed into well-separated
/// bits so low-entropy seeds (1, 2, 3, …) still give independent streams.
///
/// This is the finalizer alone: unlike a SplitMix64 generator it does not add
/// the golden-ratio increment `0x9e37_79b9_7f4a_7c15` first, so
/// `splitmix64(0) == 0`, and `splitmix64(0x9e37_79b9_7f4a_7c15)` is the first
/// output of reference SplitMix64 seeded with 0. Callers needing a multi-input
/// seed combine their inputs first (xor+rotate, Weyl add, …) and pass the
/// result through this function.
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
/// argument. Floor ≈ 2⁻²⁴, cap ≈ 1 − 2⁻²⁴.
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

/// Map a Philox 32-bit word to an `f64` uniform on the **open** interval (0, 1).
///
/// Convention: all 32 bits, centred by +0.5 — `(word + 0.5) / 2³²`. The result
/// is never exactly 0 or 1, so an inverse-CDF fed by it never sees a saturating
/// argument. Floor = 2⁻³³, cap = 1 − 2⁻³³. Every step is exact in `f64` (a 33-bit
/// numerator divided by a power of two), so the value is bit-identical on every
/// host. Resolution is the 2⁻³² grid of one word.
///
/// `word`: any Philox output word. Returns a value strictly inside (0, 1).
#[inline]
#[must_use]
pub fn u32_to_unit_f64(word: u32) -> f64 {
    (f64::from(word) + 0.5) * (1.0 / 4_294_967_296.0) // (w+0.5) · 2⁻³²
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

/// Blocks `w0, w0 + 1, …` (counter words 1–3 fixed to `hi`) written to `out`,
/// `LANES` blocks per step through [`philox4x32_10_lanes`]. `out.len()` is a
/// multiple of the step and the run must not carry out of word 0. Kept out of
/// line: inlined into `fill_u32` it compiles slower on `wasm32` with `simd128`.
#[inline(never)]
fn fill_run(key: [u32; 2], w0: u32, hi: [u32; 3], out: &mut [u32]) {
    let mut ctr = w0;
    for chunk in out.chunks_exact_mut(4 * LANES) {
        let mut c = [[0; LANES], [hi[0]; LANES], [hi[1]; LANES], [hi[2]; LANES]];
        for (l, w) in (0u32..).zip(&mut c[0]) {
            *w = ctr.wrapping_add(l);
        }
        philox4x32_10_lanes(&mut c, key);
        for (l, block) in chunk.chunks_exact_mut(4).enumerate() {
            for (out, word) in block.iter_mut().zip(&c) {
                *out = word[l];
            }
        }
        ctr = ctr.wrapping_add(LANES as u32);
    }
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
        Self::from_u64_seed_stream(seed, 0)
    }

    /// Open stream `stream` of a `u64` seed: the key is [`splitmix64`]`(seed)` as
    /// in [`from_u64_seed`](Self::from_u64_seed), and `stream` fills the top 64
    /// bits of the counter, so the stream starts at block `stream · 2⁶⁴`.
    ///
    /// Each seed thus carries 2⁶⁴ non-overlapping streams of 2⁶⁴ blocks each —
    /// one per parallel task, draw, or purpose — and a stream's words depend only
    /// on `(seed, stream)`, never on which other streams were read. Counter words
    /// 0–1 hold the block index within the stream, words 2–3 hold `stream`.
    /// `from_u64_seed_stream(seed, 0)` equals `from_u64_seed(seed)`.
    /// [`seek`](Self::seek) takes the full 128-bit counter, not a block index
    /// within the stream: block `b` of stream `s` is
    /// `seek((u128::from(s) << 64) | u128::from(b))`.
    ///
    /// `seed`: any `u64`. `stream`: the stream index. Returns a stream positioned
    /// at its first block.
    #[inline]
    #[must_use]
    pub fn from_u64_seed_stream(seed: u64, stream: u64) -> Self {
        let k = splitmix64(seed);
        Self::new([k as u32, (k >> 32) as u32], u128::from(stream) << 64)
    }

    /// The two key words this stream was built with (as passed to
    /// [`new`](Self::new), or derived by [`from_u64_seed`](Self::from_u64_seed)).
    #[inline]
    #[must_use]
    pub fn key(&self) -> [u32; 2] {
        self.key
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
        // `>=` (not `==`) lets the optimizer drop the bounds check on `buf`.
        if self.buf_pos >= 4 {
            self.buf = philox4x32_10(counter_words(self.counter), self.key);
            self.counter = self.counter.wrapping_add(1);
            self.buf_pos = 0;
        }
        let w = self.buf[self.buf_pos];
        self.buf_pos += 1;
        w
    }

    /// Fill `dest` with the next `dest.len()` stream words: exactly the words,
    /// and the end state, of `dest.len()` calls to [`next_u32`](Self::next_u32).
    ///
    /// Whole blocks are generated directly into `dest`, several at a time on
    /// SIMD targets; prefer this over a `next_u32` loop when you need many
    /// words at once.
    pub fn fill_u32(&mut self, dest: &mut [u32]) {
        // Finish the current block so the loops below start block-aligned.
        let (head, dest) = dest.split_at_mut((4 - self.buf_pos).min(dest.len()));
        for w in head {
            *w = self.next_u32();
        }
        // Runs of whole steps counted in a `u32` copy of counter word 0 (the
        // `u128` is touched once per run: costly where the widest integer is
        // 64 bits, e.g. wasm32). A run stops before word 0 would carry.
        let bps = LANES; // blocks per step
        let mut rest = dest;
        while rest.len() >= 4 * bps {
            let [w0, w1, w2, w3] = counter_words(self.counter);
            let room = ((u32::MAX - w0) / bps as u32) as usize;
            if room == 0 {
                let (step, tail) = rest.split_at_mut(4 * bps);
                for w in step {
                    *w = self.next_u32();
                }
                rest = tail;
                continue;
            }
            let steps = (rest.len() / (4 * bps)).min(room);
            let (run, tail) = rest.split_at_mut(4 * bps * steps);
            fill_run(self.key, w0, [w1, w2, w3], run);
            self.counter = self.counter.wrapping_add((bps * steps) as u128);
            rest = tail;
        }
        for w in rest {
            *w = self.next_u32();
        }
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
    /// the set that would otherwise bias the result.
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
            // Finish the current block first so every bulk chunk starts
            // block-aligned and takes `fill_u32`'s multi-block path.
            let lead = (4 * (4 - self.buf_pos)).min(dst.len() & !3);
            let (head, dst) = dst.split_at_mut(lead);
            for b in head.chunks_exact_mut(4) {
                b.copy_from_slice(&self.next_u32().to_le_bytes());
            }
            let mut chunks = dst.chunks_exact_mut(4 * 64);
            let mut words = [0u32; 64];
            for chunk in &mut chunks {
                self.fill_u32(&mut words);
                for (b, w) in chunk.chunks_exact_mut(4).zip(&words) {
                    b.copy_from_slice(&w.to_le_bytes());
                }
            }
            let mut chunks = chunks.into_remainder().chunks_exact_mut(4);
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
    /// stream. For low-entropy `u64` seeds use [`Philox::from_u64_seed`], which
    /// avalanches via SplitMix64. `seed_from_u64` is `rand_core`'s default: PCG32
    /// fills all 24 bytes (key and counter), so it gives a different stream than
    /// `from_u64_seed`.
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

    // Pinned against Vigna's reference SplitMix64 (state += golden gamma, then
    // this finalizer): its first outputs for seed 0. Every `from_u64_seed` key
    // rides on these bits, so a changed shift or constant must fail here.
    #[test]
    fn splitmix64_matches_reference_outputs() {
        const GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;
        assert_eq!(splitmix64(GAMMA), 0xe220_a839_7b1d_cdaf);
        assert_eq!(splitmix64(GAMMA.wrapping_mul(2)), 0x6e78_9e6a_a1b9_65f4);
    }

    // (w >> 9 + 0.5) · 2⁻²³ is exact: extremes land on 2⁻²⁴ and 1 − 2⁻²⁴.
    #[test]
    fn unit_f32_exact_endpoints() {
        let eps = 1.0 / 16_777_216.0; // 2⁻²⁴
        assert_eq!(u32_to_unit_f32(0), eps);
        assert_eq!(u32_to_unit_f32(u32::MAX), 1.0 - eps);
        assert_eq!(u32_to_unit_f32(1 << 31), 0.5 + eps);
    }

    // (w + 0.5) / 2³² is exact: the extremes land on 2⁻³³ and 1 − 2⁻³³.
    #[test]
    fn unit_f64_exact_endpoints() {
        let eps = 1.0 / 8_589_934_592.0; // 2⁻³³
        assert_eq!(u32_to_unit_f64(0), eps);
        assert_eq!(u32_to_unit_f64(u32::MAX), 1.0 - eps);
        assert_eq!(u32_to_unit_f64(1 << 31), 0.5 + eps);
    }

    // Pins the layout: block index in counter words 0–1, stream in words 2–3,
    // key = the two halves of splitmix64(seed); stream 0 is `from_u64_seed`.
    #[test]
    fn seed_stream_counter_layout() {
        let k = splitmix64(7);
        let key = [k as u32, (k >> 32) as u32];
        for stream in [0, 0x0123_4567_89ab_cdef_u64] {
            let (lo, hi) = (stream as u32, (stream >> 32) as u32);
            let mut r = if stream == 0 {
                Philox::from_u64_seed(7)
            } else {
                Philox::from_u64_seed_stream(7, stream)
            };
            assert_eq!(r.key(), key);
            for block in 0..3u32 {
                for want in philox4x32_10([block, 0, lo, hi], key) {
                    assert_eq!(r.next_u32(), want);
                }
            }
        }
    }

    // `seek` lands on the start of the block and discards a half-read buffer.
    // The counter has four distinct nonzero words, so each word's position in
    // the little-endian split is pinned.
    #[test]
    fn seek_addresses_blocks_and_discards_buffer() {
        let key = [42, 7];
        let mut r = Philox::new(key, 0);
        r.next_u32(); // leave three buffered words behind
        r.seek(0x0000_0003_0000_0002_0000_0001_0000_0004);
        for want in philox4x32_10([4, 1, 2, 3], key) {
            assert_eq!(r.next_u32(), want);
        }
    }

    // The bulk path must be invisible: same words and same end state as a
    // `next_u32` loop, from every in-block offset, across the multi-lane chunk
    // boundary, and across the 128-bit counter wrap (lane counters carry).
    #[test]
    fn fill_u32_equals_next_u32_loop() {
        let mut buf = [0u32; 4 * LANES * 2 + 7];
        for start in [0u128, 5, u128::MAX - 9, (1u128 << 64) - 3] {
            for skip in 0..4 {
                for len in [0, 1, 3, 4, 5, 4 * LANES - 1, 4 * LANES, buf.len()] {
                    let mut seq = Philox::new([0x1234_5678, 0x9abc_def0], start);
                    let mut bulk = seq.clone();
                    for _ in 0..skip {
                        seq.next_u32();
                        bulk.next_u32();
                    }
                    bulk.fill_u32(&mut buf[..len]);
                    for (i, &w) in buf[..len].iter().enumerate() {
                        assert_eq!(
                            w,
                            seq.next_u32(),
                            "start {start} skip {skip} len {len} word {i}"
                        );
                    }
                    for _ in 0..5 {
                        assert_eq!(bulk.next_u32(), seq.next_u32(), "end state, len {len}");
                    }
                }
            }
        }
    }

    #[test]
    fn next_u64_is_two_words_low_first() {
        let mut a = Philox::from_u64_seed(99);
        let lo = u64::from(a.next_u32());
        let hi = u64::from(a.next_u32());
        let mut b = Philox::from_u64_seed(99);
        assert_eq!(b.next_u64(), lo | (hi << 32));
    }

    // `bounded` is Lemire's method on the word stream, exactly: same values and
    // same word consumption as the textbook loop below (threshold 2³² mod n,
    // reject while the low half is under it). Unbiasedness is Lemire's theorem;
    // pinning the algorithm also pins the resample indices consumers derive.
    // 3·10⁹ rejects ~30 % of words, so the rejection loop is exercised.
    #[test]
    fn bounded_is_lemire_on_the_stream() {
        let mut r = Philox::from_u64_seed(11);
        let mut words = r.clone();
        for n in [1u32, 2, 7, 1000, 1 << 31, 3_000_000_000, u32::MAX] {
            let t = ((1u64 << 32) % u64::from(n)) as u32;
            for _ in 0..2000 {
                let want = loop {
                    let m = u64::from(words.next_u32()) * u64::from(n);
                    if (m as u32) >= t {
                        break (m >> 32) as u32;
                    }
                };
                assert_eq!(r.bounded(n), want, "bounded({n})");
            }
        }
    }
}
