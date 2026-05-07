# Issue draft for godotengine/godot

**Title**: `RandomNumberGenerator` seeding produces correlated streams when instances are created in quick succession

---

### Tested versions

- Reproduced on Godot 4.6.2 stable, official build (commit `71f334935`).
- The same code path exists unchanged in `master` and has been unchanged in shape since 4.0.

### System information

- Godot v4.6.2.stable.official.71f334935 (macOS arm64, also reproduced on x86_64 in CI).
- The bug is platform-independent: the offending code is in shared core (`core/math/random_pcg.cpp`) and the symptom appears on every platform tested.

### Issue description

When two or more `RandomNumberGenerator` instances are constructed in quick succession (a pattern that occurs frequently in idiomatic GDScript: rerolling dice, drawing loot, shuffling a hand on each call), the resulting RNG instances frequently produce byte-identical output sequences. The defect is in the seeding strategy used by `RandomPCG::randomize()`, which is invoked automatically by the `RandomNumberGenerator` constructor.

The relevant code lives at `core/math/random_pcg.cpp`, lines 42 to 44:

```cpp
void RandomPCG::randomize() {
    seed(((uint64_t)OS::get_singleton()->get_unix_time() + OS::get_singleton()->get_ticks_usec()) * pcg.state + PCG_DEFAULT_INC_64);
}
```

The two timing inputs do not provide sufficient entropy:

1. `OS::get_unix_time()` returns a `double` of seconds since the epoch, but the cast `(uint64_t)` truncates the fractional part, leaving only second-level granularity. Two calls within the same wall-clock second contribute identical values.
2. `OS::get_ticks_usec()` has microsecond resolution, but `RandomNumberGenerator` construction completes well within a single microsecond on modern hardware, so back-to-back constructions in the same frame routinely read the same value.

When both inputs collide, the entire seed expression becomes deterministic. PCG itself is a fine generator and the bounded-rand path uses correct rejection sampling; the failure is purely upstream, in the seed source.

### Empirical evidence

A reproduction in pure GDScript on Godot 4.6.2, 5000 trials, each trial constructing a fresh `RandomNumberGenerator` and rolling 5 six-sided dice:

| Variant                                                  | Identical consecutive 5d6 sequences (out of 4999) | Expected |
| -------------------------------------------------------- | ------------------------------------------------: | -------: |
| `RandomNumberGenerator.new()` per trial                  |                                              2168 |    ~0.64 |
| `RandomNumberGenerator` instantiated once and reused     |                                                 1 |    ~0.64 |

The fresh-per-trial case shows roughly 3,300 times the expected collision rate, well outside any plausible statistical fluctuation. A pure C++ reproduction of the same code path (no GDScript per-iteration overhead between constructions) raises the rate to approximately 96 percent.

A direct probe of the entropy source confirms the cause: 5000 successive reads of `(uint64_t)get_unix_time() + get_ticks_usec()` produced only 143 distinct values, with up to 37 calls sharing a single value.

### Steps to reproduce

Drop the following script onto a `Node` in any Godot 4 project and run:

```gdscript
extends Node

const TRIALS := 5000
const DICE := 5

func _ready() -> void:
    var rolls: Array[PackedInt32Array] = []
    for t in TRIALS:
        var rng := RandomNumberGenerator.new()
        var r := PackedInt32Array(); r.resize(DICE)
        for d in DICE:
            r[d] = rng.randi_range(1, 6)
        rolls.append(r)

    var ties := 0
    for t in range(1, TRIALS):
        if rolls[t] == rolls[t - 1]:
            ties += 1

    print("Identical consecutive 5d6 sequences: %d / %d" % [ties, TRIALS - 1])
    print("Expected for a fair RNG: ~%.2f" % (float(TRIALS - 1) / pow(6.0, DICE)))
    get_tree().quit()
```

Expected output (fair RNG): `~0.64` ties.
Observed output: ~2000 ties on Godot 4.6.2, varying with hardware speed.

### Why this is filed despite #48087 being closed

This bug was previously reported in issue [#48087](https://github.com/godotengine/godot/issues/48087) (April 2021, Godot 3.3) under the title *"randomize() often producing same seed when called in `_ready()`"*. The reporter identified the precise root cause and quoted the relevant line of `random_pcg.cpp`. The issue was closed as completed.

The change that landed at that time added `unix_time` to the formula. As shown above, the cast `(uint64_t)OS::get_unix_time()` discards the fractional seconds, so on modern hardware the contribution is a constant within any given second. The amendment did not resolve the underlying problem; calls within the same second still collapse to identical seeds, which on a 60 fps update loop covers up to 60 successive constructions.

A second relevant historical issue is [#66989](https://github.com/godotengine/godot/issues/66989) (October 2022, Godot 4.0 beta), titled *"RandomNumberGenerator is constructed with a fixed seed, but documentation states it is randomized"*. The fix for #66989 made `RandomNumberGenerator()`'s constructor automatically call `randbase.randomize()` (visible at `core/math/random_number_generator.h:61`). That change correctly aligned the constructor with the documented behavior, but it also routed every new `RandomNumberGenerator` instance through the seeding formula from #48087. Where users previously had to opt in to the buggy seeder by calling `randomize()` explicitly, every fresh instance now goes through it automatically. The user-visible impact of the unresolved #48087 bug therefore expanded from "explicit `randomize()` callers" to "every `RandomNumberGenerator.new()` call site", which in practice is most game code.

The combination of the two fixes is what makes the present-day symptom severe. Each fix individually was reasonable in isolation, but together they leave a degenerate seed source applied automatically to every fresh `RandomNumberGenerator`.

### Documentation expectations

The class reference for [`RandomNumberGenerator`](https://docs.godotengine.org/en/stable/classes/class_randomnumbergenerator.html) states that `seed`'s default value "is pseudo-random, and changes when calling `randomize()`". The [random number generation tutorial](https://docs.godotengine.org/en/stable/tutorials/math/random_number_generation.html) describes instances as having their own independent seed and state, and frames the design as "creating multiple instances, each with their own seed and state". Users reasonably interpret this as a guarantee that each new instance is independent of others, which is the contract the current implementation does not honor when instances are created in succession.

Neither the class reference nor the tutorial mentions any caveat about instance lifetime or warns against creating new RNGs in close succession. Users who follow the documentation literally will encounter the correlated streams reported here.

### Secondary observation: the global `Math::default_rand`

While investigating this bug, a related concern surfaced. The global `randi()`, `randf()`, and friends route through `Math::default_rand`, declared at `core/math/math_funcs.cpp:36`. This singleton is initialized with the hardcoded `RandomPCG::DEFAULT_SEED` constant and is not automatically randomized at engine startup. The same game code therefore produces deterministic output across launches until `randomize()` is called explicitly. This is consistent with documented behavior in older Godot versions, but it is the opposite of what `RandomNumberGenerator` instances do post-#66989, and the asymmetry is surprising for users who did not read the implementation. This is mentioned as context only, and not as part of the present issue's scope.

### Expected behavior

Two `RandomNumberGenerator` instances created in succession, with no explicit seed, should be statistically independent. Concretely, the rate of byte-identical consecutive output sequences in the reproduction script above should match the theoretical expectation for a uniform RNG (approximately `(N - 1) / 6^k` for `k` six-sided dice). Any seed source that achieves this will resolve the issue.

### Reproduction project

A self-contained reproduction is available at [https://github.com/atelico/better-godot-rng/tree/main/tests/rng_experiment](https://github.com/atelico/better-godot-rng/tree/main/tests/rng_experiment). It also includes a comparison against an alternative implementation that does not exhibit the bug, for sanity-checking the experimental setup.

### Related history

- [#48087](https://github.com/godotengine/godot/issues/48087): initial report of this exact bug against Godot 3.3, closed completed in 2021. The fix that landed addressed only one of the two failure modes (cross-second calls), not the dominant intra-second case.
- [#66989](https://github.com/godotengine/godot/issues/66989): made the `RandomNumberGenerator` constructor auto-randomize, which is the change that broadened the practical surface of #48087's still-unfixed bug to every default-constructed instance.
- [#27171](https://github.com/godotengine/godot/pull/27171): unrelated earlier seeding cleanup, switched to using `pcg32_srandom_r` correctly. Addressed the seeding pipeline downstream of the entropy source, not the source itself.
