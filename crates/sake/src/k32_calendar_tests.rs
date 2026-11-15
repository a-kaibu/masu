//! k32's calendar math (civil_from_days / days_from_civil) against the plain formulas in i128,
//! which cannot overflow: the saturating rewrite answers the same wherever i64 holds the answer.
use super::*;
use proptest::prelude::*;

fn civil_ref(days: i128) -> (i128, i128, i128) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}
fn days_ref(y: i128, m: i128, d: i128) -> i128 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

proptest! {
    #[test]
    fn civil_from_days_is_the_formula(days in -(1_i64 << 50)..(1_i64 << 50)) {
        let (y, m, d) = civil_from_days(days);
        prop_assert_eq!((i128::from(y), i128::from(m), i128::from(d)), civil_ref(i128::from(days)));
        prop_assert_eq!(days_from_civil(y, m, d), days);
    }
    #[test]
    fn days_from_civil_is_the_formula(y in -1_000_000_i64..1_000_000, m in -20_i64..40, d in -100_i64..100) {
        prop_assert_eq!(i128::from(days_from_civil(y, m, d)), days_ref(i128::from(y), i128::from(m), i128::from(d)));
    }
}

#[test]
fn known_days() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(days_from_civil(2000, 3, 1), 11017);
    assert_eq!(civil_from_days(-719468), (0, 3, 1));
    // (no panic at the ends)
    let _ = civil_from_days(i64::MIN);
    let _ = civil_from_days(i64::MAX);
    let _ = days_from_civil(i64::MAX, 1, 1);
}
