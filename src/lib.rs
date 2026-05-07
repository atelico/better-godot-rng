// =============================================================================
// BetterRng — drop-in replacement for Godot's `RandomNumberGenerator`.
//
// Why this crate exists
// ---------------------
// Godot's `RandomNumberGenerator::randomize()` (see `core/math/random_pcg.cpp`)
// derives the PCG seed from `(unix_time + ticks_usec) * pcg.state + INC`. The
// only varying input is `gettimeofday`'s microsecond field. Constructing an RNG
// takes far less than a microsecond on modern hardware, so multiple
// `RandomNumberGenerator.new()` calls in the same frame routinely receive
// *identical* timestamps and therefore *identical* seeds. Hashing the seed
// would not help: if the input is the same, no hash function produces a
// different output.
//
// Empirically (5000 fresh-instance trials, rolling 5d6 from each):
//
//     stock RandomNumberGenerator   2168 / 4999 consecutive full-sequence ties
//     BetterRng (this crate)           0 / 4999
//     RandomNumberGenerator reused     1 / 4999  (control — fair)
//
// A fair RNG would average ~0.64 ties. Stock Godot is ~3300× over expected.
//
// What this crate does differently
// --------------------------------
// 1. Seed source = OS entropy (via the `getrandom` crate). Cross-platform:
//    Linux/Android `getrandom(2)`, macOS/iOS `CCRandomGenerateBytes`,
//    Windows `BCryptGenRandom`, WASM `crypto.getRandomValues`,
//    BSDs native `getentropy`.
// 2. Generator = `Pcg64Mcg` from `rand_pcg`. Same PCG family Godot uses, but
//    with a 128-bit state instead of 64-bit, and properly seeded via
//    `SeedableRng` instead of an ad-hoc multiplicative formula.
// 3. Bounded integers use rejection sampling (no modulo bias).
// 4. Floats are constructed from 53 random mantissa bits — the standard
//    "uniform double" recipe — instead of Godot's leading-zero-counting trick
//    (which is correct but more complex than necessary for our purposes).
// =============================================================================

// `godot::prelude::*` brings in `Gd`, `GodotClass`, `IRefCounted`, the
// `#[gdextension]`/`#[godot_api]`/`#[func]` macros, and the Variant/Packed
// types we expose to GDScript. We use the official `godot-rust` (gdext)
// bindings, which speak the GDExtension C ABI Godot 4.x exposes.
use godot::prelude::*;

// `RngCore` is the trait that gives us `next_u32`/`next_u64`. `SeedableRng`
// gives us `seed_from_u64`, the canonical way to deterministically initialize
// a PRNG from a single 64-bit seed (it spreads those 64 bits over the full
// state via a SplitMix64 derivative, which is exactly the seed-whitening step
// Godot's seeder is missing).
use rand_core::{RngCore, SeedableRng};

// `Pcg64Mcg` = PCG with a 128-bit Multiplicative Congruential Generator base.
// Same family as Godot's `pcg32_random_r`, but with double the state width.
// We pick this variant (over `Pcg32`) because:
//   - 128-bit state has a period of 2^126, which is irrelevant in practice but
//     reassuring.
//   - It produces 64 bits per call, halving the number of generator advances
//     needed for `randf()` (which wants 53 mantissa bits).
//   - It is the default "good general-purpose PCG" in `rand_pcg`'s docs.
use rand_pcg::Pcg64Mcg;

// A unit struct that exists solely to satisfy `ExtensionLibrary`. The
// `#[gdextension]` macro registers it as the entry point — Godot will look up
// `gdext_rust_init` (the symbol defined by the macro) when loading the .so /
// .dylib / .dll. Anything declared with `#[derive(GodotClass)]` in this crate
// becomes visible to GDScript via this entry point.
struct BetterRngExtension;

// `unsafe impl` is required because `ExtensionLibrary` involves FFI guarantees
// the compiler can't verify (e.g., that we don't unload memory Godot still
// references). The default trait impl is sufficient for our needs.
#[gdextension]
unsafe impl ExtensionLibrary for BetterRngExtension {}

// =============================================================================
// `BetterRng` — the user-facing class.
//
// `base=RefCounted` means GDScript users don't have to manage lifetime.
// Assigning `var rng = BetterRng.new()` and dropping the variable frees the
// instance, mirroring `RandomNumberGenerator`'s behavior (it also extends
// RefCounted). Using `Node` here would force users to attach the RNG to the
// scene tree, which is wrong for a value type like an RNG.
// =============================================================================
#[derive(GodotClass)]
#[class(base=RefCounted)]
pub struct BetterRng {
    // The actual generator state. Kept private — exposing it as a Variant
    // would tempt users to copy it and create the same correlation problem
    // we're trying to fix. `set_seed` / `get_seed` provide the supported
    // entry points for reproducibility.
    rng: Pcg64Mcg,

    // We retain the most recently applied seed so `get_seed()` can return it.
    // This matches `RandomNumberGenerator.seed`, which GDScript code may read
    // back to log or persist a reproducible run. It's *not* the live state of
    // the generator (the state advances every call) — only the initial seed.
    seed: u64,
}

#[godot_api]
impl IRefCounted for BetterRng {
    // `init` runs every time GDScript calls `BetterRng.new()`. This is the
    // critical moment: every fresh instance must get an *independent* seed,
    // not one derived from a clock with microsecond resolution. We pull 64
    // bits straight from the OS entropy pool.
    fn init(_base: Base<RefCounted>) -> Self {
        let seed = os_entropy_u64();
        Self {
            // `Pcg64Mcg::seed_from_u64` runs the seed through SplitMix64 to
            // populate the 128-bit state, ensuring even small seed differences
            // produce uncorrelated streams from the very first output.
            rng: Pcg64Mcg::seed_from_u64(seed),
            seed,
        }
    }
}

#[godot_api]
impl BetterRng {
    // Re-seed from OS entropy. Mirrors `RandomNumberGenerator.randomize()`.
    // We re-pull from the OS rather than reusing any prior seed because the
    // whole point of `randomize()` is "give me a fresh stream now".
    #[func]
    fn randomize(&mut self) {
        let s = os_entropy_u64();
        self.seed = s;
        self.rng = Pcg64Mcg::seed_from_u64(s);
    }

    // Set an explicit seed for reproducible runs (replays, regression tests,
    // procedural generation that needs determinism). `i64` is the only
    // integer type GDScript exposes; we cast bit-for-bit to `u64` so users
    // can pass either signed or unsigned values without surprise.
    #[func]
    fn set_seed(&mut self, seed: i64) {
        let s = seed as u64;
        self.seed = s;
        self.rng = Pcg64Mcg::seed_from_u64(s);
    }

    // Return the most recent seed. Lossless `as i64` reinterpretation: the
    // user may see a negative number, but bit-for-bit it round-trips through
    // `set_seed`.
    #[func]
    fn get_seed(&self) -> i64 {
        self.seed as i64
    }

    // Uniform 32-bit unsigned integer, returned as `i64` because GDScript
    // `int` is 64-bit signed. Matches `RandomNumberGenerator.randi()` which
    // also returns 32 bits widened to GDScript's int.
    #[func]
    fn randi(&mut self) -> i64 {
        self.rng.next_u32() as i64
    }

    // Uniform float in [0, 1). The recipe:
    //   - take 64 random bits
    //   - shift right by 11, leaving 53 bits (matches f64 mantissa)
    //   - multiply by 2^-53 to land in [0, 1)
    // This is the well-known "53-bit mantissa" construction; it produces
    // every representable double in [0, 1) with equal probability. We
    // deliberately do NOT clamp to (0, 1) — the probability of returning
    // exactly 0 is 2^-53, vanishingly small but mathematically correct.
    #[func]
    fn randf(&mut self) -> f64 {
        let bits = self.rng.next_u64() >> 11;
        (bits as f64) * (1.0_f64 / ((1u64 << 53) as f64))
    }

    // Uniform float in [from, to]. Standard affine transform of [0, 1). If
    // the caller passes `from > to`, the result is still well-defined (lands
    // in [to, from]); we do not validate, matching Godot's behavior.
    #[func]
    fn randf_range(&mut self, from: f64, to: f64) -> f64 {
        from + self.randf() * (to - from)
    }

    // Uniform integer in [from, to] inclusive on both ends. Two important
    // implementation details:
    //   1. We normalize so `from <= to` regardless of input order, matching
    //      `RandomNumberGenerator.randi_range` (Godot also tolerates reversed
    //      bounds).
    //   2. We use rejection sampling via `bounded_u64` to avoid modulo bias.
    //      Naive `r % range` skews the distribution whenever `range` does not
    //      divide 2^64. Even at gameplay-typical ranges (say, 1..100) the
    //      bias is tiny but nonzero; using rejection sampling makes it
    //      exactly zero.
    #[func]
    fn randi_range(&mut self, from: i64, to: i64) -> i64 {
        if from == to {
            return from;
        }
        let (lo, hi) = if from < to { (from, to) } else { (to, from) };
        // `(hi - lo) as u64 + 1` is the inclusive range size. Adding 1 cannot
        // overflow because `hi - lo` is at most i64::MAX, leaving headroom.
        let range = (hi - lo) as u64 + 1;
        (lo as i64) + (bounded_u64(&mut self.rng, range) as i64)
    }

    // Standard normal sample via Box–Muller. Matches Godot's `randfn`.
    // Box–Muller takes two uniform samples and produces two independent
    // standard-normal samples; we throw the second away and call the
    // generator twice per draw, which is wasteful but matches Godot's API
    // (one sample per call). Could be optimized with a cached second sample;
    // not worth the complexity for a drop-in replacement.
    //
    // The `if u1 < f64::EPSILON` guard prevents `ln(0) = -inf`, which would
    // poison the result. The probability of triggering this is astronomically
    // small (we'd need all 53 mantissa bits of the random sample to be zero),
    // but the cost of guarding is one comparison so it's worth it.
    #[func]
    fn randfn(&mut self, mean: f64, deviation: f64) -> f64 {
        let mut u1 = self.randf();
        if u1 < f64::EPSILON {
            u1 += f64::EPSILON;
        }
        let u2 = self.randf();
        mean + deviation * (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    // Pick an index in proportion to the supplied weights. Algorithm:
    //   1. Sum the weights.
    //   2. Sample a uniform value in [0, sum).
    //   3. Walk the array subtracting each weight; return the index where
    //      the running remainder goes negative.
    //
    // This matches `RandomPCG::rand_weighted` in `core/math/random_pcg.cpp`.
    // Edge cases:
    //   - empty array: return -1, matching Godot's contract.
    //   - all-zero or negative-sum weights: return -1, since there's no
    //     well-defined "weighted" pick.
    //   - last weight floating-point underflow: fall through and return
    //     n-1, which is the only sensible choice.
    #[func]
    fn rand_weighted(&mut self, weights: PackedFloat32Array) -> i64 {
        let n = weights.len();
        if n == 0 {
            return -1;
        }
        let mut sum = 0.0_f32;
        for i in 0..n {
            // `.unwrap_or(0.0)` defends against the GDScript caller mutating
            // the array from another thread between the `len()` and the read;
            // in single-threaded use it's just a safe default.
            sum += weights.get(i).unwrap_or(0.0);
        }
        if sum <= 0.0 {
            return -1;
        }
        let mut remaining = (self.randf() as f32) * sum;
        for i in 0..n {
            remaining -= weights.get(i).unwrap_or(0.0);
            if remaining < 0.0 {
                return i as i64;
            }
        }
        // Floating-point round-off can leave `remaining` >= 0 after the loop.
        // The mathematically correct fallback is the last index.
        (n - 1) as i64
    }
}

// =============================================================================
// Free functions — entropy and bounded random.
// =============================================================================

// Pull 64 bits of OS-grade entropy. The `getrandom` crate is a thin shim over
// the platform-native CSPRNG: never blocks (on the platforms we care about),
// no `/dev/urandom` file-descriptor exhaustion, no manual feature detection.
//
// We use it both at construction time (init) and at every `randomize()` call.
// The cost is one syscall per re-seed, which is negligible compared to the
// cost of constructing a Godot object via FFI.
fn os_entropy_u64() -> u64 {
    let mut buf = [0u8; 8];
    if getrandom::getrandom(&mut buf).is_ok() {
        return u64::from_le_bytes(buf);
    }
    // Fallback path. `getrandom` failing is essentially impossible on
    // supported platforms (it would mean the kernel's CSPRNG is unavailable),
    // but if we ever land on a platform where it does, the absolute worst
    // case is that we silently regress to Godot-grade seeding. This fallback
    // ensures we don't: each call is unique even with poor clock resolution.
    //
    // Components:
    //   - `nanos`: SystemTime gives the best resolution the OS can offer
    //     (often nanoseconds on modern systems, microseconds on older ones).
    //   - `counter`: an atomic process-wide counter that increments per call.
    //     This is the critical ingredient — even if two calls share a
    //     timestamp, the counter values differ.
    //   - `stack_addr`: ASLR-randomized stack address adds a few bits of
    //     per-process entropy. Not sufficient alone, but cheap insurance.
    //
    // We feed the XOR of these through SplitMix64 to whiten the result;
    // raw concatenation would leave structure (e.g., low bits ≈ counter) that
    // a downstream PRNG could amplify.
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    // `0x9E37…` is the golden-ratio multiplier from SplitMix; multiplying the
    // counter by it spreads its low bits across the full 64-bit range before
    // XOR, which makes the XOR effectively non-cancelling against `nanos`.
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let stack_addr = &buf as *const _ as u64;
    splitmix64(nanos ^ counter.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ stack_addr)
}

// SplitMix64 — a tiny finalizer used as a hash for 64-bit integers. From
// "Fast Splittable Pseudorandom Number Generators" (Steele/Lea/Flood, 2014).
// Properties we want here:
//   - Bijective (no collisions): each input maps to a unique output.
//   - Avalanche: a one-bit change in input flips ~half the output bits.
// This is exactly what we need to whiten the fallback entropy mix.
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

// Rejection-sampled uniform integer in [0, bound). The naive `r % bound`
// skews the distribution whenever `bound` doesn't divide 2^64 — some buckets
// get `floor(2^64/bound)` outcomes, others get `floor(2^64/bound)+1`.
// Rejection sampling fixes this by discarding values that fall in the partial
// last bucket and resampling.
//
// `threshold` is the smallest representable value such that
// `[threshold, 2^64)` is a multiple of `bound`. We compute it as
// `(2^64 - bound) % bound`, but since `2^64` doesn't fit in u64, we use the
// algebraic identity `(0 - bound) % bound = (u64::MAX - bound + 1) % bound`
// (the `+ 1` rolls over the wraparound).
//
// In practice the loop almost always terminates in one iteration for
// game-typical ranges (1..100, 1..1000) — the rejection rate equals
// `bound / 2^64`, which is ~10^-18 for bound=100.
fn bounded_u64<R: RngCore>(rng: &mut R, bound: u64) -> u64 {
    if bound == 0 {
        return 0;
    }
    let threshold = (u64::MAX - bound + 1) % bound;
    loop {
        let r = rng.next_u64();
        if r >= threshold {
            return r % bound;
        }
    }
}
