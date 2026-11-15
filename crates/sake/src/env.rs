//! A process environment as a value: pure functions, no globals (k32 keeps the process's one in
//! a Mutex and calls these). Names compare case-insensitively (ASCII, as Windows' environment
//! does), and the variables stay in Windows' order: sorted by upper-cased name.

/// The variables, sorted by upper-cased name, one per name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Env(Vec<(String, String)>);

fn key(name: &str) -> String {
    name.to_ascii_uppercase()
}

/// What every sake process starts with (besides [`wow64`]'s by the machine).
const DEFAULTS: [(&str, &str); 17] = [
    ("PATH", "C:\\windows\\system32;C:\\windows"),
    ("TEMP", "C:\\windows\\temp"),
    ("TMP", "C:\\windows\\temp"),
    ("windir", "C:\\windows"),
    ("SystemRoot", "C:\\windows"),
    ("OS", "Windows_NT"),
    ("COMPUTERNAME", "SAKE"),
    ("USERNAME", "user"),
    // (where SHGetFolderPath's folders are)
    ("USERPROFILE", "C:\\users\\user"),
    ("HOMEDRIVE", "C:"),
    ("HOMEPATH", "\\users\\user"),
    ("APPDATA", "C:\\users\\user\\AppData\\Roaming"),
    ("LOCALAPPDATA", "C:\\users\\user\\AppData\\Local"),
    ("PUBLIC", "C:\\users\\Public"),
    ("ProgramData", "C:\\ProgramData"),
    ("ALLUSERSPROFILE", "C:\\ProgramData"),
    ("SystemDrive", "C:"),
];

/// The variables 64-bit Windows gives a process by its machine: a 32-bit (WoW64) one has the
/// 32-bit Program Files as ProgramFiles and the 64-bit one as ProgramW6432.
pub(crate) fn wow64(x64: bool) -> &'static [(&'static str, &'static str)] {
    const BOTH: [(&str, &str); 4] = [
        ("ProgramFiles(x86)", "C:\\Program Files (x86)"),
        ("ProgramW6432", "C:\\Program Files"),
        ("CommonProgramFiles(x86)", "C:\\Program Files (x86)\\Common Files"),
        ("CommonProgramW6432", "C:\\Program Files\\Common Files"),
    ];
    const X64: [(&str, &str); 7] = [
        BOTH[0], BOTH[1], BOTH[2], BOTH[3],
        ("ProgramFiles", "C:\\Program Files"),
        ("CommonProgramFiles", "C:\\Program Files\\Common Files"),
        ("PROCESSOR_ARCHITECTURE", "AMD64"),
    ];
    const X86: [(&str, &str); 8] = [
        BOTH[0], BOTH[1], BOTH[2], BOTH[3],
        ("ProgramFiles", "C:\\Program Files (x86)"),
        ("CommonProgramFiles", "C:\\Program Files (x86)\\Common Files"),
        ("PROCESSOR_ARCHITECTURE", "x86"),
        ("PROCESSOR_ARCHITEW6432", "AMD64"),
    ];
    if x64 { &X64 } else { &X86 }
}

impl Env {
    /// A new process's: the defaults and the machine's, then `extra` over them (in order; the
    /// host's WINETEST_* and SAKE_GUEST_ENV's pairs, which the caller reads).
    pub(crate) fn initial(x64: bool, extra: impl IntoIterator<Item = (String, String)>) -> Env {
        let mut e = Env::of(DEFAULTS.iter().chain(wow64(x64)).map(|&(k, v)| (k.to_string(), v.to_string())));
        for (k, v) in extra {
            e.put(&k, Some(&v));
        }
        e
    }

    /// `vars` as an environment (a later one of a name wins over an earlier one).
    pub(crate) fn of(vars: impl IntoIterator<Item = (String, String)>) -> Env {
        let mut e = Env::default();
        for (k, v) in vars {
            e.put(&k, Some(&v));
        }
        e
    }

    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        let k = key(name);
        self.0.iter().find(|(n, _)| key(n) == k).map(|(_, v)| v.as_str())
    }

    /// Sets (or with None removes) a variable; a set one keeps the name it was first set with.
    pub(crate) fn put(&mut self, name: &str, value: Option<&str>) {
        let k = key(name);
        let at = self.0.iter().position(|(n, _)| key(n) == k);
        match (at.and_then(|i| self.0.get_mut(i)), value) {
            (Some(slot), Some(v)) => slot.1 = v.to_string(),
            (Some(_), None) => {
                self.0.retain(|(n, _)| key(n) != k);
            }
            (None, Some(v)) => {
                let i = self.0.iter().position(|(n, _)| key(n) > k).unwrap_or(self.0.len());
                self.0.insert(i, (name.to_string(), v.to_string()));
            }
            (None, None) => {}
        }
    }

    pub(crate) fn vars(&self) -> &[(String, String)] {
        &self.0
    }

    /// GetEnvironmentStrings' block: NAME=VALUE\0 each, then \0.
    pub(crate) fn block(&self) -> String {
        self.0.iter().map(|(k, v)| format!("{k}={v}\0")).collect::<String>() + "\0"
    }

    /// %NAME% replaced by its value (unknown names, and a lone %, stay as written).
    pub(crate) fn expand(&self, s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some((before, after)) = rest.split_once('%') {
            out += before;
            match after.split_once('%').and_then(|(name, tail)| self.get(name).map(|v| (v, tail))) {
                Some((v, tail)) => {
                    out += v;
                    rest = tail;
                }
                None => {
                    out.push('%');
                    rest = after;
                }
            }
        }
        out + rest
    }
}

#[cfg(test)]
#[path = "env_tests.rs"]
mod tests;
