#![allow(non_snake_case)]
//! ntdll's message formatter, RtlFormatMessage(Ex), which kernel32's FormatMessage is made of
//! (clean-room rules: sake-re/ntdll/rtl-format-message.md; checked by Wine's ntdll:rtlstr and
//! kernel32:format_msg). [`format`] is the whole rule over the message, the arguments and the
//! room in the buffer: what the buffer holds afterwards, the status and the returned length.

use crate::api::{implement, Ctx};
use crate::mem;
use crate::ntstr::Status;
use std::num::NonZeroU32;

const NUL: u16 = 0;
const TAB: u16 = 9;
const LF: u16 = 10;
const CR: u16 = 13;
const SPACE: u16 = 0x20;
const BANG: u16 = 0x21;
const PERCENT: u16 = 0x25;
const STAR: u16 = 0x2a;
const ZERO: u16 = 0x30;

/// the argument cache's size: an insert number plus its stars must stay below it
const SLOTS: usize = 200;
/// the longest spec between the '!'s, with its leading '%' (31 units of the 32-unit local)
const SPEC_UNITS: usize = 31;

/// MaxWidth: 0 keeps line breaks (CR LF); 0xFFFFFFFF makes them spaces; any other value also
/// wraps lines at that column
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    Keep,
    Spaces,
    Wrap(NonZeroU32),
}

impl Width {
    pub const fn of(v: u32) -> Width {
        match NonZeroU32::new(v) {
            None => Width::Keep,
            Some(_) if v == u32::MAX => Width::Spaces,
            Some(w) => Width::Wrap(w),
        }
    }
}

/// how the arguments are passed: an array of ULONG_PTRs, or a va_list
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgList {
    Array,
    VaList,
}

/// Where the arguments come from, and the strings they point to (guest memory, or a test's).
pub trait Guest: crt::fmt::Args {
    /// the list's next argument: an array slot (pointer-sized), or the va_list's next 4 bytes
    /// (x86 without `eight`) or 8
    fn next(&mut self, list: ArgList, eight: bool) -> u64;
}

/// one call
pub struct Request<'a> {
    /// the message up to (not with) its 0
    pub message: &'a [u16],
    pub width: Width,
    pub ignore_inserts: bool,
    /// the string and character arguments are ANSI
    pub ansi: bool,
    /// None: Arguments is NULL
    pub args: Option<ArgList>,
    /// the buffer's size in units (BufferSize / 2)
    pub room: u32,
    pub ptr64: bool,
}

/// what a call leaves: the buffer's units from its start (what was written, a 0 printf put
/// after an insert included), the status and *ReturnLength (bytes with the 0; only on success)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub buffer: Vec<u16>,
    pub status: Status,
    pub length: Option<u32>,
}

impl Outcome {
    /// the message's units (success), without the 0
    pub fn text(&self) -> Option<&[u16]> {
        let n = kome::num::us(self.length?.checked_div(2)?.checked_sub(1)?);
        self.buffer.get(..n)
    }
}

/// the buffer as it is written: units at `q` on, `room` units left (negative: overflow)
struct Out {
    buf: Vec<u16>,
    q: usize,
    room: i64,
}

impl Out {
    fn write_at(&mut self, at: usize, units: &[u16]) {
        for (k, &u) in units.iter().enumerate() {
            let i = at.saturating_add(k);
            match self.buf.get_mut(i) {
                Some(x) => *x = u,
                None => self.buf.push(u),
            }
        }
    }
    /// takes `n` units of room: Err when it goes negative (nothing written)
    fn take(&mut self, n: usize) -> Result<(), Status> {
        self.room = self.room.saturating_sub(i64::try_from(n).unwrap_or(i64::MAX));
        if self.room < 0 { Err(Status::BufferOverflow) } else { Ok(()) }
    }
    fn put(&mut self, units: &[u16]) -> Result<(), Status> {
        self.take(units.len())?;
        self.write_at(self.q, units);
        self.q = self.q.saturating_add(units.len());
        Ok(())
    }
    /// RtlStringCchPrintfExW(q, room): no room, or the text not fitting with its 0 (then
    /// room - 1 units and a 0 are written): overflow; else the text and a 0 after it
    fn printed(&mut self, t: &[u16]) -> Result<(), Status> {
        let room = usize::try_from(self.room).unwrap_or(0);
        let Some(fits) = room.checked_sub(1) else { return Err(Status::BufferOverflow) };
        if t.len() > fits {
            let cut = t.get(..fits).unwrap_or_default();
            self.write_at(self.q, cut);
            self.write_at(self.q.saturating_add(cut.len()), &[NUL]);
            return Err(Status::BufferOverflow);
        }
        self.write_at(self.q, t);
        self.write_at(self.q.saturating_add(t.len()), &[NUL]);
        self.take(t.len())?;
        self.q = self.q.saturating_add(t.len());
        Ok(())
    }
}

const fn blank(u: u16) -> bool {
    u == SPACE || u == TAB
}

fn digit(u: u16) -> Option<u32> {
    match u {
        0x30..=0x39 => Some(u32::from(u.wrapping_sub(ZERO))),
        _ => None,
    }
}

/// ArgumentsAreAnsi: the spec's last unit c / s become hc / hs (unless h, w or l is before
/// them), S becomes s and C c
fn ansi_spec(spec: &mut Vec<u16>) {
    let n = spec.len();
    let last = spec.last().copied().unwrap_or(0);
    let before = n.checked_sub(2).and_then(|i| spec.get(i)).copied().unwrap_or(0);
    let sized = matches!(before, 0x68 | 0x77 | 0x6c);
    match last {
        0x63 | 0x73 if !sized => {
            spec.pop();
            spec.extend([0x68, last]);
        }
        0x53 => {
            spec.pop();
            spec.push(0x73);
        }
        0x43 => {
            spec.pop();
            spec.push(0x63);
        }
        _ => {}
    }
}

/// printf's arguments for one insert: the value first, then the stars' (as the function pushes
/// them), each a 4-byte word on x86 and an 8-byte one on x64; strings from the guest
struct Queue<'g> {
    words: Vec<u64>,
    at: usize,
    ptr64: bool,
    g: &'g mut dyn Guest,
}

impl Queue<'_> {
    fn word(&mut self) -> u64 {
        let w = self.words.get(self.at).copied().unwrap_or(0);
        self.at = self.at.saturating_add(1);
        w
    }
}

impl crt::fmt::Args for Queue<'_> {
    fn int(&mut self, bytes: u32) -> u64 {
        if bytes <= 4 {
            return self.word() & 0xffff_ffff;
        }
        if self.ptr64 {
            return self.word();
        }
        let lo = self.word() & 0xffff_ffff;
        lo | self.word() << 32
    }
    fn float(&mut self) -> f64 {
        f64::from_bits(self.int(8))
    }
    fn narrow(&mut self, p: u64, max: Option<usize>) -> Vec<u8> {
        self.g.narrow(p, max)
    }
    fn wide(&mut self, p: u64, max: Option<usize>) -> Vec<u16> {
        self.g.wide(p, max)
    }
    fn counted(&mut self, p: u64) -> (usize, u64) {
        self.g.counted(p)
    }
    fn a2w(&self, s: &[u8]) -> Vec<u16> {
        self.g.a2w(s)
    }
    fn w2a(&self, s: &[u16]) -> Vec<u8> {
        self.g.w2a(s)
    }
    fn store(&mut self, _: u64, _: u32, _: u64) {}
}

struct Engine<'r> {
    r: &'r Request<'r>,
    out: Out,
    col: u32,
    /// where the last ' ' that a line may break at was written
    last: Option<usize>,
    /// the arguments read so far, by slot
    cache: Vec<u64>,
}

impl Engine<'_> {
    fn at(&self, k: usize) -> u16 {
        self.r.message.get(k).copied().unwrap_or(NUL)
    }
    fn add_col(&mut self, n: usize) {
        self.col = self.col.wrapping_add(kome::num::u32_of(n));
    }
    fn unit(&mut self, c: u16) -> Result<(), Status> {
        let q = self.out.q;
        self.out.put(&[c])?;
        if c == SPACE {
            self.last = Some(q);
        }
        self.add_col(1);
        Ok(())
    }
    fn line_break(&mut self) -> Result<(), Status> {
        self.out.put(&[CR, LF])?;
        self.col = 0;
        self.last = None;
        Ok(())
    }
    /// at or past the width: a break at the last space's run of blanks, or here
    fn wrap(&mut self) -> Result<(), Status> {
        let Width::Wrap(w) = self.r.width else { return Ok(()) };
        if self.col < w.get() {
            return Ok(());
        }
        let Some(ls) = self.last else { return self.line_break() };
        let q = self.out.q;
        let mut fwd = ls;
        while fwd < q && self.out.buf.get(fwd).copied().is_some_and(blank) {
            fwd = fwd.saturating_add(1);
        }
        let mut back = ls;
        while let Some(b) = back.checked_sub(1)
            && self.out.buf.get(b).copied().is_some_and(blank)
        {
            back = b;
        }
        let n = fwd.saturating_sub(back);
        if n == 1 {
            self.out.take(1)?;
        } else if let Some(extra) = n.checked_sub(2) {
            self.out.room = self.out.room.saturating_add(i64::try_from(extra).unwrap_or(0));
        }
        let tail: Vec<u16> = self.out.buf.get(fwd..q).unwrap_or_default().to_vec();
        self.out.write_at(back, &[CR, LF]);
        self.out.write_at(back.saturating_add(2), &tail);
        self.out.q = back.saturating_add(2).saturating_add(tail.len());
        self.col = kome::num::u32_of(tail.len());
        self.last = None;
        Ok(())
    }
    fn finish(&mut self) -> Result<u32, Status> {
        if self.out.room < 1 {
            return Err(Status::BufferOverflow);
        }
        self.out.write_at(self.out.q, &[NUL]);
        Ok(kome::num::u32_of(self.out.q).saturating_mul(2).saturating_add(2))
    }
    fn run(&mut self, g: &mut dyn Guest) -> Result<u32, Status> {
        let mut p = 0usize;
        loop {
            let c = self.at(p);
            if c == NUL {
                return self.finish();
            }
            p = p.saturating_add(1);
            match c {
                PERCENT => {
                    let start = self.out.q;
                    let d = self.at(p);
                    if d.wrapping_sub(0x31) <= 8 {
                        p = self.insert(g, p)?;
                    } else {
                        match d {
                            ZERO => return self.finish(),
                            NUL => return Err(Status::InvalidParameter),
                            // %r %n: CR, CR LF, a new line
                            0x72 | 0x6e => {
                                self.out.put(if d == 0x72 { &[CR] } else { &[CR, LF] })?;
                                self.col = 0;
                                self.last = None;
                                self.wrap()?;
                                p = p.saturating_add(1);
                                continue;
                            }
                            // %t: to the next tab stop (and the +1 below), a break may come there
                            0x74 => {
                                self.out.take(1)?;
                                self.col = if self.col & 7 != 0 { self.col.wrapping_add(7) & !7 } else { self.col.wrapping_add(8) };
                                self.last = Some(self.out.q);
                                self.out.write_at(self.out.q, &[TAB]);
                                self.out.q = self.out.q.saturating_add(1);
                            }
                            // %b: a space a line may break at
                            0x62 => {
                                self.out.take(1)?;
                                self.last = Some(self.out.q);
                                self.out.write_at(self.out.q, &[SPACE]);
                                self.out.q = self.out.q.saturating_add(1);
                            }
                            _ if self.r.ignore_inserts => self.out.put(&[PERCENT, d])?,
                            _ => self.out.put(&[d])?,
                        }
                        p = p.saturating_add(1);
                    }
                    self.add_col(self.out.q.saturating_sub(start));
                }
                CR | LF => {
                    let pair = if c == CR { LF } else { CR };
                    if self.at(p) == pair {
                        p = p.saturating_add(1);
                    }
                    if self.r.width == Width::Keep {
                        self.line_break()?;
                        continue;
                    }
                    self.unit(SPACE)?;
                }
                _ => self.unit(c)?,
            }
            if self.r.width != Width::Keep {
                self.wrap()?;
            }
        }
    }
    /// %n[!spec!] from the first digit at `p`: where the message goes on
    fn insert(&mut self, g: &mut dyn Guest, mut p: usize) -> Result<usize, Status> {
        let mut n = digit(self.at(p)).unwrap_or(0);
        p = p.saturating_add(1);
        if let Some(d) = digit(self.at(p)) {
            n = n.wrapping_mul(10).wrapping_add(d);
            p = p.saturating_add(1);
            if let Some(d) = digit(self.at(p)) {
                n = n.wrapping_mul(10).wrapping_add(d);
                p = p.saturating_add(1);
                if digit(self.at(p)).is_some() {
                    return Err(Status::InvalidParameter);
                }
            }
        }
        let mut stars = 0usize;
        let (mut spec, bare): (Vec<u16>, bool) = if self.at(p) == BANG {
            p = p.saturating_add(1);
            let mut spec = vec![PERCENT];
            loop {
                let c = self.at(p);
                if c == BANG {
                    break;
                }
                if c == NUL || spec.len() >= SPEC_UNITS {
                    return Err(Status::InvalidParameter);
                }
                if c == STAR {
                    if stars > 1 {
                        return Err(Status::InvalidParameter);
                    }
                    stars = stars.saturating_add(1);
                }
                spec.push(c);
                p = p.saturating_add(1);
            }
            p = p.saturating_add(1);
            (spec, false)
        } else {
            (vec![PERCENT, 0x73], true)
        };
        if self.r.ignore_inserts {
            let text = if bare || spec == [PERCENT, 0x73] {
                format!("%{n}")
            } else {
                format!("%{n}!{}!", String::from_utf16_lossy(spec.get(1..).unwrap_or_default()))
            };
            let t: Vec<u16> = text.encode_utf16().collect();
            self.out.printed(&t)?;
            return Ok(p);
        }
        let Some(list) = self.r.args else { return Err(Status::InvalidParameter) };
        let k = kome::num::us(n.saturating_sub(1));
        if k.saturating_add(stars) >= SLOTS {
            return Err(Status::InvalidParameter);
        }
        let i64 = spec.get(1..).unwrap_or_default().windows(3).any(|w| w == [0x49, 0x36, 0x34]);
        if self.r.ansi {
            ansi_spec(&mut spec);
        }
        while self.cache.len() <= k {
            self.cache.push(g.next(list, i64 && list == ArgList::VaList));
        }
        let v = self.cache.get(k).copied().unwrap_or(0);
        let mut star = [0u64; 2];
        for (i, s) in star.iter_mut().enumerate().take(stars) {
            *s = g.next(list, false);
            // (a va_list's first star is not kept: the next slot is the argument after it)
            if i == 1 || list == ArgList::Array {
                self.cache.push(*s);
            }
        }
        let mut words = if i64 && !self.r.ptr64 { vec![v & 0xffff_ffff, v >> 32] } else { vec![v] };
        words.extend(star);
        let ptr64 = self.r.ptr64;
        let text = crt::fmt::format(&spec, crt::fmt::Mode::legacy(ptr64), &mut Queue { words, at: 0, ptr64, g });
        match text {
            Ok(t) => self.out.printed(&t)?,
            Err(_) => return Err(Status::BufferOverflow),
        }
        Ok(p)
    }
}

/// RtlFormatMessage's rule
pub fn format(r: &Request, g: &mut dyn Guest) -> Outcome {
    let mut e = Engine { r, out: Out { buf: Vec::new(), q: 0, room: i64::from(r.room) }, col: 0, last: None, cache: Vec::new() };
    let res = e.run(g);
    let buffer = e.out.buf;
    match res {
        Ok(n) => Outcome { buffer, status: Status::Success, length: Some(n) },
        Err(status) => Outcome { buffer, status, length: None },
    }
}

/// the arguments in guest memory: an array, or a va_list whose variable is kept up to date
pub struct MemGuest {
    at: u32,
    /// the va_list variable (Arguments of a va_list call)
    va: Option<u32>,
    strings: crate::misc::VaArgs,
}

impl MemGuest {
    /// `arguments`: the ULONG_PTR array, or the va_list * (`list`)
    pub fn new(list: ArgList, arguments: u32) -> MemGuest {
        match list {
            ArgList::Array => MemGuest { at: arguments, va: None, strings: crate::misc::VaArgs::stack(0) },
            ArgList::VaList => MemGuest { at: mem::rp(arguments), va: Some(arguments), strings: crate::misc::VaArgs::stack(0) },
        }
    }
}

impl Guest for MemGuest {
    fn next(&mut self, list: ArgList, eight: bool) -> u64 {
        let x64 = mem::x64();
        let wide = x64 || (eight && list == ArgList::VaList);
        let v = if !wide {
            u64::from(mem::r32(self.at))
        } else if x64 {
            mem::r64(self.at)
        } else {
            u64::from(mem::r32(self.at)) | u64::from(mem::r32(mem::ga(self.at) + 4)) << 32
        };
        self.at = self.at.wrapping_add(if wide { 8 } else { 4 });
        if let Some(va) = self.va {
            mem::wp(va, self.at);
        }
        v
    }
}

impl crt::fmt::Args for MemGuest {
    fn int(&mut self, _: u32) -> u64 {
        0
    }
    fn float(&mut self) -> f64 {
        0.0
    }
    fn narrow(&mut self, p: u64, max: Option<usize>) -> Vec<u8> {
        crate::misc::GuestArgs(&mut self.strings).narrow(p, max)
    }
    fn wide(&mut self, p: u64, max: Option<usize>) -> Vec<u16> {
        crate::misc::GuestArgs(&mut self.strings).wide(p, max)
    }
    fn counted(&mut self, p: u64) -> (usize, u64) {
        crate::misc::GuestArgs(&mut self.strings).counted(p)
    }
    fn a2w(&self, s: &[u8]) -> Vec<u16> {
        crate::text::a2w(s)
    }
    fn w2a(&self, s: &[u16]) -> Vec<u8> {
        crate::text::w2a(s)
    }
    fn store(&mut self, _: u64, _: u32, _: u64) {}
}

/// RtlFormatMessageEx(Message, MaxWidth, IgnoreInserts, ArgumentsAreAnsi, ArgumentsAreAnArray,
/// Arguments, Buffer, BufferSize, ReturnLength, Flags); RtlFormatMessage is it without the last
fn RtlFormatMessageEx(c: &mut Ctx) -> u32 {
    let (msg, width, ignore, ansi, array, args) = (c.a(0), c.a(1), c.a(2) & 0xff != 0, c.a(3) & 0xff != 0, c.a(4) & 0xff != 0, c.a(5));
    let (buf, size, ret) = (c.a(6), c.a(7), c.a(8));
    let message = mem::wstr(msg);
    let list = if array { ArgList::Array } else { ArgList::VaList };
    let present = (args != 0).then_some(list);
    let r = Request { message: &message, width: Width::of(width), ignore_inserts: ignore, ansi, args: present, room: size >> 1, ptr64: mem::x64() };
    let mut g = MemGuest::new(if args == 0 { ArgList::Array } else { list }, args);
    let o = format(&r, &mut g);
    for (k, &u) in o.buffer.iter().enumerate() {
        mem::w16(mem::ga(buf).at(kome::num::u32_of(k), 2), u);
    }
    if let Some(n) = o.length
        && ret != 0
    {
        mem::w32(ret, n);
    }
    o.status.code()
}

pub fn init() {
    implement("ntdll", "RtlFormatMessage", RtlFormatMessageEx);
    implement("ntdll", "RtlFormatMessageEx", RtlFormatMessageEx);
}

#[cfg(test)]
#[path = "rtlfmt_tests.rs"]
pub(crate) mod tests;
