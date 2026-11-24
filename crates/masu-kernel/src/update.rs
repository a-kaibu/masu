use alloc::vec::Vec;
use core::{cmp::Ordering, time::Duration};

pub const CHECK_OK: Duration = Duration::from_secs(600);
pub const CHECK_ERR: Duration = Duration::from_secs(60);

fn version(text: &str) -> (Vec<u64>, Option<&str>) {
    let text = text.trim().trim_start_matches('v');
    let (digits, pre) = text
        .split_once('-')
        .map_or((text, None), |(digits, pre)| (digits, Some(pre)));
    let mut numbers: Vec<u64> = digits
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect();
    while numbers.last() == Some(&0) {
        numbers.pop();
    }
    (numbers, pre)
}

/// Compare release versions numerically and prefer a release to its prerelease.
pub fn newer(a: &str, b: &str) -> bool {
    let ((a, ap), (b, bp)) = (version(a), version(b));
    match a.cmp(&b) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => match (ap, bp) {
            (None, Some(_)) => true,
            (Some(x), Some(y)) => x > y,
            _ => false,
        },
    }
}

pub fn fresh(age: Duration, ok: bool) -> bool {
    age < if ok { CHECK_OK } else { CHECK_ERR }
}
