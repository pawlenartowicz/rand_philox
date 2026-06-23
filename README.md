# rand_philox

[![crates.io](https://img.shields.io/crates/v/rand_philox.svg)](https://crates.io/crates/rand_philox)
[![docs.rs](https://img.shields.io/docsrs/rand_philox)](https://docs.rs/rand_philox)
[![CI](https://github.com/pawlenartowicz/rand_philox/actions/workflows/ci.yml/badge.svg)](https://github.com/pawlenartowicz/rand_philox/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/crates/l/rand_philox.svg)](#license)

**Counter-based Philox4x32-10 RNG with a reproducibility contract: bit-identical output across hosts and thread counts, KAT-pinned, `no_std`, no float in the core, and a `RngCore` drop-in for `rand_chacha`.**

The pitch is the contract, not the algorithm. "It's Philox" is already on the shelf; a maintained, KAT-frozen, `no_std`, no-float, `rand`-ecosystem drop-in positioned for the *reproducible-parallel-draws* audience is the slot `rand` leaves empty.

## When to use

- **Reproducible parallel draws** — every word is a pure function of `(key, 128-bit counter)` with no carried state, so a draw reproduces exactly regardless of how many draws ran before it, on how many threads, or on which host. Address any draw directly with `seek`.
- **`no_std` / WASM** — the block function, helpers, and `Philox` are `no_std`, zero-dependency, allocation-free, and have no float in the core (the one float helper, `u32_to_unit_f32`, is a leaf you opt into). The WASM-clean surface is CI-pinned on a wasm target.
- **A deterministic `rand` drop-in** — implements `rand_core`'s `TryRng` + `SeedableRng` (hence `Rng` / `RngCore`), so it slots in wherever `rand_chacha` does when you want reproducibility instead of cryptographic strength.

If you need a CSPRNG, use [`rand_chacha`]. If you need raw sequential throughput over cross-host reproducibility, a streaming PRNG (`rand_pcg`, `rand_xoshiro`) will be faster.

## Install

```toml
[dependencies]
rand_philox = "0.1"
```

Minimum supported Rust version: **1.85** (the `rand_core` 0.10 floor; the no-feature core compiles below that). The `rand_core` feature is on by default; disable default features for the pure `no_std`, zero-dependency core.

## Example

```rust
use rand_philox::Philox;

// Same (key, counter) reproduces the same stream, anywhere.
let mut a = Philox::from_u64_seed(42);
let mut b = Philox::from_u64_seed(42);
assert_eq!(a.next_u32(), b.next_u32());

// Jump to any counter position and read deterministically.
a.seek(1_000_000);
let x = a.next_u32();
b.seek(1_000_000);
assert_eq!(x, b.next_u32());
```

## Convention

- **Counter** — the `counter` passed to `Philox::new` / `seek` is a **block index**: Philox's native 128-bit counter, one increment per `philox4x32_10` call = four output words. `seek(c)` positions the stream at the start of block `c`.
- **`next_u64`** — two consecutive stream words, low word first (`lo | (hi << 32)`).
- **`u32_to_unit_f32`** — maps a word to an `f32` on the **open** interval (0, 1); 23-bit mantissa centred by +0.5 so it never returns 0 or 1.
- **`bounded(n)`** — unbiased integer in `[0, n)` via Lemire (no modulo bias); `n ≥ 1`, `n == 1` always 0.
- **Seed (`SeedableRng`)** — 24 bytes = `key[0] ‖ key[1] ‖ counter`, all little-endian; a transparent bijection onto `Philox::new`. For low-entropy `u64` seeds use `from_u64_seed` / `seed_from_u64`, which avalanche via SplitMix64.

The `philox4x32_10` output is validated against the three published Random123 known-answer vectors, frozen by the crate's KAT test.

## Design

| Property | Detail |
|---|---|
| Counter-based | draw value = pure function of `(key, counter)`; no carried state |
| Reproducible | bit-identical across hosts and thread counts; KAT-pinned output |
| `no_std` | core is `#![no_std]`, zero-dependency, allocation-free |
| No float in core | the only float touch, `u32_to_unit_f32`, is an opt-in leaf helper |
| No `unsafe` | `#![forbid(unsafe_code)]` |
| WASM-clean | build + test pinned on a wasm target in CI |

## Citing

> Salmon, J. K., Moraes, M. A., Dror, R. O., & Shaw, D. E. (2011). *Parallel random numbers: as easy as 1, 2, 3.* Proceedings of SC'11. (Philox; Random123.)

> Lemire, D. (2019). *Fast random integer generation in an interval.* ACM TOMACS 29(1). (The `bounded` method.)

## Credits / Third-party

The Philox constants and the known-answer vectors come from **Random123** (D. E. Shaw Research), BSD-3-Clause. The `philox4x32_10` implementation is an independent Rust port; see [`THIRD-PARTY-NOTICES`](THIRD-PARTY-NOTICES) for the full license.

This crate was **extracted to serve both [MCPower](https://github.com/pawlenartowicz/) and CommonStats** — the FastStats Rust statistical ecosystem — from one shared, battle-tested core, so a reproducibility-critical primitive has a single versioned home rather than living duplicated in each consumer. CommonStats is the second consumer; the wider ecosystem builds its domain-specific counter addressing on top of this generic primitive.

## License

Dual-licensed under `MIT OR Apache-2.0` at your option.

---
**Paweł Lenartowicz** — [Freestyler Scientist](https://freestylerscientist.pl) · [GitHub](https://github.com/pawlenartowicz/) · [ORCID](https://orcid.org/0000-0002-6906-7217)

[`rand_chacha`]: https://crates.io/crates/rand_chacha
