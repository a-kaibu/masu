//! Each conversion against an oracle that does not use `as`: the exact value of an integral
//! float is read back from its decimal text (Rust prints it exactly), so truncation and
//! nearest-even rounding are checked against the definitions, on random bit patterns and the
//! edges of every range.

use super::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// an integral (or infinite) float's exact value, saturated to i128
fn exact(t: f64) -> i128 {
    if t == f64::INFINITY {
        return i128::MAX;
    }
    if t == f64::NEG_INFINITY {
        return i128::MIN;
    }
    format!("{t:.0}").parse::<i128>().unwrap_or(if t > 0.0 { i128::MAX } else { i128::MIN })
}

/// the definition: NaN 0, else toward zero, clamped to [lo, hi]
fn want_trunc(x: f64, lo: i128, hi: i128) -> i128 {
    if x.is_nan() { 0 } else { exact(x.trunc()).clamp(lo, hi) }
}

fn floats(seed: u64) -> Vec<f64> {
    let mut r = Rng(seed);
    let mut v: Vec<f64> = vec![0.0, -0.0, 0.5, -0.5, 1.0, -1.0, 0.999_999, -0.999_999, f64::INFINITY, f64::NEG_INFINITY, f64::NAN, f64::MAX, f64::MIN, f64::MIN_POSITIVE];
    for e in [7, 8, 15, 16, 31, 32, 53, 63, 64] {
        let p = 2f64.powi(e);
        v.extend([p, -p, p - 1.0, -p + 1.0, p + 1.0, -p - 1.0, p - 0.5, -p - 0.5, p * (1.0 - f64::EPSILON), -p * (1.0 + f64::EPSILON)]);
    }
    for _ in 0..200_000 {
        v.push(f64::from_bits(r.next()));
        // (mostly in the integer ranges)
        let m = f64::from_bits(r.next() >> 12 | 0x3ff0_0000_0000_0000) - 1.0;
        let e = i32::try_from(r.next() % 70).unwrap_or(0) - 3;
        v.push(if r.next() & 1 == 0 { m * 2f64.powi(e) } else { -m * 2f64.powi(e) });
    }
    v
}

#[test]
fn trunc_is_toward_zero_saturating_nan_zero() {
    for x in floats(1) {
        assert_eq!(i128::from(trunc_i8(x)), want_trunc(x, i128::from(i8::MIN), i128::from(i8::MAX)), "{x}");
        assert_eq!(i128::from(trunc_u8(x)), want_trunc(x, 0, i128::from(u8::MAX)), "{x}");
        assert_eq!(i128::from(trunc_i16(x)), want_trunc(x, i128::from(i16::MIN), i128::from(i16::MAX)), "{x}");
        assert_eq!(i128::from(trunc_u16(x)), want_trunc(x, 0, i128::from(u16::MAX)), "{x}");
        assert_eq!(i128::from(trunc_i32(x)), want_trunc(x, i128::from(i32::MIN), i128::from(i32::MAX)), "{x}");
        assert_eq!(i128::from(trunc_u32(x)), want_trunc(x, 0, i128::from(u32::MAX)), "{x}");
        assert_eq!(i128::from(trunc_i64(x)), want_trunc(x, i128::from(i64::MIN), i128::from(i64::MAX)), "{x}");
        assert_eq!(i128::from(trunc_u64(x)), want_trunc(x, 0, i128::from(u64::MAX)), "{x}");
        let us_max = i128::try_from(usize::MAX).unwrap_or(i128::MAX);
        assert_eq!(i128::try_from(trunc_usize(x)).unwrap_or(i128::MAX), want_trunc(x, 0, us_max), "{x}");
        // (f32 sources: the same definition on the f32 value)
        let y = to_f32(x);
        let yd = f64::from(y);
        assert_eq!(i128::from(trunc_i32(y)), want_trunc(yd, i128::from(i32::MIN), i128::from(i32::MAX)), "{y}");
        assert_eq!(i128::from(trunc_u32(y)), want_trunc(yd, 0, i128::from(u32::MAX)), "{y}");
        assert_eq!(i128::from(trunc_i64(y)), want_trunc(yd, i128::from(i64::MIN), i128::from(i64::MAX)), "{y}");
        assert_eq!(i128::from(trunc_u8(y)), want_trunc(yd, 0, i128::from(u8::MAX)), "{y}");
    }
}

/// `r` is the nearest f32 to the integer `n`, ties to even
fn nearest_f32(n: i128, r: f32) -> bool {
    if !r.is_finite() {
        return false;
    }
    let d = |f: f32| (exact(f64::from(f)) - n).abs();
    let (dr, up, down) = (d(r), d(r.next_up()), d(r.next_down()));
    dr <= up && dr <= down && (dr < up && dr < down || r.to_bits() & 1 == 0)
}

fn nearest_f64(n: i128, r: f64) -> bool {
    if !r.is_finite() {
        return false;
    }
    let d = |f: f64| (exact(f) - n).abs();
    let (dr, up, down) = (d(r), d(r.next_up()), d(r.next_down()));
    dr <= up && dr <= down && (dr < up && dr < down || r.to_bits() & 1 == 0)
}

#[test]
fn integers_round_to_nearest_even() {
    let mut r = Rng(2);
    let mut v: Vec<u64> = vec![0, 1, (1 << 24) - 1, 1 << 24, (1 << 24) + 1, (1 << 24) + 3, (1 << 53) + 1, u64::from(u32::MAX), u64::MAX, u64::MAX - 1, 1 << 63];
    for _ in 0..100_000 {
        let x = r.next();
        v.extend([x, x >> (x % 64), x >> 40]);
    }
    for x in v {
        let n = i128::from(x);
        assert!(nearest_f32(n, to_f32(x)), "{x}");
        assert!(nearest_f64(n, to_f64(x)), "{x}");
        let s = x.cast_signed();
        assert!(nearest_f32(i128::from(s), to_f32(s)), "{s}");
        assert!(nearest_f64(i128::from(s), to_f64(s)), "{s}");
        let [a, b, c, d, ..] = x.to_le_bytes();
        let w = u32::from_le_bytes([a, b, c, d]);
        assert!(nearest_f32(i128::from(w), to_f32(w)), "{w}");
        assert!(nearest_f32(i128::from(w.cast_signed()), to_f32(w.cast_signed())), "{w}");
        let u = usize::try_from(x).unwrap_or(usize::MAX);
        assert!(nearest_f32(i128::try_from(u).unwrap_or(i128::MAX), to_f32(u)), "{u}");
    }
}

#[test]
fn f64_to_f32_rounds_to_nearest_even() {
    for x in floats(3) {
        let r = to_f32(x);
        if x.is_nan() {
            assert!(r.is_nan());
            continue;
        }
        if x.abs() > f64::from(f32::MAX) {
            // (past the largest f32 by half an ulp or more: infinity; else the largest)
            let half_ulp = f64::from(f32::MAX) - f64::from(f32::MAX.next_down());
            let big = x.abs() >= f64::from(f32::MAX) + half_ulp / 2.0;
            assert!(if big { r.is_infinite() } else { r.abs() == f32::MAX }, "{x}");
            assert_eq!(r.is_sign_negative(), x.is_sign_negative());
            continue;
        }
        // (f64 holds every f32 and their differences near x exactly)
        let d = |f: f32| (f64::from(f) - x).abs();
        let (dr, up, down) = (d(r), d(r.next_up()), d(r.next_down()));
        assert!(dr <= up && dr <= down && (dr < up && dr < down || r.to_bits() & 1 == 0 || x == 0.0), "{x} -> {r}");
    }
}
