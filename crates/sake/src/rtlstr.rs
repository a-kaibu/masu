//! ntdll's Rtl functions on counted strings (STRING / UNICODE_STRING): init, append, copy, case,
//! duplicate, find (clean-room rules: sake-re/ntdll/rtl-strings.md). Each function's rule is a
//! pure function over the string headers that says what it writes (a plan); the handlers read the
//! headers, carry the plan out and answer its status.
use crate::api::{implement, Ctx};
use crate::mem;
use crate::ntstr::Status;

/// a STRING / UNICODE_STRING header: Length and MaximumLength in bytes, the Buffer
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Counted {
    pub len: u16,
    pub max: u16,
    pub buf: u32,
}

impl Counted {
    pub fn read(at: u32) -> Counted {
        Counted { len: mem::r16(mem::ga(at)), max: mem::r16(mem::ga(at) + 2), buf: mem::rp(mem::ga(at) + mem::ps()) }
    }
    pub fn write(self, at: u32) {
        mem::w16(mem::ga(at), self.len);
        mem::w16(mem::ga(at) + 2, self.max);
        mem::wp(mem::ga(at) + mem::ps(), self.buf);
    }
    /// the validation Duplicate and Find make: even lengths, Length within MaximumLength within
    /// 0xFFFE, a Buffer unless both are 0
    pub const fn valid(self) -> bool {
        self.len & 1 == 0 && self.max & 1 == 0 && self.len <= self.max && self.max <= 0xfffe && (self.buf != 0 || (self.len == 0 && self.max == 0))
    }
    fn bytes(self) -> Vec<u8> {
        (0..u32::from(self.len)).map(|i| mem::r8(mem::ga(self.buf) + i)).collect()
    }
    fn units(self) -> Vec<u16> {
        (0..u32::from(self.len / 2)).map(|i| mem::r16(mem::ga(self.buf).at(i, 2))).collect()
    }
}

/// what an append writes: `n` bytes at Buffer + `at`, the new Length, and where a 0 unit goes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Append {
    pub at: u16,
    pub n: u16,
    pub len: u16,
    pub nul: Option<u16>,
}

fn too_small(dst: Counted, n: u32) -> bool {
    u32::from(dst.len).saturating_add(n) > u32::from(dst.max)
}

/// RtlAppendAsciizToString: `n` = strlen of the source (no 0 written)
pub fn append_asciiz(dst: Counted, n: usize) -> Result<Append, Status> {
    let Ok(n) = u16::try_from(n) else { return Err(Status::BufferTooSmall) };
    if too_small(dst, u32::from(n)) {
        return Err(Status::BufferTooSmall);
    }
    Ok(Append { at: dst.len, n, len: dst.len.wrapping_add(n), nul: None })
}

/// RtlAppendStringToString: `n` = the source's Length (an empty source: nothing at all)
pub fn append_string(dst: Counted, n: u16) -> Result<Option<Append>, Status> {
    if n == 0 {
        return Ok(None);
    }
    if too_small(dst, u32::from(n)) {
        return Err(Status::BufferTooSmall);
    }
    Ok(Some(Append { at: dst.len, n, len: dst.len.wrapping_add(n), nul: None }))
}

/// the unicode appends: at the whole units of Length; a 0 unit after when one more unit fits
fn append_units(dst: Counted, n: u16) -> Result<Append, Status> {
    if too_small(dst, u32::from(n)) {
        return Err(Status::BufferTooSmall);
    }
    let at = dst.len & !1;
    let len = dst.len.wrapping_add(n);
    let nul = (u32::from(len).saturating_add(1) < u32::from(dst.max)).then(|| at.wrapping_add(n & !1));
    Ok(Append { at, n, len, nul })
}

/// RtlAppendUnicodeStringToString: `n` = the source's Length (an empty source: nothing at all)
pub fn append_unicode_string(dst: Counted, n: u16) -> Result<Option<Append>, Status> {
    if n == 0 {
        return Ok(None);
    }
    append_units(dst, n).map(Some)
}

/// RtlAppendUnicodeToString: `units` = wcslen of the source
pub fn append_unicode(dst: Counted, units: usize) -> Result<Append, Status> {
    if units > 0x7ffe {
        return Err(Status::BufferTooSmall);
    }
    append_units(dst, kome::num::lo16(kome::num::u32_of(units).saturating_mul(2)))
}

/// RtlInitUnicodeStringEx: the header for a string of `units` units at `s` (0: none)
pub fn init_ex(s: u32, units: usize) -> (Counted, Status) {
    let none = Counted { len: 0, max: 0, buf: s };
    if s == 0 {
        return (none, Status::Success);
    }
    match u16::try_from(units) {
        Ok(n) if n <= 0x7ffe => (Counted { len: n.wrapping_mul(2), max: n.wrapping_mul(2).wrapping_add(2), buf: s }, Status::Success),
        _ => (none, Status::NameTooLong),
    }
}

/// RtlUpperChar: ASCII only
pub const fn upper_char(b: u8) -> u8 {
    if b.is_ascii_lowercase() { b ^ 0x20 } else { b }
}

/// the case of RtlUpcaseUnicodeString / RtlDowncaseUnicodeString
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    Up,
    Down,
}

impl Case {
    pub fn map(self, u: u16) -> u16 {
        match self {
            Case::Up => crate::nt::up(u),
            Case::Down => crate::nt::down(u),
        }
    }
}

/// where Up/Downcase's units go
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseTarget {
    /// an empty source without allocation: Length 0 and nothing else
    Empty,
    /// a new buffer of the source's Length
    Allocate,
    /// the caller's buffer
    Into,
}

pub fn case_target(dst: Counted, src_len: u16, alloc: bool) -> Result<CaseTarget, Status> {
    if !alloc && src_len == 0 {
        Ok(CaseTarget::Empty)
    } else if alloc {
        Ok(CaseTarget::Allocate)
    } else if src_len > dst.max || dst.buf == 0 {
        Err(Status::BufferOverflow)
    } else {
        Ok(CaseTarget::Into)
    }
}

/// RtlDuplicateUnicodeString's flags
pub const DUP_NUL: u32 = 1;
pub const DUP_ALLOCATE_EMPTY: u32 = 2;

/// the new string's Length and MaximumLength (MaximumLength 0: no buffer)
pub fn duplicate(flags: u32, src: Option<Counted>) -> Result<(u16, u16), Status> {
    // (allocating an empty one is only asked with the 0: flag 2 alone is invalid)
    if flags & !3 != 0 || flags == DUP_ALLOCATE_EMPTY || src.is_some_and(|s| !s.valid()) {
        return Err(Status::InvalidParameter);
    }
    let len = src.map_or(0, |s| s.len);
    let max = if flags & DUP_NUL != 0 {
        if len == 0xfffe {
            return Err(Status::NameTooLong);
        }
        len.wrapping_add(2)
    } else {
        len
    };
    let max = if flags & DUP_ALLOCATE_EMPTY == 0 && len == 0 { 0 } else { max };
    Ok((len, max))
}

/// RtlFindCharInUnicodeString's flags
pub const FIND_FROM_END: u32 = 1;
pub const FIND_COMPLEMENT: u32 = 2;
pub const FIND_CASE_INSENSITIVE: u32 = 4;

/// the byte position Find answers for `s` and `set` (forward: after the unit, backward: at it)
pub fn find(flags: u32, s: &[u16], set: &[u16]) -> Result<u16, Status> {
    let fold = |u: u16| if flags & FIND_CASE_INSENSITIVE != 0 { crate::nt::down(u) } else { u };
    let set: Vec<u16> = set.iter().map(|&u| fold(u)).collect();
    let hit = |u: &u16| set.contains(&fold(*u)) != (flags & FIND_COMPLEMENT != 0);
    let pos = |i: usize| kome::num::lo16(kome::num::u32_of(i).saturating_mul(2));
    if flags & FIND_FROM_END != 0 {
        s.iter().rposition(hit).map(pos)
    } else {
        s.iter().position(hit).map(|i| pos(i.saturating_add(1)))
    }
    .ok_or(Status::NotFound)
}

/// where RtlUnicodeStringToAnsiString puts the text (`size` = its ANSI bytes + the 0)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnsiTarget {
    /// a new buffer of `size`
    Allocate,
    /// the caller's: at most `cap` bytes then the 0; `cut` when they are fewer than the text's
    Into { cap: u16, cut: bool },
}

pub fn ansi_target(dst_max: u16, size: usize, alloc: bool) -> Result<AnsiTarget, Status> {
    let Ok(size) = u16::try_from(size) else { return Err(Status::InvalidParameter2) };
    if alloc {
        Ok(AnsiTarget::Allocate)
    } else if size <= dst_max {
        Ok(AnsiTarget::Into { cap: size.wrapping_sub(1), cut: false })
    } else if dst_max == 0 {
        Err(Status::BufferOverflow)
    } else {
        Ok(AnsiTarget::Into { cap: dst_max.wrapping_sub(1), cut: true })
    }
}

/// RtlCompareUnicodeStrings: the first units that differ (upper-cased when `ci`), else the counts
pub fn compare_units(a: &[u16], b: &[u16], ci: bool) -> i32 {
    for (&x, &y) in a.iter().zip(b) {
        if x != y {
            let (x, y) = if ci { (crate::nt::up(x), crate::nt::up(y)) } else { (x, y) };
            if x != y {
                return i32::from(x).wrapping_sub(i32::from(y));
            }
        }
    }
    kome::num::lo32(a.len()).wrapping_sub(kome::num::lo32(b.len())).cast_signed()
}

/// the format RtlGUIDFromString reads and RtlStringFromGUID writes
const GUID_FORMAT: &str = "{%08lx-%04x-%04x-%02x%02x-%02x%02x%02x%02x%02x%02x}";

fn hex_digit(u: u16) -> Option<u32> {
    match u {
        0x30..=0x39 => Some(u32::from(u).wrapping_sub(0x30)),
        0x61..=0x66 => Some(u32::from(u).wrapping_sub(0x57)),
        0x41..=0x46 => Some(u32::from(u).wrapping_sub(0x37)),
        _ => None,
    }
}

/// RtlGUIDFromString's scan of `units` with GUID_FORMAT: the GUID's 16 bytes, or None
pub fn guid_from(units: &[u16]) -> Option<[u8; 16]> {
    let f: Vec<u16> = GUID_FORMAT.encode_utf16().collect();
    let (mut fi, mut si) = (0usize, 0usize);
    let mut fields: Vec<(u32, bool)> = Vec::new();
    while let Some(&fc) = f.get(fi) {
        if fc != u16::from(b'%') {
            // (a literal: the next unit equals it)
            if units.get(si) != Some(&fc) {
                return None;
            }
            si = si.saturating_add(1);
            fi = fi.saturating_add(1);
            continue;
        }
        let (mut width, mut long) = (0u32, false);
        fi = fi.saturating_add(1);
        loop {
            let c = f.get(fi).copied()?;
            fi = fi.saturating_add(1);
            match c {
                0x30..=0x39 => width = width.wrapping_mul(10).wrapping_add(u32::from(c).wrapping_sub(0x30)),
                0x6c => long = true,
                0x78 | 0x58 => break,
                _ => {}
            }
        }
        let mut v = 0u32;
        for _ in 0..width {
            let d = hex_digit(units.get(si).copied()?)?;
            v = v.wrapping_shl(4).wrapping_add(d);
            si = si.saturating_add(1);
        }
        fields.push((v, long));
    }
    if units.get(si).is_some_and(|&u| u != 0) {
        return None;
    }
    let mut g = [0u8; 16];
    let mut at = 0usize;
    for (k, (v, _)) in fields.iter().enumerate() {
        // (Data1 4 bytes, Data2/3 2, Data4 a byte each)
        let n = match k {
            0 => 4,
            1 | 2 => 2,
            _ => 1,
        };
        for (i, b) in v.to_le_bytes().into_iter().take(n).enumerate() {
            if let Some(x) = g.get_mut(at.saturating_add(i)) {
                *x = b;
            }
        }
        at = at.saturating_add(n);
    }
    Some(g)
}

/// RtlStringFromGUID's text (lower case, braces)
pub fn guid_text(g: &[u8; 16]) -> String {
    let [a0, a1, a2, a3, b0, b1, c0, c1, d @ ..] = *g;
    let d1 = u32::from_le_bytes([a0, a1, a2, a3]);
    let (d2, d3) = (u16::from_le_bytes([b0, b1]), u16::from_le_bytes([c0, c1]));
    let [e0, e1, e2, e3, e4, e5, e6, e7] = d;
    format!("{{{d1:08x}-{d2:04x}-{d3:04x}-{e0:02x}{e1:02x}-{e2:02x}{e3:02x}{e4:02x}{e5:02x}{e6:02x}{e7:02x}}}")
}

/// RtlUTF8ToUnicodeN's conversion of all of `b`: the units, and whether some were replaced. A
/// lead byte C2..F4 reads its continuation bytes; the second one is checked against the lead's
/// range (E0: A0.., ED: ..9F, F0: 90.., F4: ..8F) and a wrong one is swallowed with the lead into one
/// U+FFFD; a byte that is no continuation ends the sequence (one U+FFFD) and is read again; any
/// other byte past 0x7F, or a sequence the end cuts, is one U+FFFD
pub fn utf8_units(b: &[u8]) -> (Vec<u16>, bool) {
    let (mut w, mut bad) = (Vec::new(), false);
    let cont = |x: Option<&u8>| x.is_some_and(|&x| x & 0xc0 == 0x80);
    let mut i = 0usize;
    while let Some(&c) = b.get(i) {
        let (n, lo, hi) = match c {
            0..0x80 => {
                w.push(u16::from(c));
                i = i.saturating_add(1);
                continue;
            }
            0xc2..=0xdf => (2usize, 0x80u8, 0xbfu8),
            0xe0 => (3, 0xa0, 0xbf),
            0xed => (3, 0x80, 0x9f),
            0xe1..=0xef => (3, 0x80, 0xbf),
            0xf0 => (4, 0x90, 0xbf),
            0xf4 => (4, 0x80, 0x8f),
            0xf1..=0xf3 => (4, 0x80, 0xbf),
            0x80..=0xc1 | 0xf5..=0xff => (1, 0, 0),
        };
        let mut v = u32::from(c) & (0x7f >> n);
        let mut k = 1usize;
        let ok = n > 1 && loop {
            if k == n {
                break true;
            }
            let x = b.get(i.saturating_add(k));
            if !cont(x) {
                break false;
            }
            let x = x.copied().unwrap_or(0);
            k = k.saturating_add(1);
            if k == 2 && !(lo..=hi).contains(&x) {
                break false;
            }
            v = v << 6 | u32::from(x & 0x3f);
        };
        i = i.saturating_add(k);
        match char::from_u32(v).filter(|_| ok) {
            Some(ch) => {
                let mut buf = [0u16; 2];
                w.extend_from_slice(ch.encode_utf16(&mut buf));
            }
            None => {
                w.push(0xfffd);
                bad = true;
            }
        }
    }
    (w, bad)
}

/// RtlUnicodeToUTF8N's conversion of all of `w`: each character's bytes (a surrogate out of a
/// pair is U+FFFD), and whether some were replaced
pub fn utf8_chars(w: &[u16]) -> (Vec<Vec<u8>>, bool) {
    let (mut out, mut bad) = (Vec::new(), false);
    for r in char::decode_utf16(w.iter().copied()) {
        let c = r.unwrap_or_else(|_| {
            bad = true;
            '\u{fffd}'
        });
        let mut buf = [0u8; 4];
        out.push(c.encode_utf8(&mut buf).as_bytes().to_vec());
    }
    (out, bad)
}

/// RtlHashUnicodeString's hash (both algorithms are X65599): h * 0x1003F + unit, upper-cased when
/// `ci`
pub fn hash(units: &[u16], ci: bool) -> u32 {
    units.iter().fold(0u32, |h, &u| h.wrapping_mul(0x1003f).wrapping_add(u32::from(if ci { crate::nt::up(u) } else { u })))
}

fn st(r: Result<(), Status>) -> u32 {
    r.err().unwrap_or(Status::Success).code()
}

/// carry an append out: `src` read from `from`
fn put(dst_at: u32, dst: Counted, a: Append, from: u32) {
    let b: Vec<u8> = (0..u32::from(a.n)).map(|i| mem::r8(mem::ga(from) + i)).collect();
    mem::write(dst.buf.wrapping_add(u32::from(a.at)), &b);
    mem::w16(mem::ga(dst_at), a.len);
    if let Some(z) = a.nul {
        mem::w16(mem::ga(dst.buf.wrapping_add(u32::from(z))), 0);
    }
}

fn wlen(s: u32) -> usize {
    let mut n = 0u32;
    while mem::r16(mem::ga(s).at(n, 2)) != 0 {
        n = n.saturating_add(1);
    }
    kome::num::us(n)
}
fn clen(s: u32) -> usize {
    let mut n = 0u32;
    while mem::r8(mem::ga(s) + n) != 0 {
        n = n.saturating_add(1);
    }
    kome::num::us(n)
}

fn init_ex_h(c: &mut Ctx) -> u32 {
    let (d, s) = (c.a(0), c.a(1));
    let (h, status) = init_ex(s, if s == 0 { 0 } else { wlen(s) });
    h.write(d);
    status.code()
}
fn append_asciiz_h(c: &mut Ctx) -> u32 {
    let (d, s) = (c.a(0), c.a(1));
    if s == 0 {
        return 0;
    }
    let dst = Counted::read(d);
    st(append_asciiz(dst, clen(s)).map(|a| put(d, dst, a, s)))
}
fn append_string_h(c: &mut Ctx) -> u32 {
    let (d, s) = (c.a(0), Counted::read(c.a(1)));
    let dst = Counted::read(d);
    st(append_string(dst, s.len).map(|a| a.map_or((), |a| put(d, dst, a, s.buf))))
}
fn append_unicode_string_h(c: &mut Ctx) -> u32 {
    let (d, s) = (c.a(0), Counted::read(c.a(1)));
    let dst = Counted::read(d);
    st(append_unicode_string(dst, s.len).map(|a| a.map_or((), |a| put(d, dst, a, s.buf))))
}
fn append_unicode_h(c: &mut Ctx) -> u32 {
    let (d, s) = (c.a(0), c.a(1));
    if s == 0 {
        return 0;
    }
    let dst = Counted::read(d);
    st(append_unicode(dst, wlen(s)).map(|a| put(d, dst, a, s)))
}
fn copy_string_h(c: &mut Ctx) -> u32 {
    let (d, s) = (c.a(0), c.a(1));
    mem::w16(mem::ga(d), 0);
    if s != 0 {
        let (dst, src) = (Counted::read(d), Counted::read(s));
        let n = src.len.min(dst.max);
        mem::w16(mem::ga(d), n);
        mem::write(dst.buf, &Counted { len: n, ..src }.bytes());
    }
    0
}
fn upper_char_h(c: &mut Ctx) -> u32 {
    u32::from(upper_char(kome::num::lo8(c.a(0))))
}
fn upper_string_h(c: &mut Ctx) -> u32 {
    let (d, s) = (c.a(0), c.a(1));
    let (dst, src) = (Counted::read(d), Counted::read(s));
    let n = src.len.min(dst.max);
    mem::w16(mem::ga(d), n);
    let b: Vec<u8> = Counted { len: n, ..src }.bytes().into_iter().map(upper_char).collect();
    mem::write(dst.buf, &b);
    0
}
fn case_h(c: &mut Ctx, case: Case) -> u32 {
    let (d, src, alloc) = (c.a(0), Counted::read(c.a(1)), c.boolean(2));
    let mut dst = Counted::read(d);
    match case_target(dst, src.len, alloc) {
        Err(e) => return e.code(),
        Ok(CaseTarget::Empty) => {
            mem::w16(mem::ga(d), 0);
            return 0;
        }
        Ok(CaseTarget::Allocate) => {
            let b = mem::heap_alloc(u32::from(src.len), false);
            mem::wp(mem::ga(d) + mem::ps(), b);
            if b == 0 {
                mem::w16(mem::ga(d) + 2, 0);
                return Status::NoMemory.code();
            }
            mem::w16(mem::ga(d) + 2, src.len);
            dst.buf = b;
        }
        Ok(CaseTarget::Into) => {}
    }
    for (i, u) in src.units().into_iter().enumerate() {
        mem::w16(mem::ga(dst.buf).at(kome::num::u32_of(i), 2), case.map(u));
    }
    mem::w16(mem::ga(d), src.len);
    0
}
fn duplicate_h(c: &mut Ctx) -> u32 {
    let (flags, s, d) = (c.a(0), c.a(1), c.a(2));
    if d == 0 {
        return Status::InvalidParameter.code();
    }
    let src = (s != 0).then(|| Counted::read(s));
    match duplicate(flags, src) {
        Err(e) => e.code(),
        Ok((len, max)) => {
            let buf = if max == 0 { 0 } else { mem::heap_alloc(u32::from(max), false) };
            if max != 0 && buf == 0 {
                return Status::NoMemory.code();
            }
            if buf != 0 {
                if let Some(src) = src {
                    mem::write(buf, &src.bytes());
                }
                if flags & DUP_NUL != 0 {
                    mem::w16(mem::ga(buf) + u32::from(len & !1), 0);
                }
            }
            Counted { len, max, buf }.write(d);
            0
        }
    }
}
fn find_h(c: &mut Ctx) -> u32 {
    let (flags, s, set, pos) = (c.a(0), c.a(1), c.a(2), c.a(3));
    if pos != 0 {
        mem::w16(mem::ga(pos), 0);
    }
    if flags & !7 != 0 || pos == 0 {
        return Status::InvalidParameter.code();
    }
    let (s, set) = (Counted::read(s), Counted::read(set));
    if !s.valid() || !set.valid() {
        return Status::InvalidParameter.code();
    }
    match find(flags, &s.units(), &set.units()) {
        Ok(p) => {
            mem::w16(mem::ga(pos), p);
            0
        }
        Err(e) => e.code(),
    }
}

fn compare_units_h(c: &mut Ctx) -> u32 {
    // (the counts are SIZE_T: one slot each on both architectures)
    let (a, na, b, nb, ci) = (c.a(0), c.a(1), c.a(2), c.a(3), c.boolean(4));
    let read = |s: u32, n: u32| -> Vec<u16> { (0..n).map(|i| mem::r16(mem::ga(s).at(i, 2))).collect() };
    compare_units(&read(a, na), &read(b, nb), ci).cast_unsigned()
}
fn guid_from_h(c: &mut Ctx) -> u32 {
    let (s, g) = (Counted::read(c.a(0)), c.a(1));
    match guid_from(&s.units()) {
        Some(b) => {
            mem::write(g, &b);
            0
        }
        None => Status::InvalidParameter.code(),
    }
}
fn guid_text_h(c: &mut Ctx) -> u32 {
    let (g, d) = (c.a(0), c.a(1));
    let mut b = [0u8; 16];
    for (i, x) in b.iter_mut().enumerate() {
        *x = mem::r8(mem::ga(g) + kome::num::u32_of(i));
    }
    let buf = mem::heap_alloc(0x4e, false);
    mem::w16(mem::ga(d) + 2, 0x4e);
    mem::wp(mem::ga(d) + mem::ps(), buf);
    if buf == 0 {
        return Status::NoMemory.code();
    }
    mem::w16(mem::ga(d), 0x4c);
    for (i, u) in guid_text(&b).encode_utf16().chain([0]).enumerate() {
        mem::w16(mem::ga(buf).at(kome::num::u32_of(i), 2), u);
    }
    0
}
fn some(bad: bool) -> Status {
    if bad { Status::SomeNotMapped } else { Status::Success }
}
fn utf8_to_unicode_h(c: &mut Ctx) -> u32 {
    let (dst, cap, pw, src, n) = (c.a(0), c.a(1), c.a(2), c.a(3), c.a(4));
    if src == 0 {
        return Status::InvalidParameter4.code();
    }
    let b: Vec<u8> = (0..n).map(|i| mem::r8(mem::ga(src) + i)).collect();
    let (w, bad) = utf8_units(&b);
    let need = kome::num::u32_of(w.len()).saturating_mul(2);
    if dst == 0 {
        if pw == 0 {
            return Status::InvalidParameter.code();
        }
        mem::w32(pw, need);
        return some(bad).code();
    }
    let room = kome::num::us(cap / 2);
    for (i, &u) in w.iter().take(room).enumerate() {
        mem::w16(mem::ga(dst).at(kome::num::u32_of(i), 2), u);
    }
    if pw != 0 {
        mem::w32(pw, kome::num::u32_of(w.len().min(room)).saturating_mul(2));
    }
    if w.len() > room { Status::BufferTooSmall.code() } else { some(bad).code() }
}
fn unicode_to_utf8_h(c: &mut Ctx) -> u32 {
    let (dst, cap, pw, src, n) = (c.a(0), c.a(1), c.a(2), c.a(3), c.a(4));
    if src == 0 {
        return Status::InvalidParameter4.code();
    }
    if dst == 0 && pw == 0 {
        return Status::InvalidParameter.code();
    }
    if dst != 0 && n & 1 != 0 {
        return Status::InvalidParameter5.code();
    }
    let w: Vec<u16> = (0..n / 2).map(|i| mem::r16(mem::ga(src).at(i, 2))).collect();
    let (chars, bad) = utf8_chars(&w);
    if dst == 0 {
        mem::w32(pw, kome::num::u32_of(chars.iter().map(Vec::len).sum::<usize>()));
        return some(bad).code();
    }
    let mut out: Vec<u8> = Vec::new();
    let mut full = false;
    for ch in &chars {
        if out.len().saturating_add(ch.len()) > kome::num::us(cap) {
            full = true;
            break;
        }
        out.extend(ch);
    }
    mem::write(dst, &out);
    if pw != 0 {
        mem::w32(pw, kome::num::u32_of(out.len()));
    }
    if full { Status::BufferTooSmall.code() } else { some(bad).code() }
}

fn hash_h(c: &mut Ctx) -> u32 {
    let (s, ci, alg, out) = (c.a(0), c.boolean(1), c.a(2), c.a(3));
    if s == 0 || out == 0 {
        return Status::InvalidParameter.code();
    }
    mem::w32(out, 0);
    if alg > 1 {
        return Status::InvalidParameter.code();
    }
    mem::w32(out, hash(&Counted::read(s).units(), ci));
    0
}

fn unicode_to_ansi_h(c: &mut Ctx) -> u32 {
    let (d, src, alloc) = (c.a(0), Counted::read(c.a(1)), c.boolean(2));
    // (whole characters: a double-byte one that does not fit is left out)
    let chars: Vec<Vec<u8>> = src.units().iter().map(|&u| crate::text::w2a(&[u])).collect();
    let size = chars.iter().map(Vec::len).sum::<usize>().saturating_add(1);
    let mut dst = Counted::read(d);
    let (cap, cut) = match ansi_target(dst.max, size, alloc) {
        Err(e) => return e.code(),
        Ok(AnsiTarget::Allocate) => {
            let n = kome::num::lo16(kome::num::u32_of(size));
            let b = mem::heap_alloc(u32::from(n), false);
            mem::wp(mem::ga(d) + mem::ps(), b);
            if b == 0 {
                return Status::NoMemory.code();
            }
            mem::w16(mem::ga(d) + 2, n);
            dst.buf = b;
            (n.wrapping_sub(1), false)
        }
        Ok(AnsiTarget::Into { cap, cut }) => (cap, cut),
    };
    let mut out: Vec<u8> = Vec::new();
    for ch in &chars {
        if out.len().saturating_add(ch.len()) > usize::from(cap) {
            break;
        }
        out.extend(ch);
    }
    out.push(0);
    mem::write(dst.buf, &out);
    mem::w16(mem::ga(d), kome::num::lo16(kome::num::u32_of(out.len().saturating_sub(1))));
    if cut { Status::BufferOverflow } else { Status::Success }.code()
}

/// RtlIsTextUnicode's flags (IS_TEXT_UNICODE_*)
pub mod itu {
    pub const ASCII16: u32 = 0x1;
    pub const STATISTICS: u32 = 0x2;
    pub const CONTROLS: u32 = 0x4;
    pub const SIGNATURE: u32 = 0x8;
    pub const REVERSE_ASCII16: u32 = 0x10;
    pub const REVERSE_STATISTICS: u32 = 0x20;
    pub const REVERSE_CONTROLS: u32 = 0x40;
    pub const REVERSE_SIGNATURE: u32 = 0x80;
    pub const ILLEGAL_CHARS: u32 = 0x100;
    pub const ODD_LENGTH: u32 = 0x200;
    pub const DBCS_LEADBYTE: u32 = 0x400;
    pub const NULL_BYTES: u32 = 0x1000;
}

/// RtlIsTextUnicode (sake-re/ntdll/rtl-strings.md): `bytes` the first min(Length, 512) bytes,
/// `lead` the ANSI code page's lead bytes (None: not a DBCS code page), `mask` *Flags (None:
/// no Flags). The answer and the flags (also what goes to *Flags: the tests found, masked).
pub fn text_unicode(bytes: &[u8], length: u32, lead: Option<&dyn Fn(u8) -> bool>, mask: Option<u32>) -> (bool, u32) {
    let all = kome::num::us(length >> 1);
    let mut n = all.min(256);
    let unit = |k: usize| -> u16 {
        let lo = bytes.get(k.saturating_mul(2)).copied().unwrap_or(0);
        let hi = bytes.get(k.saturating_mul(2).saturating_add(1)).copied().unwrap_or(0);
        u16::from_le_bytes([lo, hi])
    };
    if n == 0 || (length == 2 && unit(0) != 0 && unit(0) >> 8 == 0) {
        return (false, 5);
    }
    // (a last unit with a 0 high byte is left out: a terminating 0, or an ANSI character)
    if length > 2 && all <= 256 && length & 1 == 0 && unit(n.wrapping_sub(1)) & 0xff00 == 0 {
        n = n.wrapping_sub(1);
    }
    #[derive(Default)]
    struct Count {
        nul: u32,
        tab: u32,
        lf: u32,
        cr: u32,
        space: u32,
        ideo_space: u32,
        ffff: u32,
        rbom: u32,
        rcr: u32,
        rtab: u32,
        rlf: u32,
        rcrlf: u32,
        rspace: u32,
        ansi_breaks: u32,
        zero_bytes: u32,
        dhi: u32,
        dlo: u32,
    }
    let mut k = Count::default();
    let inc = |x: &mut u32| *x = x.wrapping_add(1);
    let (mut phi, mut plo) = (0u8, 0u8);
    for i in 0..n {
        let c = unit(i);
        match c {
            0 => inc(&mut k.nul),
            9 => inc(&mut k.tab),
            0xa => inc(&mut k.lf),
            0xd => inc(&mut k.cr),
            0x20 => inc(&mut k.space),
            0x3000 => inc(&mut k.ideo_space),
            0xffff => inc(&mut k.ffff),
            0xfffe => inc(&mut k.rbom),
            0xd00 => inc(&mut k.rcr),
            0x900 => inc(&mut k.rtab),
            0xa00 => inc(&mut k.rlf),
            0xa0d => inc(&mut k.rcrlf),
            0x2000 => inc(&mut k.rspace),
            _ => {}
        }
        let [lo, hi] = c.to_le_bytes();
        // (a CR LF or LF CR across the units: the high byte before and this low byte)
        if (lo == 0xd && phi == 0xa) || (lo == 0xa && phi == 0xd) {
            inc(&mut k.ansi_breaks);
        }
        k.zero_bytes = k.zero_bytes.wrapping_add(u32::from(hi == 0)).wrapping_add(u32::from(lo == 0));
        k.dhi = k.dhi.wrapping_add(u32::from(hi.abs_diff(phi)));
        k.dlo = k.dlo.wrapping_add(u32::from(lo.abs_diff(plo)));
        (phi, plo) = (hi, lo);
    }
    // (the last unit: a CR LF or LF CR inside it, a Ctrl-Z high byte; a 0 high byte is not a
    // null byte)
    if (plo == 0xd && phi == 0xa) || (plo == 0xa && phi == 0xd) {
        inc(&mut k.ansi_breaks);
    }
    if phi == 0 {
        k.zero_bytes = k.zero_bytes.wrapping_sub(1);
    } else if phi == 0x1a {
        inc(&mut k.ansi_breaks);
    }
    let window = length.min(0x200);
    let mut flags = if k.dlo < 0x7f && k.dhi == 0 {
        itu::ASCII16
    } else if k.dhi != 0 && k.dlo == 0 {
        itu::REVERSE_ASCII16
    } else {
        0
    };
    let mut weight = 3u32;
    if let Some(lead) = lead {
        let mut leads = 0u32;
        let mut b = 0usize;
        while b < kome::num::us(window) {
            if lead(bytes.get(b).copied().unwrap_or(0)) {
                leads = leads.wrapping_add(1);
                b = b.saturating_add(1);
            }
            b = b.saturating_add(1);
        }
        if leads != 0 && mask.is_some_and(|m| m & itu::DBCS_LEADBYTE != 0) {
            let half = kome::num::u32_of(all.min(256) >> 1).wrapping_sub(1);
            weight = if leads < half / 3 {
                3
            } else if leads < half.wrapping_mul(2) / 3 {
                2
            } else {
                1
            };
            flags |= itu::DBCS_LEADBYTE;
        }
    }
    if k.dhi.wrapping_mul(weight) < k.dlo {
        flags |= itu::STATISTICS;
    }
    if k.dlo.wrapping_mul(weight) < k.dhi {
        flags |= itu::REVERSE_STATISTICS;
    }
    let sum = |v: &[u32]| v.iter().fold(0u32, |a, &x| a.wrapping_add(x));
    if sum(&[k.space, k.ideo_space, k.tab, k.lf, k.cr]) != 0 {
        flags |= itu::CONTROLS;
    }
    if sum(&[k.rtab, k.rspace, k.rlf, k.rcr]) != 0 {
        flags |= itu::REVERSE_CONTROLS;
    }
    if sum(&[k.rcrlf, k.nul, k.rbom, k.ffff]) != 0 || (k.ansi_breaks != 0 && k.ansi_breaks >= window / 40) {
        flags |= itu::ILLEGAL_CHARS;
    }
    if length & 1 != 0 {
        flags |= itu::ODD_LENGTH;
    }
    if k.zero_bytes != 0 {
        flags |= itu::NULL_BYTES;
    }
    match unit(0) {
        0xfeff => flags |= itu::SIGNATURE,
        0xfffe => flags |= itu::REVERSE_SIGNATURE,
        _ => {}
    }
    let r = mask.map_or(flags, |m| m & flags);
    let yes = r & 0xb08 == itu::SIGNATURE || (r & 0xff0 == 0 && r & 0xf00f != 0);
    (yes, r)
}

fn is_text_unicode_h(c: &mut Ctx) -> u32 {
    let (buf, length, pf) = (c.a(0), c.a(1).cast_signed(), c.a(2));
    let length = u32::try_from(length).unwrap_or(0);
    let bytes: Vec<u8> = (0..length.min(0x200)).map(|i| mem::r8(mem::ga(buf) + i)).collect();
    let cp = crate::text::acp();
    let lead = |b: u8| crate::text::is_lead_cp(cp, b);
    let dbcs = cp != 65001 && (0u8..=255).any(lead);
    let mask = (pf != 0).then(|| mem::r32(pf));
    let (yes, flags) = text_unicode(&bytes, length, if dbcs { Some(&lead) } else { None }, mask);
    if pf != 0 {
        mem::w32(pf, flags);
    }
    u32::from(yes)
}

pub fn init() {
    let d = "ntdll";
    implement(d, "RtlInitUnicodeStringEx", init_ex_h);
    implement(d, "RtlAppendAsciizToString", append_asciiz_h);
    implement(d, "RtlAppendStringToString", append_string_h);
    implement(d, "RtlAppendUnicodeStringToString", append_unicode_string_h);
    implement(d, "RtlAppendUnicodeToString", append_unicode_h);
    implement(d, "RtlCopyString", copy_string_h);
    implement(d, "RtlUpperChar", upper_char_h);
    implement(d, "RtlUpperString", upper_string_h);
    implement(d, "RtlDowncaseUnicodeChar", |c: &mut Ctx| u32::from(crate::nt::down(kome::num::lo16(c.a(0)))));
    implement(d, "RtlUpcaseUnicodeString", |c: &mut Ctx| case_h(c, Case::Up));
    implement(d, "RtlDowncaseUnicodeString", |c: &mut Ctx| case_h(c, Case::Down));
    implement(d, "RtlDuplicateUnicodeString", duplicate_h);
    implement(d, "RtlFindCharInUnicodeString", find_h);
    implement(d, "RtlCompareUnicodeStrings", compare_units_h);
    implement(d, "RtlGUIDFromString", guid_from_h);
    implement(d, "RtlStringFromGUID", guid_text_h);
    implement(d, "RtlUTF8ToUnicodeN", utf8_to_unicode_h);
    implement(d, "RtlUnicodeToUTF8N", unicode_to_utf8_h);
    implement(d, "RtlHashUnicodeString", hash_h);
    implement(d, "RtlUnicodeStringToAnsiString", unicode_to_ansi_h);
    implement(d, "RtlIsTextUnicode", is_text_unicode_h);
    implement("advapi32", "IsTextUnicode", is_text_unicode_h);
}

#[cfg(test)]
#[path = "rtlstr_tests.rs"]
mod tests;
