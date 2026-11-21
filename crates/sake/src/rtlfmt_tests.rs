use super::*;
use crate::HashMap;

/// arguments as words (x86: 4 bytes each, a va_list's 8-byte read takes two) and strings at
/// made-up addresses
#[derive(Default)]
pub(crate) struct Words {
    pub words: Vec<u64>,
    pub at: usize,
    pub ptr64: bool,
    pub wide: HashMap<u64, Vec<u16>>,
    pub narrow: HashMap<u64, Vec<u8>>,
}

impl Words {
    pub fn new(words: &[u64]) -> Words {
        Words { words: words.to_vec(), ..Words::default() }
    }
    fn word(&mut self) -> u64 {
        let w = self.words.get(self.at).copied().unwrap_or(0xdead_beef);
        self.at += 1;
        w
    }
    /// a UTF-16 string argument: its address
    pub fn w(&mut self, s: &str) -> u64 {
        let p = 0x1_0000 + 0x100 * (self.wide.len() + self.narrow.len()) as u64;
        self.wide.insert(p, s.encode_utf16().collect());
        p
    }
    pub fn a(&mut self, s: &str) -> u64 {
        let p = 0x1_0000 + 0x100 * (self.wide.len() + self.narrow.len()) as u64;
        self.narrow.insert(p, s.bytes().collect());
        p
    }
}

impl Guest for Words {
    fn next(&mut self, list: ArgList, eight: bool) -> u64 {
        if self.ptr64 || !eight || list == ArgList::Array {
            return self.word();
        }
        let lo = self.word() & 0xffff_ffff;
        lo | self.word() << 32
    }
}

impl crt::fmt::Args for Words {
    fn int(&mut self, _: u32) -> u64 {
        0
    }
    fn float(&mut self) -> f64 {
        0.0
    }
    fn narrow(&mut self, p: u64, max: Option<usize>) -> Vec<u8> {
        let s = self.narrow.get(&p).cloned().unwrap_or_default();
        s.into_iter().take(max.unwrap_or(usize::MAX)).collect()
    }
    fn wide(&mut self, p: u64, max: Option<usize>) -> Vec<u16> {
        let s = self.wide.get(&p).cloned().unwrap_or_default();
        s.into_iter().take(max.unwrap_or(usize::MAX)).collect()
    }
    fn counted(&mut self, _: u64) -> (usize, u64) {
        (0, 0)
    }
    fn a2w(&self, s: &[u8]) -> Vec<u16> {
        s.iter().map(|&b| u16::from(b)).collect()
    }
    fn w2a(&self, s: &[u16]) -> Vec<u8> {
        s.iter().map(|&u| u as u8).collect()
    }
    fn store(&mut self, _: u64, _: u32, _: u64) {}
}

fn units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

struct Call<'a> {
    msg: &'a str,
    width: u32,
    ignore: bool,
    ansi: bool,
    list: Option<ArgList>,
    room: u32,
}

impl Call<'_> {
    fn new(msg: &str) -> Call<'_> {
        Call { msg, width: 0, ignore: false, ansi: false, list: Some(ArgList::VaList), room: 128 }
    }
    fn run(&self, g: &mut Words) -> Outcome {
        let m = units(self.msg);
        let r = Request { message: &m, width: Width::of(self.width), ignore_inserts: self.ignore, ansi: self.ansi, args: self.list, room: self.room, ptr64: g.ptr64 };
        format(&r, g)
    }
}

/// Wine's testfmt: success, the text, and the length in bytes with the 0
fn testfmt(msg: &str, want: &str, width: u32, ansi: bool, g: &mut Words) {
    let o = Call { width, ansi, ..Call::new(msg) }.run(g);
    assert_eq!(o.status, Status::Success, "{msg:?}");
    assert_eq!(o.text().map(String::from_utf16_lossy), Some(want.to_string()), "{msg:?} width {width}");
    assert_eq!(o.length, Some(2 * (want.encode_utf16().count() as u32 + 1)), "{msg:?}");
}

fn plain(msg: &str, want: &str, width: u32) {
    testfmt(msg, want, width, false, &mut Words::new(&[]));
}

/// Wine's ntdll:rtlstr test_RtlFormatMessage (written from Windows' answers), the basic formats
#[test]
fn basic_formats_follow_windows() {
    plain("test", "test", 0);
    plain("", "", 0);
    let mut g = Words::new(&[]);
    let t = g.w("test");
    g.words = vec![t];
    testfmt("%1", "test", 0, false, &mut g);
    for (m, want, ansi, a, wide) in [
        ("%1!s!", "test", false, "test", true),
        ("%1!s!", "foo", true, "foo", false),
        ("%1!S!", "test", false, "test", false),
        ("%1!S!", "foo", true, "foo", true),
        ("%1!hs!%1!hS!", "testtest", false, "test", false),
        ("%1!ls!%1!lS!%1!ws!%1!wS!", "foofoofoofoo", true, "foo", true),
        ("%1!Saa!", "testaa", false, "test", false),
    ] {
        let mut g = Words::new(&[]);
        let p = if wide { g.w(a) } else { g.a(a) };
        g.words = vec![p];
        testfmt(m, want, 0, ansi, &mut g);
    }
    for (m, want, ansi, ch) in [
        ("%1!c!", "a", false, 'a'),
        ("%1!c!", "b", true, 'b'),
        ("%1!C!", "c", false, 'c'),
        ("%1!C!", "d", true, 'd'),
        ("%1!hc!", "e", false, 'e'),
        ("%1!hC!", "f", false, 'f'),
        ("%1!lc!", "g", true, 'g'),
        ("%1!lC!", "h", true, 'h'),
        ("%1!wc!", "i", true, 'i'),
        ("%1!wC!", "j", true, 'j'),
    ] {
        testfmt(m, want, 0, ansi, &mut Words::new(&[ch as u64]));
    }
    testfmt("%1!04X!", "BEEF", 0, false, &mut Words::new(&[0xbeef]));
    plain("%.%%%Z%n%t%r%!% ", ".%Z\r\n\t\r! ", 0);
    testfmt("%1!*.*u!,%1!*.*u!", "  001, 0002", 0, false, &mut Words::new(&[5, 3, 1, 4, 2]));
    testfmt("%1!*.*u!,%3!*.*u!", "  001,  0002", 0, false, &mut Words::new(&[5, 3, 1, 6, 4, 2]));
    testfmt("%1", "(null)", 0, false, &mut Words::new(&[0]));
    let mut g = Words::new(&[]);
    let abc = g.a("abc");
    g.words = vec![abc, 0];
    testfmt("%2", "(null)", 0, true, &mut g);
    let mut g = Words::new(&[]);
    let h = g.w("hello");
    g.words = vec![h];
    testfmt("ab%1!!cd", "abcd", 0, false, &mut g);
    testfmt("abc%1!#.000000000000000000000000000x!", "abc0x22", 0, false, &mut Words::new(&[34]));
    plain("a\r\nb\rc\r\rd\r\r\ne", "a\r\nb\r\nc\r\n\r\nd\r\n\r\ne", 0);
    testfmt("%1!Ix! %2!QQ!", "1234 QQ", 0, false, &mut Words::new(&[0x1234]));
    testfmt("%1!#llx!%2!#x!%1!#hx!", "0x1234560x789abc0x3456", 0, false, &mut Words::new(&[0x123456, 0x789abc]));
}

/// I64 inserts read 8 bytes of a va_list on x86 (for every slot that insert reads); an array
/// keeps one ULONG_PTR a slot, and the stars come after the value (Wine's 32-bit expectations)
#[test]
fn i64_and_stars_read_as_windows() {
    let w = |v: &[u64]| Words::new(v);
    testfmt("%1!#I64x! %2!x!", "0x567800001234 beef", 0, false, &mut w(&[0x1234, 0x5678, 0xbeef]));
    testfmt(
        "%1!x! %2!#I64x! %3!#I64x! %4!x!",
        "dead 0x1111222233334444 0x5555666677778888 beef",
        0,
        false,
        &mut w(&[0xdead, 0x33334444, 0x11112222, 0x77778888, 0x55556666, 0xbeef]),
    );
    testfmt(
        "%3!#I64x! %4!#I64x! %3!x! %1!x!",
        "0x1111222233334444 0x5555666677778888 33334444 1",
        0,
        false,
        &mut w(&[1, 2, 3, 4, 0x33334444, 0x11112222, 0x77778888, 0x55556666, 0xbeef, 0xbee2]),
    );
    testfmt("%2!x! %1!I64x!", "5678 1234", 0, false, &mut w(&[0x1234, 0x5678, 0xbeef]));
    testfmt("%2!*.*I64x! %1!u! %4!u! %2!u!", "  000090000000b 19 7 15", 0, false, &mut w(&[19, 17, 15, 13, 11, 9, 7]));
    let arr = |m: &str| Call { list: Some(ArgList::Array), ..Call::new(m) }.run(&mut w(&[19, 17, 15, 13, 11, 9, 7]));
    assert_eq!(arr("%2!*.*I64x! %1!u! %4!u! %2!u!").text().map(String::from_utf16_lossy).as_deref(), Some("        d0000000f 19 13 17"));
    assert_eq!(arr("%1!I64u! %2!u! %4!.*I64x! %5!I64u!").text().map(String::from_utf16_lossy).as_deref(), Some("19 17 0000b00000000 11"));
    let arr = |m: &str| Call { list: Some(ArgList::Array), ..Call::new(m) }.run(&mut w(&[6, 4, 2, 5, 3, 1]));
    assert_eq!(arr("%1!*.*u!,%1!*.*u!").text().map(String::from_utf16_lossy).as_deref(), Some("  0002, 00003"));
    assert_eq!(arr("%1!*.*u!,%4!*.*u!").text().map(String::from_utf16_lossy).as_deref(), Some("  0002,  001"));
    // (x64: one 8-byte slot for everything)
    let mut g = Words { ptr64: true, ..w(&[19, 17, 15, 13, 11, 9]) };
    testfmt("%2!*.*I64x! %1!u! %4!u! %2!u!", "  00000000000000d 19 11 17", 0, false, &mut g);
}

#[test]
fn max_width_follows_windows() {
    let mut g = Words::new(&[]);
    let v: Vec<u64> = ["testing"].iter().map(|s| g.w(s)).collect();
    g.words = v;
    testfmt("%1", "testing\r\n", 3, false, &mut g);
    let mut g = Words::new(&[]);
    let v: Vec<u64> = ["testing", "abcdef", "foobar"].iter().map(|s| g.w(s)).collect();
    g.words = v;
    testfmt("%1%2%3", "testing\r\nabcdef\r\nfoobar\r\n", 4, false, &mut g);
    let mut g = Words::new(&[]);
    let v: Vec<u64> = ["test", "abcd", "abc", "def"].iter().map(|s| g.w(s)).collect();
    g.words = v;
    testfmt("%1%2%3%4", "test\r\nabcd\r\nabcdef\r\n", 4, false, &mut g);
    let mut g = Words::new(&[]);
    let v: Vec<u64> = ["testing", "foo bar"].iter().map(|s| g.w(s)).collect();
    g.words = v;
    testfmt("%1a\nb%2", "testing\r\na\r\nbfoo bar\r\n", 3, false, &mut g);
    plain("a%tb%t%t%t%c%r%r%r%r%r%rdefg", "a\r\nb\r\n\r\n\r\nc\r\r\r\r\r\rdef\r\ng", 3);
    plain("test abcd ", "test\r\n\r\nabcd\r\n ", 4);
    let mut g = Words::new(&[]);
    let h = g.w("hello");
    g.words = vec![h];
    testfmt("test abcdef %1 foobar", "tes\r\nt\r\nabc\r\ndef\r\n\r\nhello\r\nfoo\r\nbar\r\n", 3, false, &mut g);
    plain("te st\nabc d\nfoo", "te st\r\nabc d\r\nfoo", 6);
    plain("te st    ab    d\nfoo", "te st\r\n  ab\r\n d foo", 7);
    plain("te\tst\t\t\t\tab\t\t\td\nfoo", "te\tst\t\t\r\n\t\tab\t\t\t\r\nd foo", 7);
    plain("te st\n\n\r\n\nab    d\nfoo    ", "te st\r\n  ab\r\n d foo\r\n   ", 7);
    plain("te st\r\nabc d\n\nfoo\rbar", "te st abc d  foo bar", 0xff);
    plain("te st%r%nabc d%nfoo%rbar", "te st\r\r\nabc d\r\nfoo\rbar", 0xff);
    plain(
        "\x01\x02\x03\x04\x07\x07\x07\x07\x08\x08\x08\x08\t\t\t\t\x0b\x0b\x0b\x0b\x0c\x0c\x0c\x0c\r\r\r\r    a",
        "\x01\x02\r\n\x03\x04\r\n\x07\x07\r\n\x07\x07\r\n\x08\x08\r\n\x08\x08\r\n\t\t\r\n\t\t\r\n\x0b\x0b\r\n\x0b\x0b\r\n\x0c\x0c\r\n\x0c\x0c\r\n\r\n\r\n\r\n\r\na",
        2,
    );
    // (kernel32's 0xFF: line breaks become spaces, nothing wraps)
    plain("a\r\nb", "a b", u32::MAX);
}

/// Exhaustive over every unit, as Wine's loop: { i, ' ', i, i, i, i, i, ' ', i } at width 4 into
/// a buffer of "xxxxxxxxxx"
#[test]
fn every_unit_wraps_as_windows() {
    for i in 1u16..0xffff {
        let src = [i, 0x20, i, i, i, i, i, 0x20, i];
        let r = Request { message: &src, width: Width::of(4), ignore_inserts: false, ansi: false, args: Some(ArgList::VaList), room: 128, ptr64: false };
        let o = format(&r, &mut Words::new(&[]));
        let mut buffer = units("xxxxxxxxxx");
        for (k, &u) in o.buffer.iter().enumerate() {
            if k < buffer.len() {
                buffer[k] = u;
            } else {
                buffer.push(u);
            }
        }
        let end = buffer.iter().position(|&u| u == 0).unwrap_or(buffer.len());
        let got = &buffer[..end];
        let want: Vec<u16> = match i {
            9 => units("\r\n\r\n\t"),
            10 | 13 | 0x20 => units("\r\n\r\n "),
            0x25 => units(" %% \r\nxxxx"),
            _ => [i, 13, 10, i, i, i, i, 13, 10, i, 0x20, i].to_vec(),
        };
        assert_eq!(got, &want[..], "{i:04x}");
    }
}

#[test]
fn overflows_keep_what_was_written() {
    let run = |m: &str, room: u32, g: &mut Words| Call { room, ..Call::new(m) }.run(g);
    let o = run("testing", 4, &mut Words::new(&[]));
    assert_eq!((o.status, o.buffer, o.length), (Status::BufferOverflow, units("test"), None));
    let mut g = Words::new(&[]);
    let t = g.w("test");
    g.words = vec![t];
    let o = run("%1", 4, &mut g);
    assert_eq!((o.status, o.buffer), (Status::BufferOverflow, units("tes\0")));
    let o = run("%1!x!", 4, &mut Words::new(&[0x12345678]));
    assert_eq!((o.status, o.buffer), (Status::BufferOverflow, units("123\0")));
    let mut g = Words::new(&[5]);
    let abc = g.w("abc");
    g.words.push(abc);
    let o = run("%1!*s!", 5, &mut g);
    assert_eq!((o.status, o.buffer), (Status::BufferOverflow, units("  ab\0")));
    let o = run("ab%n", 3, &mut Words::new(&[]));
    assert_eq!((o.status, o.buffer), (Status::BufferOverflow, units("ab")));
}

#[test]
fn ignore_inserts_and_invalid_args_follow_windows() {
    let ig = |m: &str| Call { ignore: true, list: None, ..Call::new(m) }.run(&mut Words::new(&[]));
    let o = ig("%1!x!%r%%%n%t");
    assert_eq!(o.text().map(String::from_utf16_lossy).as_deref(), Some("%1!x!\r%%\r\n\t"));
    let o = ig("ab%0cd");
    assert_eq!((o.status, o.buffer), (Status::Success, units("ab\0")));
    assert_eq!(ig("%1!s! %12 %3!d!").text().map(String::from_utf16_lossy).as_deref(), Some("%1 %12 %3!d!"));
    for (list, m) in [(None, "abc%1"), (None, "abc%"), (Some(ArgList::Array), "abc%")] {
        {
            let o = Call { list, ..Call::new(m) }.run(&mut Words::new(&[]));
            assert_eq!((o.status, o.buffer, o.length), (Status::InvalidParameter, units("abc"), None), "{m}");
        }
    }
    let o = Call::new("%1!u! %2!u").run(&mut Words::new(&[34]));
    assert_eq!((o.status, o.buffer), (Status::InvalidParameter, units("34 ")));
    let o = Call::new("%1!**u!").run(&mut Words::new(&[34]));
    assert_eq!(o.text().map(String::from_utf16_lossy).as_deref(), Some("*u"));
    let o = Call::new("%1!0.3+*u!").run(&mut Words::new(&[34]));
    assert_eq!(o.text().map(String::from_utf16_lossy).as_deref(), Some("+*u"));
    // (three stars, a 4th digit, a spec of more than 30 units, an index past the cache)
    for m in ["%1!***u!", "%1234", "%1!0123456789012345678901234567890u!", "%201", "%200!*u!"] {
        assert_eq!(Call::new(m).run(&mut Words::new(&[1, 2, 3])).status, Status::InvalidParameter, "{m}");
    }
    assert_eq!(Call::new("%1!-----------------------------u!").run(&mut Words::new(&[1])).text().map(String::from_utf16_lossy).as_deref(), Some("1"));
    let many: Vec<u64> = (0..200).collect();
    assert_eq!(Call::new("%200!u! %199!u!").run(&mut Words::new(&many)).text().map(String::from_utf16_lossy).as_deref(), Some("199 198"));
}

proptest::proptest! {
    /// Without inserts and without wrapping, a buffer too small holds the start of the whole
    /// answer (as much as fits), and the status says so; with room enough the answer is whole
    #[test]
    fn short_buffers_hold_a_prefix(msg in "[ab \\r\\n%nrt]{0,40}", room in 0u32..60) {
        let m = units(&msg);
        let full = format(&Request { message: &m, width: Width::Keep, ignore_inserts: true, ansi: false, args: None, room: 1000, ptr64: false }, &mut Words::new(&[]));
        let o = format(&Request { message: &m, width: Width::Keep, ignore_inserts: true, ansi: false, args: None, room, ptr64: false }, &mut Words::new(&[]));
        if full.status == Status::Success && o.status == Status::Success {
            proptest::prop_assert_eq!(o, full);
        } else if full.status == Status::Success {
            proptest::prop_assert_eq!(o.status, Status::BufferOverflow);
            proptest::prop_assert!(o.buffer.len() <= room as usize);
            proptest::prop_assert_eq!(&full.buffer[..o.buffer.len()], &o.buffer[..]);
        }
    }

    /// Wrapping at any width: no line is longer than the width, and only blanks and line
    /// breaks change (the other units stay, in order)
    #[test]
    fn wrapped_lines_fit(msg in "[ab \\t\\r\\n]{0,60}", width in 1u32..30) {
        let m = units(&msg);
        let o = format(&Request { message: &m, width: Width::of(width), ignore_inserts: false, ansi: false, args: None, room: 10_000, ptr64: false }, &mut Words::new(&[]));
        proptest::prop_assert_eq!(o.status, Status::Success);
        let text = String::from_utf16_lossy(o.text().unwrap_or_default());
        for line in text.split("\r\n") {
            proptest::prop_assert!(line.chars().count() <= width as usize, "{:?} width {}: {:?}", msg, width, text);
        }
        let strip = |t: &str| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        proptest::prop_assert_eq!(strip(&text), strip(&msg));
    }
}
