//! Env against the k32 code it replaced (copied here verbatim as the oracle) and against a
//! model (a map by upper-cased name), on random operation sequences.
use super::*;
use proptest::prelude::*;
use std::collections::BTreeMap;

/// k32's env_put before Env (the oracle).
fn old_put(e: &mut Vec<(String, String)>, name: &str, value: Option<&str>) {
    let key = name.to_ascii_uppercase();
    let at = e.iter().position(|(k, _)| k.to_ascii_uppercase() == key);
    match (at, value) {
        (Some(i), Some(v)) => e[i].1 = v.to_string(),
        (Some(i), None) => {
            e.remove(i);
        }
        (None, Some(v)) => {
            let i = e.iter().position(|(k, _)| k.to_ascii_uppercase() > key).unwrap_or(e.len());
            e.insert(i, (name.to_string(), v.to_string()));
        }
        (None, None) => {}
    }
}

/// k32's first-use environment before Env (the oracle), `extra` standing for WINETEST_* and
/// SAKE_GUEST_ENV.
fn old_initial(x64: bool, extra: &[(String, String)]) -> Vec<(String, String)> {
    let mut e: Vec<(String, String)> = DEFAULTS.iter().map(|&(k, v)| (k.to_string(), v.to_string())).collect();
    e.extend(wow64(x64).iter().map(|&(k, v)| (k.to_string(), v.to_string())));
    for (k, v) in extra {
        old_put(&mut e, k, Some(v));
    }
    e.sort_by_key(|(k, _)| k.to_ascii_uppercase());
    e
}

/// k32's expand_env before Env (the oracle).
fn old_expand(s: &str, get: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out += &rest[..i];
        let after = &rest[i + 1..];
        match after.find('%').and_then(|j| get(&after[..j]).map(|v| (j, v))) {
            Some((j, v)) => {
                out += &v;
                rest = &after[j + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out + rest
}

fn name() -> impl Strategy<Value = String> {
    prop_oneof!["[a-cA-C]{1,3}", Just("path".to_string()), Just("Temp".to_string()), Just("ProgramFiles".to_string()), "[a-z_()]{1,6}"]
}

#[derive(Clone, Debug)]
enum Op {
    Put(String, Option<String>),
    Get(String),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (name(), proptest::option::of("[a-z%;\\\\ ]{0,6}")).prop_map(|(n, v)| Op::Put(n, v)),
        name().prop_map(Op::Get),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// The same variables in the same order as the old code, and the same answers as a map by
    /// upper-cased name, through any sequence of sets, removals and reads.
    #[test]
    fn as_the_old_code_and_a_map(x64: bool, extra in proptest::collection::vec((name(), "[a-z]{0,4}"), 0..4), ops in proptest::collection::vec(op(), 0..40)) {
        let mut e = Env::initial(x64, extra.clone());
        let mut old = old_initial(x64, &extra);
        prop_assert_eq!(e.vars(), &old[..]);
        let mut model: BTreeMap<String, String> = old.iter().map(|(k, v)| (k.to_ascii_uppercase(), v.clone())).collect();
        for o in ops {
            match o {
                Op::Put(n, v) => {
                    e.put(&n, v.as_deref());
                    old_put(&mut old, &n, v.as_deref());
                    match v {
                        Some(v) => model.insert(n.to_ascii_uppercase(), v),
                        None => model.remove(&n.to_ascii_uppercase()),
                    };
                }
                Op::Get(n) => prop_assert_eq!(e.get(&n), model.get(&n.to_ascii_uppercase()).map(String::as_str)),
            }
            prop_assert_eq!(e.vars(), &old[..]);
            // (sorted by upper-cased name, one per name)
            prop_assert!(e.vars().windows(2).all(|w| w[0].0.to_ascii_uppercase() < w[1].0.to_ascii_uppercase()));
            prop_assert_eq!(e.vars().len(), model.len());
        }
        let block = e.block();
        prop_assert!(block.ends_with("\0\0") || (e.vars().is_empty() && block == "\0"));
        prop_assert_eq!(block.matches('\0').count(), e.vars().len() + 1);
    }

    /// expand: the old code's answers, for any text and environment.
    #[test]
    fn expand_as_the_old_code(vars in proptest::collection::vec((name(), "[a-z%]{0,4}"), 0..6), s in "[a-cA-C%x]{0,16}") {
        let e = Env::of(vars);
        prop_assert_eq!(e.expand(&s), old_expand(&s, |n| e.get(n).map(str::to_string)));
    }

    /// `of` keeps the last value of a name, as setting them in order does.
    #[test]
    fn of_is_putting_in_order(vars in proptest::collection::vec((name(), "[a-z]{0,4}"), 0..8)) {
        let mut e = Env::default();
        for (k, v) in &vars {
            e.put(k, Some(v));
        }
        prop_assert_eq!(Env::of(vars), e);
    }
}

#[test]
fn the_machine_decides_program_files() {
    let (a, b) = (Env::initial(true, []), Env::initial(false, []));
    assert_eq!(a.get("programfiles"), Some("C:\\Program Files"));
    assert_eq!(b.get("ProgramFiles"), Some("C:\\Program Files (x86)"));
    assert_eq!(b.get("PROCESSOR_ARCHITEW6432"), Some("AMD64"));
    assert_eq!(a.get("PROCESSOR_ARCHITEW6432"), None);
    assert_eq!(a.expand("%windir%\\x %nope% 100%"), "C:\\windows\\x %nope% 100%");
}
