//! display pinned as a whole before its integer conversions become typed: seeded random sessions
//! of a Display (windows placed, restacked, shown and hidden, drawn plainly and partly, drawn from
//! shared surfaces, unshared, destroyed, the screen resized, frames taken with and without direct
//! ones), each step's frame io, view, extent, window list and a hash of the screen buffer.
//! display_golden.txt holds them; MASU_UPDATE_GOLDEN=1 rewrites it.
use super::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + i32::try_from(self.below(u64::try_from(hi - lo).unwrap())).unwrap()
    }
}

fn fnv(h: &mut u64, v: &[u32]) {
    for &x in v {
        for b in x.to_le_bytes() {
            *h ^= u64::from(b);
            *h = h.wrapping_mul(0x100_0000_01b3);
        }
    }
}

fn session(seed: u64) -> String {
    let mut r = Rng(seed);
    let mut d = Display { w: 40, h: 30, wins: HashMap::new(), order: Vec::new(), screen: vec![0xff00_0000; 40 * 30], serial: 0, bands: Default::default(), stale: Rect::default(), ext: Rect { l: 0, t: 0, r: 40, b: 30 } };
    // (shared surfaces outlive the windows that show them)
    let mut surfaces: Vec<Vec<u32>> = Vec::new();
    let mut seen = 0u32;
    let mut out = String::new();
    for step in 0..120 {
        let h = 1 + u32::try_from(r.below(5)).unwrap();
        let op = r.below(10);
        match op {
            0..=2 => {
                let (x, y) = (r.range(-30, 50), r.range(-20, 40));
                let (w, ht) = (r.range(0, 35), r.range(0, 25));
                let rect = Rect { l: x, t: y, r: x + w, b: y + ht };
                let client = Rect { l: x + 1, t: y + 2, r: x + w - 1, b: y + ht - 1 };
                d.pos(h, rect, client, r.below(4) != 0, r.below(3) == 0, u32::try_from(r.below(6)).unwrap());
            }
            3..=5 => {
                if let Some(rect) = d.wins.get(&h).map(|w| w.rect) {
                    let (w, ht) = (rect.w().max(0), rect.h().max(0));
                    let stride = w + r.range(0, 3);
                    let n = usize::try_from(stride.max(1) * ht.max(1) + 8).unwrap();
                    let px: Vec<u32> = (0..n).map(|_| 0xff00_0000 | u32::try_from(r.below(1 << 24)).unwrap()).collect();
                    let (l, t) = (r.range(-2, w + 1), r.range(-2, ht + 1));
                    let dirty = Rect { l, t, r: l + r.range(0, w + 2), b: t + r.range(0, ht + 2) };
                    unsafe { d.draw(h, dirty, px.as_ptr(), stride, 0, 0) };
                }
            }
            6 => {
                if let Some(rect) = d.wins.get(&h).map(|w| w.rect) {
                    let (w, ht) = (rect.w().max(0), rect.h().max(0));
                    // (as big as any window gets: a window moved to another size reads its surface at that size)
                    let px: Vec<u32> = (0..40 * 30).map(|_| 0xff00_0000 | u32::try_from(r.below(1 << 24)).unwrap()).collect();
                    unsafe { d.draw_shared(h, Rect { l: 0, t: 0, r: w, b: ht }, px.as_ptr(), w, ht) };
                    surfaces.push(px);
                }
            }
            7 => d.unshare(h),
            8 => {
                if r.below(4) == 0 {
                    d.destroy(h);
                } else if r.below(4) == 0 {
                    d.set_size(r.range(0, 60), r.range(0, 45));
                }
            }
            _ => {}
        }
        let mut io = [0u32; 12];
        io[0] = seen;
        io[3] = u32::from(r.below(2) == 0);
        let direct = io[3] != 0;
        d.frame(&mut io, direct);
        seen = io[0];
        // (io[4] is a direct frame's surface address: as which surface it is, 1 up)
        if io[4] != 0 {
            io[4] = surfaces.iter().position(|s| s.as_ptr() as usize as u32 == io[4]).map_or(u32::MAX, |i| i as u32 + 1);
        }
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        fnv(&mut hash, &d.screen);
        let mut list = [0i32; 40];
        let n = d.windows(&mut list);
        let mut above = [0i32; 40];
        let a = d.windows_above(h, &mut above);
        let (v, e) = (d.view(), d.ext);
        out += &format!("{step} op{op} h{h} io={io:?} view={:?} ext={:?} wins={:?} above={:?} screen={}:{hash:016x}\n", [v.l, v.t, v.r, v.b], [e.l, e.t, e.r, e.b], &list[..4 * n], a.map(|a| &above[..(4 * a).min(40)]), d.screen.len());
    }
    drop(surfaces);
    out
}

#[test]
fn sessions_answer_as_pinned() {
    let mut got = String::new();
    for seed in 1..=12u64 {
        got += &session(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/display_golden.txt");
    kome::golden::check(&path, &got);
}
