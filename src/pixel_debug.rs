//! Opt-in pixel arithmetic checks (`make build-debug`). No code from this module exists in ordinary builds.
//! Checks run before lossy casts/clamps; a failure identifies the operation, channel and calling source line.
//! Accumulated light may exceed one before tone mapping. Its range is reported separately, not rejected.
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};

const CHANNELS: [&str; 4] = ["R", "G", "B", "opacity"];
const FLOAT_TOLERANCE: f64 = 8.0 * f32::EPSILON as f64;                                 // allow final-bit noise, never adjust the actual values
static FRAME_SAMPLES: AtomicU64 = AtomicU64::new(0);
static ABOVE_ONE: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static MAXIMUM: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

/// Check one channel before a clamp or narrowing conversion. Infinity as the upper bound permits finite HDR
/// values, not NaN or infinity. Small floating-point overshoot is tolerated, but the caller's value is unchanged.
#[track_caller]
pub fn check_channel(operation: &str, channel: usize, value: f64, maximum: f64) {
    let tolerance = if maximum.is_finite() { FLOAT_TOLERANCE * maximum.max(1.0) } else { 0.0 };
    assert!(value.is_finite() && value >= 0.0 && value <= maximum + tolerance,
        "pixel-debug: {operation}: {}={value:?}, expected finite 0..={maximum}", CHANNELS[channel]);
}

/// Check all four channels and the premultiplied invariant RGB <= opacity. `maximum` is 1 for group tiles,
/// 255 for byte overlays, or infinity for accumulated light. This helper has no dependency on rendering types.
#[track_caller]
pub fn check_pixel(operation: &str, rgba: [f64; 4], maximum: f64) {
    for (channel, value) in rgba.into_iter().enumerate() { check_channel(operation, channel, value, maximum); }
    for (channel, value) in rgba[..3].iter().copied().enumerate() {
        let tolerance = FLOAT_TOLERANCE * rgba[3].max(1.0);
        assert!(value <= rgba[3] + tolerance,
            "pixel-debug: {operation}: {}={value:?} exceeds premultiplied opacity={:?}", CHANNELS[channel], rgba[3]);
    }
}

/// Validate a byte overlay produced by external drawing routines. Their internal intermediates are not visible;
/// our own text, Moon and final-frame conversions are checked before narrowing as well.
#[track_caller]
pub fn check_overlay(operation: &str, rgba: &[u8]) {
    assert_eq!(rgba.len() % 4, 0, "pixel-debug: {operation}: incomplete RGBA pixel");
    for pixel in rgba.chunks_exact(4) {
        check_pixel(operation, [pixel[0] as f64, pixel[1] as f64, pixel[2] as f64, pixel[3] as f64], 255.0);
    }
}

/// Observe each frame's star pixels once, before tone mapping. Counts include repeated frames, not unique stars.
pub fn record_accumulated(rgba: [f64; 4]) {
    FRAME_SAMPLES.fetch_add(1, Ordering::Relaxed);
    for (channel, value) in rgba.into_iter().enumerate() {
        if value > 1.0 { ABOVE_ONE[channel].fetch_add(1, Ordering::Relaxed); }
        MAXIMUM[channel].fetch_max(value.to_bits(), Ordering::Relaxed);                    // finite nonnegative floats have ordered bit patterns
    }
}

/// Print after terminal restoration. Values above one here are intentional light accumulation, not numeric
/// overflow; NaN, infinity, negatives and invalid bounded channels panic at the operation that produces them.
pub fn write_report(output: &mut impl Write) -> io::Result<()> {
    writeln!(output, "Pixel checks enabled: invalid channels stop at the offending operation.")?;
    writeln!(output, "Pre-tone-map samples: {} (includes repeated frames); values above 1 are permitted accumulated light.", FRAME_SAMPLES.load(Ordering::Relaxed))?;
    for (channel, name) in CHANNELS.iter().enumerate() {
        writeln!(output, "  {name}: maximum={:.6}; samples above 1={}", f64::from_bits(MAXIMUM[channel].load(Ordering::Relaxed)), ABOVE_ONE[channel].load(Ordering::Relaxed))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_accept_bounded_colors_and_finite_accumulated_light_without_changing_them() {
        check_pixel("unit", [0.1, 0.2, 0.3, 0.3], 1.0);
        check_pixel("HDR", [2.0, 3.0, 4.0, 5.0], f64::INFINITY);
        check_overlay("overlay", &[64, 32, 16, 128]);
        check_channel("roundoff", 0, 255.00001, 255.0);
    }

    #[test]
    fn checks_reject_each_invalid_channel_and_nonpremultiplied_colors() {
        for channel in 0..4 {
            for value in [-0.01, 1.01, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                let mut pixel = [0.0, 0.0, 0.0, 1.0];
                pixel[channel] = value;
                let failure = std::panic::catch_unwind(|| check_pixel("fixture", pixel, 1.0)).unwrap_err();
                let message = failure.downcast_ref::<String>().unwrap();
                assert!(message.contains("fixture") && message.contains(CHANNELS[channel]));
            }
        }
        assert!(std::panic::catch_unwind(|| check_pixel("bad premultiply", [0.5, 0.0, 0.0, 0.1], 1.0)).is_err());
        assert!(std::panic::catch_unwind(|| check_channel("before u8", 2, 256.0, 255.0)).is_err());
        assert!(std::panic::catch_unwind(|| check_pixel("float overflow", [f64::INFINITY; 4], f64::INFINITY)).is_err());
    }
}
