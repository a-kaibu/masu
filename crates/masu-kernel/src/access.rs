/// A trusted request can omit credentials when strict mode is disabled.
pub fn anonymous_allowed(strict: bool, trusted: bool) -> bool {
    !strict && trusted
}

pub fn tls_relay_destination(route: &str) -> Option<(&'static str, u16)> {
    match route {
        "soft-denchi" => Some(("charge1.soft-denchi.jp", 443)),
        "dmm-softdenchi" => Some(("dlsoft.dmm.co.jp", 443)),
        "dmm-doujin-softdenchi" => Some(("softdenchi-doujin-api.dmm.co.jp", 443)),
        _ => None,
    }
}

/// After login, the browser can only return to a path on this server.
pub fn return_path(path: &[u8]) -> bool {
    let [b'/', tail @ ..] = path else {
        return false;
    };
    !matches!(tail.first(), Some(b'/' | b'\\')) && !tail.iter().any(|c| matches!(c, b'\r' | b'\n'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_non_strict_trusted_requests_can_omit_credentials() {
        assert!(anonymous_allowed(false, true));
        assert!(!anonymous_allowed(false, false));
        assert!(!anonymous_allowed(true, true));
        assert!(!anonymous_allowed(true, false));
    }

    #[test]
    fn accepts_only_same_server_redirect_paths() {
        assert!(return_path(b"/"));
        assert!(return_path(b"/games?q=a"));
        for path in [
            b"".as_slice(),
            b"//elsewhere",
            b"/\\elsewhere",
            b"https://elsewhere",
            b"/games\r\nnext",
        ] {
            assert!(!return_path(path));
        }
    }
}
