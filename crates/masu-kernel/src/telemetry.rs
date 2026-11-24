use alloc::{string::String, vec::Vec};

/// Return a suffix of whole lines, capped at `keep` bytes.
pub fn newest_lines(text: &[u8], keep: usize) -> &[u8] {
    if text.len() <= keep {
        return text;
    }
    let tail = text
        .get(text.len().saturating_sub(keep)..)
        .unwrap_or_default();
    tail.iter()
        .position(|&byte| byte == b'\n')
        .and_then(|i| tail.get(i.saturating_add(1)..))
        .unwrap_or_default()
}

/// Select old logs beyond either limit. The newest log is always retained.
pub fn logs_to_drop(list: &[(String, u64, u64)], count: usize, bytes: u64) -> Vec<&str> {
    let mut total = 0u64;
    let mut out = Vec::new();
    for (i, (name, size, _)) in list.iter().enumerate() {
        total = total.saturating_add(*size);
        if i > 0 && (i >= count || total > bytes) {
            out.push(name.as_str());
        }
    }
    out
}
