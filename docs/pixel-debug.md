# Pixel arithmetic checks

Build and run the instrumented binary:

```sh
make build-debug
RUST_BACKTRACE=1 target/pixel-debug/astroterm -i Tokyo -Ccu -m --graphics-protocol kitty --dataset athyg --facing 334 -t 20
```

Or run it through Make:

```sh
make run BINARY=target/pixel-debug/astroterm ARGS='-i Tokyo -m'
```

This uses the `pixel-debug` Cargo feature and an optimized `pixel-debug` profile with integer overflow checks
and source-line information. The ordinary release binary is separate and has no pixel-check instrumentation.
The checks add substantial work; do not compare these timings with a normal build.

Checks surround star premultiplication/blending, group-image merging, texel addition/scaling, overlay output,
text and Moon channel conversion, final tone mapping and RGBA-to-RGB conversion. They detect NaN, infinity,
negative values, bounded channels outside their range, and premultiplied RGB exceeding opacity. RGB and opacity
are checked independently. All bounded float and integer intermediates owned by those conversion loops are
checked before clamping or narrowing to `u8`, which could otherwise hide a bad value. Small positive floating
overshoot (eight f32 epsilons relative to the bound) is tolerated; checks never change the rendered values.

An invalid value stops execution with an error such as:

```text
pixel-debug: blend_star source: opacity=1.100000023841858, expected finite 0..=1
```

The panic identifies the check's source file and line. `RUST_BACKTRACE=1` adds the calling stack. The existing
terminal panic hook restores the terminal before printing the error.

Accumulated warp light is different from a bounded opacity: adding several texels can intentionally produce
RGB or opacity above 1. Those values are accepted while finite and nonnegative. On normal exit, the diagnostic
report on stderr lists each pre-tone-map channel's maximum and the number of samples above 1. Counts include
repeated frames and are not unique star counts. Normal saturation is therefore visible without aborting on it.

The checks validate inputs/outputs around third-party drawing. They cannot inspect tiny-skia's internal
floating-point intermediates or reconstruct a value already lost inside a third-party clamp/cast. Integer
overflow checks apply to dependencies compiled under the `pixel-debug` profile too.

Regression checks:

```sh
cargo test --features pixel-debug --lib pixel_debug
cargo test --features pixel-debug --lib pixel_wrappers
```
