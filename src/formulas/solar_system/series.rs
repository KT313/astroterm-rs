//! The VSOP87E series sums. Every term's sine and cosine come from a branch-free polynomial instead of libm, so the
//! compiler computes four terms side by side; the kernel is compiled twice, for AVX2+FMA when the CPU has them and
//! for the baseline otherwise. Both copies give the same bits: nothing calls `mul_add`, and Rust never fuses a
//! multiply and an add on its own. The terms are decoded once from the embedded bytes into separate arrays.
use std::f64::consts::FRAC_2_PI;
use std::sync::LazyLock;
use crate::formulas::{BodyId, SeriesTerms};
use crate::formulas::constants::{COS_POLYNOMIAL, HALF_PI_SPLIT, ROUNDING_MAGIC, SIN_POLYNOMIAL, VSOP87E_TERMS};

/// The decoded terms of all bodies, in the embedded order. A constant table of the embedded data, built on first
/// use; nothing invalidates it.
static SERIES_TERMS: LazyLock<SeriesTerms> = LazyLock::new(decode_series_terms);

pub(super) fn series_terms() -> &'static SeriesTerms {
    &SERIES_TERMS
}

/// Read the terms of every body: after the magic, each body's 18 term counts are followed by its terms.
fn decode_series_terms() -> SeriesTerms {
    let total = (VSOP87E_TERMS.len() - 8 - BodyId::PLANETS.len() * 18 * 4) / 24;
    let mut terms = SeriesTerms { amplitudes: Vec::with_capacity(total), phases: Vec::with_capacity(total), frequencies: Vec::with_capacity(total) };
    let mut offset = 8;
    for _ in BodyId::PLANETS {
        let count: usize = (0..18).map(|index| read_u32(&VSOP87E_TERMS[offset..], index)).sum();
        offset += 18 * 4;
        for term in VSOP87E_TERMS[offset..offset + 24 * count].chunks_exact(24) {
            terms.amplitudes.push(read_f64(term, 0));
            terms.phases.push(read_f64(term, 1));
            terms.frequencies.push(read_f64(term, 2));
        }
        offset += 24 * count;
    }
    assert_eq!(offset, VSOP87E_TERMS.len(), "the embedded series end after Neptune");
    terms
}

pub(super) fn read_u32(bytes: &[u8], index: usize) -> usize {
    u32::from_le_bytes(bytes[4 * index..4 * index + 4].try_into().unwrap()) as usize
}

pub(super) fn read_f64(bytes: &[u8], index: usize) -> f64 {
    f64::from_le_bytes(bytes[8 * index..8 * index + 8].try_into().unwrap())
}

/// `[Σ a·cos(b + c·t), −Σ a·c·sin(b + c·t), −Σ a·c²·cos(b + c·t)]` over one series: its value and its first two
/// derivatives per millennium. Chooses the AVX2 copy of the kernel when the CPU supports it; both give the same bits.
pub(super) fn sum_series(amplitudes: &[f64], phases: &[f64], frequencies: &[f64], t: f64) -> [f64; 3] {
    #[cfg(target_arch = "x86_64")]
    if std::is_x86_feature_detected!("avx2") && std::is_x86_feature_detected!("fma") {
        // SAFETY: the CPU supports AVX2 and FMA, detected just above (Rust caches the detection).
        return unsafe { sum_series_avx2(amplitudes, phases, frequencies, t) };
    }
    sum_series_portable(amplitudes, phases, frequencies, t)
}

/// The kernel compiled for the baseline (SSE2 on x86-64).
fn sum_series_portable(amplitudes: &[f64], phases: &[f64], frequencies: &[f64], t: f64) -> [f64; 3] {
    sum_series_lanes(amplitudes, phases, frequencies, t)
}

/// The kernel compiled for AVX2 and FMA.
///
/// # Safety
/// The caller must have detected AVX2 and FMA.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn sum_series_avx2(amplitudes: &[f64], phases: &[f64], frequencies: &[f64], t: f64) -> [f64; 3] {
    sum_series_lanes(amplitudes, phases, frequencies, t)
}

/// Sum the series in four lanes: term `i` always goes to lane `i % 4`, and the lanes combine pairwise at the end.
#[inline(always)]
fn sum_series_lanes(amplitudes: &[f64], phases: &[f64], frequencies: &[f64], t: f64) -> [f64; 3] {
    assert!(amplitudes.len() == phases.len() && phases.len() == frequencies.len(), "one amplitude, phase and frequency per term");
    let mut sums = [[0.0; 4]; 3];                                                         // value, slope and curvature, per lane

    // Whole groups of four terms, one per lane.
    let groups = amplitudes.chunks_exact(4).zip(phases.chunks_exact(4)).zip(frequencies.chunks_exact(4));
    for ((amplitudes, phases), frequencies) in groups {
        for lane in 0..4 {
            add_term(&mut sums, lane, amplitudes[lane], phases[lane], frequencies[lane], t);
        }
    }

    // The last zero to three terms, in lane order.
    let tail = amplitudes.len() / 4 * 4;
    for (lane, index) in (tail..amplitudes.len()).enumerate() {
        add_term(&mut sums, lane, amplitudes[index], phases[index], frequencies[index], t);
    }
    sums.map(|lanes| (lanes[0] + lanes[1]) + (lanes[2] + lanes[3]))
}

/// Add one term `a·cos(b + c·t)` and its two derivatives to `lane`.
#[inline(always)]
fn add_term(sums: &mut [[f64; 4]; 3], lane: usize, a: f64, b: f64, c: f64, t: f64) {
    let (sine, cosine) = approximate_sin_cos(b + c * t);
    sums[0][lane] += a * cosine;
    sums[1][lane] -= a * c * sine;
    sums[2][lane] -= a * c * c * cosine;
}

/// sin x and cos x within 4.5e-16 of libm for |x| ≤ 4e6, without branches: reduce by the nearest multiple of
/// π/2, evaluate the fdlibm kernels on [−π/4, π/4], and pick the quadrant by multiplying with exact 0/1 and ±1.
#[inline(always)]
fn approximate_sin_cos(x: f64) -> (f64, f64) {
    // The nearest quadrant count, and the remainder from π/2 in three parts whose products with q are exact.
    let [p1, p2, p3] = HALF_PI_SPLIT;
    let q = (x * FRAC_2_PI + ROUNDING_MAGIC) - ROUNDING_MAGIC;
    let r = ((x - q * p1) - q * p2) - q * p3;

    // The two kernels on the remainder.
    let [s1, s2, s3, s4, s5, s6] = SIN_POLYNOMIAL;
    let [c1, c2, c3, c4, c5, c6] = COS_POLYNOMIAL;
    let z = r * r;
    let s = r + r * z * (s1 + z * (s2 + z * (s3 + z * (s4 + z * (s5 + z * s6)))));
    let c = (1.0 - 0.5 * z) + z * z * (c1 + z * (c2 + z * (c3 + z * (c4 + z * (c5 + z * c6)))));

    // Quadrants n = q mod 4: 0 → (s, c), 1 → (c, −s), 2 → (−s, −c), 3 → (−c, s).
    let n = q - 4.0 * ((q * 0.25 - 0.375 + ROUNDING_MAGIC) - ROUNDING_MAGIC);
    let high = (n * 0.5 - 0.25 + ROUNDING_MAGIC) - ROUNDING_MAGIC;                     // floor(n / 2): 0 or 1
    let odd = n - 2.0 * high;                                                           // n mod 2: 0 or 1
    let cos_negative = odd + high - 2.0 * odd * high;                                   // 1 for n = 1 and 2
    let sine = (s * (1.0 - odd) + c * odd) * (1.0 - 2.0 * high);
    let cosine = (c * (1.0 - odd) + s * odd) * (1.0 - 2.0 * cos_negative);
    (sine, cosine)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};
    use crate::formulas::constants::{COMPUTATIONAL_INTERVAL, J2000, VSOP87_MILLENNIUM_DAYS};

    /// The epochs the series are compared at: both ends of the computational interval and four between.
    pub(in crate::formulas::solar_system) fn series_epochs() -> [f64; 6] {
        let millennium = VSOP87_MILLENNIUM_DAYS;
        [COMPUTATIONAL_INTERVAL.start_tt, J2000 - 3.0 * millennium, J2000, 2461041.5, J2000 + 2.5 * millennium, COMPUTATIONAL_INTERVAL.end_tt.next_down()]
    }

    /// Every body's byte offset of its counts in `VSOP87E_TERMS` and the index of its first decoded term.
    fn body_starts() -> Vec<(usize, usize)> {
        let (mut offset, mut first, mut starts) = (8, 0, Vec::new());
        for _ in BodyId::PLANETS {
            starts.push((offset, first));
            let count: usize = (0..18).map(|index| read_u32(&VSOP87E_TERMS[offset..], index)).sum();
            offset += 18 * 4 + 24 * count;
            first += count;
        }
        starts
    }

    /// Every series of every body as (amplitudes, phases, frequencies), in the embedded order.
    fn every_series() -> Vec<[&'static [f64]; 3]> {
        let terms = series_terms();
        let mut series = Vec::new();
        for (offset, mut first) in body_starts() {
            for index in 0..18 {
                let range = first..first + read_u32(&VSOP87E_TERMS[offset..], index);
                series.push([&terms.amplitudes[range.clone()], &terms.phases[range.clone()], &terms.frequencies[range.clone()]]);
                first = range.end;
            }
        }
        series
    }

    #[test]
    fn half_pi_split_constants_are_exact_and_sum_to_half_pi() {
        assert_eq!(HALF_PI_SPLIT.map(f64::to_bits), [0x3FF921FB54800000, 0xBE0DE973DC800000, 0xBC19D9CCEBA3F91F]);
        for part in &HALF_PI_SPLIT[..2] {
            assert_eq!(part.to_bits() & ((1 << 23) - 1), 0, "{part}: more than 30 significant bits");
        }
        assert_eq!(HALF_PI_SPLIT[0] + HALF_PI_SPLIT[1] + HALF_PI_SPLIT[2], FRAC_PI_2);
    }

    #[test]
    fn largest_series_argument_stays_inside_the_tested_range() {
        // |b + c·t| over the computational interval bounds the arguments; the split needs |q| < 2^23.
        let t_max = [COMPUTATIONAL_INTERVAL.start_tt, COMPUTATIONAL_INTERVAL.end_tt].map(|tt| ((tt - J2000) / VSOP87_MILLENNIUM_DAYS).abs()).into_iter().fold(0.0, f64::max);
        let terms = series_terms();
        let largest = terms.phases.iter().zip(&terms.frequencies).map(|(b, c)| b.abs() + c.abs() * t_max).fold(0.0, f64::max);
        assert!(largest < 4.0e6, "{largest}");
        assert!((largest * FRAC_2_PI).round() < f64::from(1 << 22), "{largest}");
    }

    #[test]
    fn polynomial_sin_cos_matches_libm_over_the_whole_argument_range() {
        // Random arguments over the whole range and near zero, multiples of π/4, and points around multiples of π/2.
        let mut seed = 17_u32;
        let mut next_unit = || {
            let mut draw = || { seed = seed.wrapping_mul(1664525).wrapping_add(1013904223); f64::from(seed >> 8) };
            (draw() * 16777216.0 + draw()) / 281474976710655.0                              // 48 random bits in [0, 1]
        };
        let mut arguments: Vec<f64> = (0..200_000).map(|_| -4.0e6 + 8.0e6 * next_unit()).collect();
        arguments.extend((0..100_000).map(|_| -10.0 + 20.0 * next_unit()));
        arguments.extend((-2000..2000).map(|k| f64::from(k) * FRAC_PI_4));
        for k in (-2_600_000..2_600_000).step_by(9973) {
            arguments.extend([-1e-9, 0.0, 1e-9].map(|offset| f64::from(k) * FRAC_PI_2 + offset));
        }
        arguments.extend([0.0, -0.0]);

        let mut worst = 0.0_f64;
        for x in arguments {
            let (sine, cosine) = approximate_sin_cos(x);
            let error = (sine - x.sin()).abs().max((cosine - x.cos()).abs());
            assert!(error <= 4.5e-16, "x = {x}: sin {sine} against {}, cos {cosine} against {}", x.sin(), x.cos());
            worst = worst.max(error);
        }
        eprintln!("largest difference from libm: {worst:e}");
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn avx2_and_portable_sums_are_bit_identical() {
        if !(std::is_x86_feature_detected!("avx2") && std::is_x86_feature_detected!("fma")) { return; }
        for tt in series_epochs() {
            let t = (tt - J2000) / VSOP87_MILLENNIUM_DAYS;
            for [amplitudes, phases, frequencies] in every_series() {
                let portable = sum_series_portable(amplitudes, phases, frequencies, t);
                // SAFETY: AVX2 and FMA were detected above.
                let wide = unsafe { sum_series_avx2(amplitudes, phases, frequencies, t) };
                assert_eq!(portable.map(f64::to_bits), wide.map(f64::to_bits), "{} terms at {tt}", amplitudes.len());
            }
        }
    }

    #[test]
    fn every_tail_length_is_summed_in_lane_order() {
        // Synthetic terms, summed by a plain loop with the same lane assignment and final combination.
        let terms: Vec<[f64; 3]> = (0..9).map(|index| { let i = f64::from(index); [0.3 + 0.1 * i, 1.0 + 0.7 * i, 100.0 * i - 250.0] }).collect();
        let t = 0.37;
        assert_eq!(sum_series(&[], &[], &[], t), [0.0; 3]);
        for length in 0..=9 {
            let column = |index: usize| terms[..length].iter().map(|term| term[index]).collect::<Vec<_>>();
            let actual = sum_series(&column(0), &column(1), &column(2), t);
            let mut lanes = [[0.0; 4]; 3];
            for (index, &[a, b, c]) in terms[..length].iter().enumerate() {
                let (sine, cosine) = approximate_sin_cos(b + c * t);
                lanes[0][index % 4] += a * cosine;
                lanes[1][index % 4] -= a * c * sine;
                lanes[2][index % 4] -= a * c * c * cosine;
            }
            let expected = lanes.map(|lane| (lane[0] + lane[1]) + (lane[2] + lane[3]));
            assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits), "{length} terms");
        }
    }

    #[test]
    fn decoded_terms_match_the_embedded_table() {
        let terms = series_terms();
        assert_eq!([terms.amplitudes.len(), terms.phases.len(), terms.frequencies.len()], [51_880; 3]);

        // The first and last term of every body, read straight from the embedded bytes.
        let starts = body_starts();
        for (body, &(offset, first)) in starts.iter().enumerate() {
            let count = starts.get(body + 1).map_or(terms.amplitudes.len(), |next| next.1) - first;
            for (index, term) in [(first, 0), (first + count - 1, count - 1)] {
                let bytes = &VSOP87E_TERMS[offset + 18 * 4 + 24 * term..][..24];
                let embedded = [read_f64(bytes, 0), read_f64(bytes, 1), read_f64(bytes, 2)];
                assert_eq!([terms.amplitudes[index], terms.phases[index], terms.frequencies[index]], embedded, "body {body}, term {term}");
            }
        }
        let earth = starts[BodyId::Earth as usize].1;                                     // the first term of the Earth's X, power 0
        assert_eq!([terms.amplitudes[earth], terms.phases[earth], terms.frequencies[earth]], [0.99982624851, 1.75348568475, 6283.07584999140]);
    }
}
