use super::Rect;
use proptest::prelude::{prop_assert, prop_assert_eq, proptest, Strategy};

fn rect() -> impl Strategy<Value = Rect> {
    (-50i32..50, -50i32..50, -50i32..50, -50i32..50).prop_map(|(l, t, r, b)| Rect { l, t, r, b })
}

/// the set of points of a rectangle, for the laws below
fn points(r: &Rect) -> Vec<(i32, i32)> {
    (r.t..r.b).flat_map(|y| (r.l..r.r).map(move |x| (x, y))).collect()
}

proptest! {
    #[test]
    fn and_has_exactly_the_common_points(a in rect(), b in rect(), x in -60i32..60, y in -60i32..60) {
        prop_assert_eq!(a.and(&b).has(x, y), a.has(x, y) && b.has(x, y));
    }

    #[test]
    fn or_covers_both(a in rect(), b in rect()) {
        let u = a.or(&b);
        for (x, y) in points(&a).into_iter().chain(points(&b)) {
            prop_assert!(u.has(x, y));
        }
    }

    #[test]
    fn minus_keeps_every_point_not_taken(a in rect(), b in rect()) {
        let m = a.minus(&b);
        for (x, y) in points(&a) {
            if !b.has(x, y) {
                prop_assert!(m.has(x, y), "{:?} - {:?} = {:?} lost ({x}, {y})", a, b, m);
            }
        }
    }

    #[test]
    fn off_moves_every_point(a in rect(), dx in -20i32..20, dy in -20i32..20, x in -60i32..60, y in -60i32..60) {
        prop_assert_eq!(a.off(dx, dy).has(x + dx, y + dy), a.has(x, y));
        prop_assert_eq!((a.off(dx, dy).w(), a.off(dx, dy).h()), (a.w(), a.h()));
    }

    #[test]
    fn empty_is_no_points(a in rect()) {
        prop_assert_eq!(a.empty(), points(&a).is_empty());
    }
}
