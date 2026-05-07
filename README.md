# BetterRng

**A drop-in replacement for Godot 4's `RandomNumberGenerator` that doesn't lie to you.**

Godot's stock RNG produces visibly correlated output when you call
`RandomNumberGenerator.new()` multiple times in quick succession — the exact
pattern most game code uses for "rerolls", "loot drops", "shuffle this deck",
and similar. This library is an actual fix.

---

## TL;DR

```gdscript
# before
var rng = RandomNumberGenerator.new()
var roll = rng.randi_range(1, 6)

# after
var rng = BetterRng.new()
var roll = rng.randi_range(1, 6)
```

Same API. Different — fair — distribution.

---

## The bug, in one paragraph

`core/math/random_pcg.cpp` derives the PCG seed from
`(unix_time + ticks_usec) * pcg.state + INC`. The only varying input is
`gettimeofday`'s microsecond field. **Constructing an RNG takes far less than
a microsecond on modern hardware**, so consecutive `RandomNumberGenerator.new()`
calls in the same frame routinely receive *identical* timestamps and therefore
*identical* seeds. Hashing the seed wouldn't help — if the input is identical,
no hash function produces different outputs. The fix has to happen at the
entropy source: pull from the OS CSPRNG (`getrandom`/`BCryptGenRandom`/
`getentropy`/`crypto.getRandomValues`) instead of the wall clock.

## The bug, in one experiment

Run inside actual Godot 4.6.2. Each "trial" creates a fresh RNG and rolls 5
six-sided dice. Then we count how often two consecutive trials produced the
**byte-identical** five-dice sequence:

| Variant                                       | Identical consecutive 5d6 (out of 4999) | Expected |
| --------------------------------------------- | --------------------------------------: | -------: |
| Stock `RandomNumberGenerator.new()` per trial |                                **2168** |    ~0.64 |
| `BetterRng.new()` per trial                   |                                   **0** |    ~0.64 |
| Stock RNG, single instance reused (control)   |                                       1 |    ~0.64 |

Stock Godot is **~3,300× over expected**. `BetterRng` is statistically
indistinguishable from the reused-RNG control — i.e., it actually behaves like
a fair RNG. Reproduce it yourself with the project under
[`tests/rng_experiment/`](tests/rng_experiment/).

## How `BetterRng` differs from stock

| Concern               | Stock Godot                                    | BetterRng                                                                                          |
| --------------------- | ---------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Seed source           | `(unix_time + ticks_usec) * pcg.state + INC`   | OS CSPRNG (`getrandom` crate: `getrandom(2)` / `BCryptGenRandom` / `CCRandomGenerateBytes` / etc.) |
| Seed whitening        | None                                           | `SeedableRng::seed_from_u64` (SplitMix64 expansion)                                                |
| Generator             | PCG32 (64-bit state)                           | PCG64-MCG (128-bit state, same family)                                                             |
| Bounded int sampling  | Rejection (correct)                            | Rejection (correct)                                                                                |
| Auto-seeded on `new()`| Yes, but from a clock collision-prone formula | Yes, from the OS CSPRNG                                                                            |

PCG itself is a fine generator; Godot's bug is purely in the seeding. We use
the same family with a wider state and a correct seed pipeline.

## API surface (matches `RandomNumberGenerator`)

| Method                                            | Returns | Notes                                              |
| ------------------------------------------------- | ------- | -------------------------------------------------- |
| `randomize()`                                     | void    | Re-seed from OS entropy.                           |
| `set_seed(seed: int)` / `get_seed() -> int`       | —       | Reproducible runs (replays, regression tests).     |
| `randi() -> int`                                  | 32-bit  | Uniform.                                           |
| `randf() -> float`                                | 53-bit  | Uniform in `[0, 1)`.                               |
| `randf_range(from, to) -> float`                  | 53-bit  | Uniform in `[from, to]`.                           |
| `randi_range(from, to) -> int`                    | 64-bit  | Inclusive, rejection-sampled (no modulo bias).     |
| `randfn(mean, deviation) -> float`                | 53-bit  | Normal distribution (Box–Muller).                  |
| `rand_weighted(weights: PackedFloat32Array) -> int` | int   | Weighted index. Empty / all-zero weights → -1.     |

---

## Install (using a release artifact)

The recommended path. Releases include prebuilt binaries for Linux x86_64 / aarch64,
macOS (universal), and Windows x86_64.

1. Go to the [Releases](../../releases) page and download the asset matching your
   target platforms (or the `all-platforms.zip` bundle).
2. Extract into your Godot project under `res://addons/better_rng/`. The
   directory should look like:
   ```
   addons/better_rng/
   ├── better_rng.gdextension
   ├── libgodot_better_rng.so          (Linux)
   ├── libgodot_better_rng.dylib       (macOS, universal)
   └── godot_better_rng.dll            (Windows)
   ```
3. Reopen the project in Godot. The editor should report
   `Initialize godot-rust ...` in the console.
4. `BetterRng` is now a registered class — use it from any GDScript.

If you exported the game, the same files need to ship next to your binary;
Godot's exporter handles this automatically when they live under `res://`.

---

## Build from source

Requirements:
- Rust 1.85 or newer (`rustup install stable`)
- A C linker for your target (Xcode CLT on macOS, `build-essential` on Linux,
  Visual Studio Build Tools on Windows)

```sh
cargo build --release
```

Output:
- Linux: `target/release/libgodot_better_rng.so`
- macOS: `target/release/libgodot_better_rng.dylib`
- Windows: `target/release/godot_better_rng.dll`

Copy that file plus `better_rng.gdextension` into
`<your_project>/addons/better_rng/` and you're done.

### Cross-compiling

#### Linux aarch64 from x86_64
```sh
rustup target add aarch64-unknown-linux-gnu
sudo apt install gcc-aarch64-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu
```

#### macOS universal (arm64 + x86_64)
```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin
lipo -create -output libgodot_better_rng.dylib \
    target/aarch64-apple-darwin/release/libgodot_better_rng.dylib \
    target/x86_64-apple-darwin/release/libgodot_better_rng.dylib
```

#### Windows x86_64 from Linux/macOS
Easiest path is to use the GitHub Actions workflow in
`.github/workflows/release.yml` rather than wrestling with MSVC toolchains
locally — push a tag and let CI handle it.

### Android / iOS / Web

Possible but not currently shipped. Each requires a separate target triple,
NDK / Xcode toolchain, and adjustments to the `.gdextension` file. PRs welcome.

---

## Releasing

Releases are produced by the GitHub Actions workflow at
`.github/workflows/release.yml`. To cut a release:

```sh
git tag v0.1.0
git push origin v0.1.0
```

CI then:
1. Builds release binaries for Linux x86_64, Linux aarch64, macOS universal,
   and Windows x86_64.
2. Bundles them under `addons/better_rng/` along with `better_rng.gdextension`.
3. Uploads each platform's bundle as a release asset, plus an
   `all-platforms.zip` containing every binary (recommended for projects that
   ship to multiple platforms).

---

## Verifying it works

```gdscript
extends Node

func _ready() -> void:
    var matches := 0
    var prev := -1
    for _i in 1000:
        var rng = BetterRng.new()
        var d = rng.randi_range(1, 6)
        if d == prev:
            matches += 1
        prev = d
    print("BetterRng consecutive matches: %d (expected ~166)" % matches)
    # Now do the same with RandomNumberGenerator and watch matches spike.
```

The full reproduction lives at [`tests/rng_experiment/`](tests/rng_experiment/).
Open it as a Godot project and run the main scene; output goes to the console.

## License

MIT. See [LICENSE](LICENSE).

## Acknowledgements

- The PCG family of generators by Melissa O'Neill (https://www.pcg-random.org/).
- The Rust [`rand_pcg`](https://crates.io/crates/rand_pcg) and
  [`getrandom`](https://crates.io/crates/getrandom) crates.
- The [godot-rust](https://github.com/godot-rust/gdext) project, without which
  none of this would be a one-evening fix.
