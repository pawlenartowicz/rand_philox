# rand_philox

[![crates.io](https://img.shields.io/crates/v/rand_philox.svg)](https://crates.io/crates/rand_philox)
[![docs.rs](https://img.shields.io/docsrs/rand_philox)](https://docs.rs/rand_philox)
[![CI](https://github.com/pawlenartowicz/rand_philox/actions/workflows/ci.yml/badge.svg)](https://github.com/pawlenartowicz/rand_philox/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/rand_philox.svg)](#license)

Philox4x32-10, the counter-based generator of Salmon et al. (2011), for Rust. Every output word is a function of a 64-bit key and a 128-bit counter, so any draw can be recomputed on its own, on any thread and any platform. The crate is `no_std` and contains no `unsafe`. Its default `rand_core` feature implements the `rand_core` 0.10 traits; with default features off it has no dependencies.

## When to use it

Use it when results must replicate exactly: parallel simulations, resampling, permutation tests. Because draw *k* of stream *s* does not depend on how many draws came before it, splitting work across threads does not change the numbers.

Do not use it for cryptography; use [`rand_chacha`] instead. For single-threaded throughput where per-draw addressing does not matter, `rand_xoshiro` and `rand_pcg` are faster.

## Install

```toml
[dependencies]
rand_philox = "0.2"
```

Minimum supported Rust version is 1.85, set by `rand_core` 0.10. The `rand_core` feature is on by default. With `default-features = false` the crate has no dependencies.

## Example

```rust
use rand_philox::Philox;

// Same (key, counter) gives the same stream.
let mut a = Philox::from_u64_seed(42);
let mut b = Philox::from_u64_seed(42);
assert_eq!(a.next_u32(), b.next_u32());

// Jump to a counter position and read from there.
a.seek(1_000_000);
let x = a.next_u32();
b.seek(1_000_000);
assert_eq!(x, b.next_u32());

// One stream per parallel task, keyed by (seed, stream).
let mut task = Philox::from_u64_seed_stream(42, 7);
let u = rand_philox::u32_to_unit_f64(task.next_u32()); // f64 in (0, 1)
assert!(u > 0.0 && u < 1.0);

// Many words at once (same words as a next_u32 loop, faster).
let mut words = [0u32; 1024];
task.fill_u32(&mut words);
```

## Output conventions

These fix the output stream; changing any of them changes every draw.

- **Counter.** The `counter` given to `Philox::new` and `seek` is a block index. One block is one `philox4x32_10` call and yields four words. The counter is split into the four Philox counter words little-endian, and `seek(c)` moves to the first word of block `c`.
- **`from_u64_seed(seed)`.** Key = `splitmix64(seed)`, low 32 bits as key word 0 and high 32 bits as key word 1. `splitmix64` here is the SplitMix64 output function (Stafford's Mix13) applied to `seed` directly, without SplitMix64's golden-ratio increment, so `splitmix64(0) == 0`. The counter starts at 0.
- **`from_u64_seed_stream(seed, stream)`.** Same key; `stream` goes in the top 64 bits of the counter. That gives 2⁶⁴ streams per seed, each 2⁶⁴ blocks long. Stream 0 is `from_u64_seed(seed)`. `seek` still takes the full counter: block `b` of stream `s` is `seek((u128::from(s) << 64) | u128::from(b))`.
- **`next_u64`.** Two consecutive words, low word first: `lo | (hi << 32)`.
- **`fill_u32` / `fill_bytes`.** The same words as repeated `next_u32` calls, and the generator ends in the same state. `fill_bytes` writes each word little-endian; a tail shorter than four bytes uses the leading bytes of one more word.
- **`bounded(n)`.** Integer in `[0, n)` by Lemire's (2019) method, with no modulo bias. It takes one word per attempt and rejects when the low 32 bits of `word · n` fall below `2³² mod n`. `n` must be at least 1.
- **`u32_to_unit_f64(w)`.** `(w + 0.5) / 2³²`, exact in `f64`, in [2⁻³³, 1 − 2⁻³³]. Never 0 or 1, so it is safe to feed into an inverse CDF.
- **`u32_to_unit_f32(w)`.** `((w >> 9) + 0.5) / 2²³`, in [2⁻²⁴, 1 − 2⁻²⁴]. Only 23 bits are used because with 24 the largest value would round to 1.0 in `f32`.
- **`SeedableRng` seed.** 24 bytes: key word 0, key word 1, then the 16-byte counter, all little-endian, passed to `Philox::new` without mixing. `seed_from_u64` is `rand_core`'s default implementation (PCG32 fills all 24 bytes), so it gives a different stream from `from_u64_seed`.

## Reproducibility

The block function matches the three Random123 known-answer vectors, and the crate's tests pin `splitmix64` against reference SplitMix64 outputs. The tests also pin the stream layout and `bounded` against their definitions.

Everything except the two float helpers uses integer arithmetic only, the float helpers are exact, and all byte conversions are explicit little-endian. The output therefore does not depend on endianness, pointer width, SIMD support or optimization level. CI runs the test suite on x86_64 Linux and on `wasm32-wasip1`.

## Performance

`next_u32` generates one four-word block at a time. `fill_u32` and `fill_bytes` generate several blocks at once straight into the output, with no `unsafe` and no target-specific code: 16 blocks in a form the compiler vectorizes on x86_64 and aarch64, 8 on `wasm32` with `simd128`, and 2 elsewhere (including default `wasm32`). Compared with a `next_u32` loop they are about 2.4× faster on Apple M-series, 1.9× on x86_64 (baseline SSE2), and 1.8× (default) to 2.8× (`simd128`) on `wasm32`, under both wasmtime and V8. Use them when you need many words.

## Citing

> Salmon, J. K., Moraes, M. A., Dror, R. O., & Shaw, D. E. (2011). *Parallel random numbers: as easy as 1, 2, 3.* Proceedings of SC'11.

> Lemire, D. (2019). *Fast random integer generation in an interval.* ACM Transactions on Modeling and Computer Simulation, 29(1).

## Third-party code

The Philox constants and known-answer vectors come from Random123 (D. E. Shaw Research, BSD-3-Clause). The implementation is an independent Rust port; see [`THIRD-PARTY-NOTICES`](THIRD-PARTY-NOTICES).

The crate provides the random number source for [CommonStats](https://crates.io/crates/commonstats) and [MCPower](https://github.com/pawlenartowicz/MCPower).

## License

Dual-licensed under `MIT OR Apache-2.0`, at your option.

---
**Paweł Lenartowicz** · [Freestyler Scientist](https://freestylerscientist.pl) · [GitHub](https://github.com/pawlenartowicz/) · [ORCID](https://orcid.org/0000-0002-6906-7217)

[`rand_chacha`]: https://crates.io/crates/rand_chacha
