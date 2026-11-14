use super::f32_text;
use proptest::prelude::*;

/// Tint's rule: the text as a double must not exceed f32's range, and must still name the
/// same f32
fn tint_accepts(t: &str, f: f32) -> bool {
    let d: f64 = t.parse().unwrap();
    d.abs() <= f32::MAX as f64 && d as f32 == f && (d as f32).to_bits() == f.to_bits()
}

#[test]
fn largest_values() {
    assert_eq!(f32_text(f32::MAX), "3.4028234663852886e38");
    assert_eq!(f32_text(f32::MIN), "-3.4028234663852886e38");
    assert_eq!(f32_text(1.5), "1.5");
    assert_eq!(f32_text(-0.0), "-0.0");
    for f in [f32::MAX, f32::MIN, f32::from_bits(0x7f7f_fffe), f32::MIN_POSITIVE, f32::from_bits(1), 3.402823466e38] {
        assert!(tint_accepts(&f32_text(f), f), "{f:?} -> {}", f32_text(f));
    }
}

proptest! {
    #[test]
    fn every_finite_value_reads_back(bits in any::<u32>()) {
        let f = f32::from_bits(bits);
        prop_assume!(f.is_finite());
        let t = f32_text(f);
        prop_assert!(tint_accepts(&t, f), "{:?} -> {}", f, t);
    }
    // the top of the range, where shortest decimals round up past f32::MAX
    #[test]
    fn top_of_the_range_reads_back(k in 0u32..0x10000) {
        for sign in [0u32, 0x8000_0000] {
            let f = f32::from_bits((0x7f7f_ffff - k) | sign);
            let t = f32_text(f);
            prop_assert!(tint_accepts(&t, f), "{:?} -> {}", f, t);
        }
    }
}
