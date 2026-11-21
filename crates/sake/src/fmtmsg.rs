//! kernel32 FormatMessageA/W as Windows does it (winref fmt): the message from a string, a
//! module's message table or the system's (a small table of common errors), inserts %1..%99
//! with printf specs (!d!, !5s!, !*d!...), the escapes %n %r %t %0 %% and %<other>, the width
//! modes of FORMAT_MESSAGE_MAX_WIDTH_MASK, va_list or ARGUMENT_ARRAY arguments,
//! IGNORE_INSERTS, ALLOCATE_BUFFER and the buffer-too-small error.

use kome::locks::Rw as _;
use crate::api::Ctx;
use crate::mem::{self, r16, r32};
use crate::ntstr::Status;
use crate::rtlfmt;
use crate::text::{a2w, w2a};

const ALLOCATE_BUFFER: u32 = 0x100;
const IGNORE_INSERTS: u32 = 0x200;
const FROM_STRING: u32 = 0x400;
const FROM_HMODULE: u32 = 0x800;
const FROM_SYSTEM: u32 = 0x1000;
const ARGUMENT_ARRAY: u32 = 0x2000;

/// Common system messages (English, as the runner's).
fn system_message(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "The operation completed successfully.\r\n",
        1 => "Incorrect function.\r\n",
        2 => "The system cannot find the file specified.\r\n",
        3 => "The system cannot find the path specified.\r\n",
        4 => "The system cannot open the file.\r\n",
        5 => "Access is denied.\r\n",
        6 => "The handle is invalid.\r\n",
        7 => "The storage control blocks were destroyed.\r\n",
        8 => "Not enough memory resources are available to process this command.\r\n",
        13 => "The data is invalid.\r\n",
        14 => "Not enough memory resources are available to complete this operation.\r\n",
        15 => "The system cannot find the drive specified.\r\n",
        18 => "There are no more files.\r\n",
        21 => "The device is not ready.\r\n",
        32 => "The process cannot access the file because it is being used by another process.\r\n",
        38 => "Reached the end of the file.\r\n",
        50 => "The request is not supported.\r\n",
        80 => "The file exists.\r\n",
        87 => "The parameter is incorrect.\r\n",
        109 => "The pipe has been ended.\r\n",
        111 => "The file name is too long.\r\n",
        112 => "There is not enough space on the disk.\r\n",
        120 => "This function is not supported on this system.\r\n",
        122 => "The data area passed to a system call is too small.\r\n",
        123 => "The filename, directory name, or volume label syntax is incorrect.\r\n",
        126 => "The specified module could not be found.\r\n",
        127 => "The specified procedure could not be found.\r\n",
        145 => "The directory is not empty.\r\n",
        183 => "Cannot create a file when that file already exists.\r\n",
        193 => "%1 is not a valid Win32 application.\r\n",
        203 => "The system could not find the environment option that was entered.\r\n",
        234 => "More data is available.\r\n",
        258 => "The wait operation timed out.\r\n",
        259 => "No more data is available.\r\n",
        267 => "The directory name is invalid.\r\n",
        317 => "The system cannot find message text for message number 0x%1 in the message file for %2.\r\n",
        487 => "Attempt to access invalid address.\r\n",
        998 => "Invalid access to memory location.\r\n",
        1114 => "A dynamic link library (DLL) initialization routine failed.\r\n",
        1400 => "Invalid window handle.\r\n",
        1460 => "This operation returned because the timeout period expired.\r\n",
        1813 => "The specified resource type cannot be found in the image file.\r\n",
        1814 => "The specified resource name cannot be found in the image file.\r\n",
        1815 => "The specified resource language ID cannot be found in the image file.\r\n",
        // (STG_E_FILENOTFOUND: RPG Maker VX's Game.exe names a missing RGSS dll with it, winref fmt)
        0x8003_0002 => "%1 could not be found.\r\n",
        0x8000_4001 => "Not implemented\r\n",
        0x8000_4002 => "No such interface supported\r\n",
        0x8000_4005 => "Unspecified error\r\n",
        0x8007_000e => "Not enough memory resources are available to complete this operation.\r\n",
        0x8007_0057 => "The parameter is incorrect.\r\n",
        0x8876_086c => "Invalid call\r\n",
        0x800b_0100 => "No signature was present in the subject.\r\n",
        0x8009_2009 => "Cannot find object or property.\r\n",
        // (an HRESULT of FACILITY_WIN32 has the Win32 code's text: E_ACCESSDENIED is "Access is denied.")
        h if h >> 16 == 0x8007 => return system_message(h & 0xffff),
        _ => return None,
    })
}

/// A module's RT_MESSAGETABLE entry for `id`: MESSAGE_RESOURCE_DATA {blocks, {low, high,
/// offset}...}, entries {length, flags (1: UTF-16), text}.
fn table_message(module: u32, id: u32) -> Option<Vec<u16>> {
    let m = if module == 0 { crate::pe::MODULES.r().iter().find(|m| !m.dll).map_or(0, |m| m.base) } else { module };
    let e = crate::pe::find_resource(m, &crate::pe::ResId::Int(11), &crate::pe::ResId::Int(1))?;
    let (p, size) = crate::pe::resource_data(m, e);
    let blocks = r32(p);
    for b in 0..blocks.min(4096) {
        let blk = p + 4 + 12 * b;
        let (lo, hi, off) = (r32(blk), r32(blk + 4), r32(blk + 8));
        if !(lo..=hi).contains(&id) {
            continue;
        }
        // (entries of a table that is not well formed: one of length 0, or past the resource, ends it)
        let end = p as u64 + size as u64;
        let mut q = p as u64 + off as u64;
        for _ in lo..id {
            if q + 4 > end {
                return None;
            }
            match r16(q as u32) {
                0 => return None,
                n => q += n as u64,
            }
        }
        if q + 4 > end {
            return None;
        }
        let q = q as u32;
        let (len, flags) = ((r16(q) as u32).min((end - q as u64) as u32), r16(q + 2));
        let body = q + 4;
        let text: Vec<u16> = if flags & 1 != 0 {
            (0..(len.saturating_sub(4)) / 2).map(|i| r16(body + 2 * i)).take_while(|&c| c != 0).collect()
        } else {
            a2w(&mem::bytes(body, len.saturating_sub(4)).iter().copied().take_while(|&c| c != 0).collect::<Vec<u8>>())
        };
        return Some(text);
    }
    None
}

/// FormatMessage(flags, source, id, language, buffer, size, va_list *args)
pub fn format_message(c: &mut Ctx, wide: bool) -> u32 {
    let (flags, source, id, lang, buf, size, va) = (c.a(0), c.a(1), c.a(2), c.a(3), c.a(4), c.a(5), c.a(6));
    let fail = |e: u32| {
        crate::k32::last_error(e);
        0
    };
    // (FormatMessageA: a source string of 0x7FFF bytes or more is refused, and the allocated
    // buffer's pointer must be there; FormatMessageW: any buffer argument must be)
    if !wide && flags & FROM_STRING != 0 && source != 0 && mem::cstr(source).len() >= 0x7fff {
        return fail(87);
    }
    if buf == 0 {
        if wide {
            return fail(87);
        }
        if flags & ALLOCATE_BUFFER != 0 {
            return fail(8);
        }
    }
    if flags & ALLOCATE_BUFFER != 0 {
        mem::wp(buf, 0);
    }
    let src: Vec<u16> = if flags & FROM_STRING != 0 {
        if source == 0 {
            return fail(87);
        }
        if wide { mem::wstr(source) } else { a2w(&mem::cstr(source)) }
    } else if flags & (FROM_HMODULE | FROM_SYSTEM) != 0 {
        // a language other than neutral, English or Japanese: no such resource
        let primary = lang & 0x3ff;
        if lang != 0 && !matches!(primary, 0 | 0x09 | 0x11) {
            return fail(15100);
        }
        let from_module = if flags & FROM_HMODULE != 0 { table_message(source, id) } else { None };
        match from_module.or_else(|| if flags & FROM_SYSTEM != 0 { system_message(id).map(|s| s.encode_utf16().collect()) } else { None }) {
            Some(t) => t,
            None => return fail(if flags & FROM_SYSTEM != 0 { 317 } else { 1813 }),
        }
    } else {
        return fail(87);
    };
    let text = match format(&src, flags, va, wide, if flags & ALLOCATE_BUFFER != 0 { None } else { Some(size) }) {
        Ok(t) => t,
        Err((e, partial)) => {
            // (not allocating: what the formatter wrote stays in the caller's buffer, ending
            // in a 0 at its last unit)
            if wide && flags & ALLOCATE_BUFFER == 0 && e == 122 {
                for (k, &u) in partial.iter().take(kome::num::us(size)).enumerate() {
                    mem::w16(mem::ga(buf).at(kome::num::u32_of(k), 2), u);
                }
                if let Some(last) = size.checked_sub(1) {
                    mem::w16(mem::ga(buf).at(last, 2), 0);
                }
            }
            return fail(e);
        }
    };
    // (nothing formatted: ERROR_NO_WORK_DONE; FormatMessageW's buffer has the formatter's 0)
    if text.is_empty() {
        if wide && flags & ALLOCATE_BUFFER == 0 && size != 0 {
            mem::w16(buf, 0);
        }
        return fail(235);
    }
    if !wide && text.len() >= 0x7fff {
        return fail(87);
    }
    let bytes: Vec<u8> = if wide { Vec::new() } else { w2a(&text) };
    let n = if wide { text.len() } else { bytes.len() } as u32;
    let unit = if wide { 2 } else { 1 };
    let dst = if flags & ALLOCATE_BUFFER != 0 {
        let p = mem::heap_alloc((n + 1).max(size.saturating_add(1)) * unit, true);
        mem::wp(buf, p);
        p
    } else {
        if n + 1 > size {
            return fail(122);
        }
        buf
    };
    if wide {
        for (k, &u) in text.iter().enumerate() {
            mem::w16(dst + 2 * k as u32, u);
        }
        mem::w16(dst + 2 * n, 0);
    } else {
        mem::write(dst, &bytes);
        mem::w8(dst + n, 0);
    }
    n
}

/// The formatted message (RtlFormatMessageEx as kernelbase calls it), or the Win32 error and
/// what the formatter left in the buffer. `room`: the caller's buffer in units (None:
/// ALLOCATE_BUFFER, which grows the buffer up to the source's size rounded up to 128 KiB).
fn format(src: &[u16], flags: u32, va: u32, wide: bool, room: Option<u32>) -> Result<Vec<u16>, (u32, Vec<u16>)> {
    let list = if flags & ARGUMENT_ARRAY != 0 { rtlfmt::ArgList::Array } else { rtlfmt::ArgList::VaList };
    let mut g = rtlfmt::MemGuest::new(if va == 0 { rtlfmt::ArgList::Array } else { list }, va);
    let r = request(src, flags, (va != 0).then_some(list), wide, room);
    outcome(rtlfmt::format(&r, &mut g))
}

fn request(src: &[u16], flags: u32, args: Option<rtlfmt::ArgList>, wide: bool, room: Option<u32>) -> rtlfmt::Request<'_> {
    let width = match flags & 0xff {
        0xff => u32::MAX,
        w => w,
    };
    let room = room.unwrap_or_else(|| {
        let len = kome::num::u32_of(src.len());
        let start = if flags & IGNORE_INSERTS != 0 { len } else { len.saturating_mul(4) };
        let bytes = start.saturating_mul(2).saturating_add(2);
        (bytes.saturating_add(0x1_ffff) & !0x1_ffff) >> 1
    });
    rtlfmt::Request { message: src, width: rtlfmt::Width::of(width), ignore_inserts: flags & IGNORE_INSERTS != 0, ansi: !wide, args, room, ptr64: mem::x64() }
}

fn outcome(o: rtlfmt::Outcome) -> Result<Vec<u16>, (u32, Vec<u16>)> {
    // (kernelbase: an overflow is ERROR_INSUFFICIENT_BUFFER, the rest RtlNtStatusToDosError's)
    if o.status == Status::Success {
        Ok(o.text().unwrap_or_default().to_vec())
    } else if o.status == Status::BufferOverflow {
        Err((122, o.buffer))
    } else {
        Err((87, o.buffer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(s: &str, flags: u32) -> Result<String, u32> {
        let w: Vec<u16> = s.encode_utf16().collect();
        let mut g = crate::rtlfmt::tests::Words::new(&[]);
        outcome(rtlfmt::format(&request(&w, flags, None, true, None), &mut g)).map(|v| String::from_utf16_lossy(&v)).map_err(|e| e.0)
    }

    /// The answers of Windows (tests/winref/golden/fmt-32) for messages without inserts.
    #[test]
    fn escapes_and_widths_follow_windows() {
        assert_eq!(f("line1%nline2%r%t|%0 after zero", 0), Ok("line1\r\nline2\r\t|".into()));
        assert_eq!(f("percent %% dot %. bang %!", 0), Ok("percent % dot . bang !".into()));
        assert_eq!(f("end\r\n", 0), Ok("end\r\n".into()));
        assert_eq!(f("trailing %", 0), Err(87));
        assert_eq!(f("%1", IGNORE_INSERTS), Ok("%1".into()));
        assert_eq!(f("%1 and %2!d!", IGNORE_INSERTS), Ok("%1 and %2!d!".into()));
        assert_eq!(
            f("the quick brown fox jumps over the lazy dog again and again", 20),
            Ok("the quick brown fox\r\njumps over the lazy\r\ndog again and again".into())
        );
        assert_eq!(f("the quick brown fox\r\njumps", 0xff), Ok("the quick brown fox jumps".into()));
        assert_eq!(f("  spaced   words  ", 0xff), Ok("  spaced   words  ".into()));
        assert_eq!(f("verylongwordwithoutanyspaces here", 10), Ok("verylongwo\r\nrdwithouta\r\nnyspaces\r\nhere".into()));
    }

    /// RPG Maker VX's Game.exe (RGSS2 Player) shows FormatMessageW(FROM_SYSTEM | ARGUMENT_ARRAY,
    /// STG_E_FILENOTFOUND, [dll]) when RGSS202J.dll is missing; without the message the box was
    /// empty. Windows (tests/winref/probes/fmt.c "stg filenotfound") has it with the name in it.
    #[test]
    fn stg_file_not_found_names_the_file() {
        let t = system_message(0x8003_0002).expect("STG_E_FILENOTFOUND");
        assert!(t.contains("%1") && t.ends_with("\r\n"), "{t:?}");
        assert_eq!(f(t, IGNORE_INSERTS), Ok(t.into()));
    }

    proptest::proptest! {
        /// Every system message (Win32 codes and the HRESULTs sake knows) formats as Windows'
        /// do: one line ending in CRLF, unchanged with IGNORE_INSERTS, and (without inserts) unchanged
        /// with them too.
        #[test]
        fn system_messages_format(id in proptest::prop_oneof![0u32..20000, 0x8000_0000u32..0x8000_ffff, 0x8003_0000u32..0x8003_0100, 0x8007_0000u32..0x8007_ffff, 0x8876_0000u32..0x8876_ffff]) {
            if let Some(t) = system_message(id) {
                proptest::prop_assert!(t.ends_with("\r\n") && !t[..t.len() - 2].contains('\n'), "{id:#x}: {t:?}");
                proptest::prop_assert_eq!(f(t, IGNORE_INSERTS), Ok(t.to_string()));
                if !t.contains('%') {
                    proptest::prop_assert_eq!(f(t, 0), Ok(t.to_string()));
                }
            }
        }
    }

    /// Whatever the text and width, no line of a width-limited message is longer than the width
    /// and only spaces and line breaks go missing.
    #[test]
    fn width_limits_every_line() {
        let mut seed = 12345u32;
        for _ in 0..2000 {
            let mut s = String::new();
            for _ in 0..(seed % 60) {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                s.push(match (seed >> 16) % 5 {
                    0 => ' ',
                    1 => 'a',
                    2 => 'b',
                    3 => 'c',
                    _ => 'd',
                });
            }
            let w = 1 + (seed >> 8) % 30;
            let out = f(&s, w).unwrap();
            for line in out.split("\r\n") {
                assert!(line.chars().count() <= w as usize, "{s:?} width {w}: {out:?}");
            }
            let strip = |t: &str| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
            assert_eq!(strip(&out), strip(&s));
        }
    }

    /// A garbage `*` width (2^31-1, as a wrong va_list gives) or a long digit string: an
    /// error at once (Windows: the allocated buffer stops growing at 128 KiB), not gigabytes
    #[test]
    fn huge_widths_fail_fast() {
        for spec in ["*d", ".*d", "-*x", "099999999999999999999999d", ".99999999999999999999999u", "*.*d"] {
            let w: Vec<u16> = format!("%1!{spec}!").encode_utf16().collect();
            let mut g = crate::rtlfmt::tests::Words::new(&[0x7fff_ffff, 0x7fff_ffff, 7]);
            let t = std::time::Instant::now();
            let r = outcome(rtlfmt::format(&request(&w, ARGUMENT_ARRAY, Some(rtlfmt::ArgList::Array), true, None), &mut g));
            assert!(matches!(r, Err((122, _))), "{spec}");
            assert!(t.elapsed() < std::time::Duration::from_secs(2), "{spec}");
        }
    }

    proptest::proptest! {
        /// Differential: numeric inserts (d i u x X o, flags - + space 0 #, width, precision)
        /// against the C library's printf (FormatMessage formats inserts with the CRT's
        /// printf); within MAX_FIELD the two agree character for character.
        #[test]
        fn numeric_inserts_are_printfs(flags in proptest::collection::vec(proptest::sample::select(vec!['-', '+', ' ', '0', '#']), 0..4),
                                       width in proptest::option::of(0usize..40), prec in proptest::option::of(0usize..40),
                                       conv in proptest::sample::select(vec!['d', 'i', 'u', 'x', 'X', 'o']),
                                       v in proptest::prop_oneof![proptest::prelude::any::<u32>(), 0u32..20, proptest::prelude::Just(0x8000_0000u32)],
                                       star in proptest::prelude::any::<bool>()) {
            let mut spec: String = flags.iter().collect();
            let mut slots = vec![];
            if let Some(w) = width {
                if star { spec.push('*'); slots.push(w as u32); } else { spec += &w.to_string(); }
            }
            if let Some(p) = prec {
                if star { spec += ".*"; slots.push(p as u32); } else { spec += &format!(".{p}"); }
            }
            spec.push(conv);
            slots.push(v);
            // (Windows passes the value first and the stars after it: the array's order is printf's)
            let w: Vec<u16> = format!("%1!{spec}!").encode_utf16().collect();
            let mut g = crate::rtlfmt::tests::Words::new(&slots.iter().map(|&x| u64::from(x)).collect::<Vec<u64>>());
            let got = outcome(rtlfmt::format(&request(&w, ARGUMENT_ARRAY, Some(rtlfmt::ArgList::Array), true, None), &mut g)).map_or_default(|v| String::from_utf16_lossy(&v));
            let mut buf = [0u8; 256];
            let fmt = std::ffi::CString::new(format!("%{spec}")).unwrap();
            let n = unsafe {
                match (width.is_some() && star, prec.is_some() && star) {
                    (true, true) => libc::snprintf(buf.as_mut_ptr() as *mut _, 256, fmt.as_ptr(), slots[0] as i32, slots[1] as i32, v),
                    (true, false) | (false, true) => libc::snprintf(buf.as_mut_ptr() as *mut _, 256, fmt.as_ptr(), slots[0] as i32, v),
                    _ => libc::snprintf(buf.as_mut_ptr() as *mut _, 256, fmt.as_ptr(), v),
                }
            };
            let want = String::from_utf8_lossy(&buf[..n as usize]).into_owned();
            proptest::prop_assert_eq!(got, want, "%{}", spec);
        }
    }
}
