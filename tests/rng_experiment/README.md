# RNG fairness experiment

Reproduces the Godot RNG correlation bug and verifies that `BetterRng` does
not have it.

## Run

1. Build the extension once at the repo root: `cargo build --release`.
2. Copy the resulting library next to `addons/better_rng/better_rng.gdextension`:
   - macOS: `cp ../../target/release/libgodot_better_rng.dylib addons/better_rng/`
   - Linux: `cp ../../target/release/libgodot_better_rng.so addons/better_rng/`
   - Windows: `copy ..\..\target\release\godot_better_rng.dll addons\better_rng\`
3. Run headless from this directory:
   ```sh
   /path/to/Godot --headless --path .
   ```

You should see ~2000 consecutive 5d6 collisions for stock Godot vs. ~0 for
BetterRng, out of 4999 trials.
