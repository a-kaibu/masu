//! The identification and setup headers.

use crate::bits::{ilog, us, Bits};
use crate::codebook::Codebook;
use crate::Error;

pub struct Ident {
    pub channels: usize,
    pub rate: u32,
    /// short and long block sizes
    pub blocksize: [usize; 2],
}

/// A floor 1 partition class.
pub struct Class {
    pub dim: usize,
    /// subclass bits
    pub sub: u32,
    /// the master book (`sub` > 0)
    pub master: Option<usize>,
    /// the subclass books
    pub subs: Vec<Option<usize>>,
}

/// A floor 1's multiplier (1..=4) and the range of its posts' values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Multiplier {
    X1,
    X2,
    X3,
    X4,
}

impl Multiplier {
    fn of(two_bits: u8) -> Multiplier {
        match two_bits & 3 {
            0 => Multiplier::X1,
            1 => Multiplier::X2,
            2 => Multiplier::X3,
            _ => Multiplier::X4,
        }
    }
    pub fn get(self) -> i32 {
        match self {
            Multiplier::X1 => 1,
            Multiplier::X2 => 2,
            Multiplier::X3 => 3,
            Multiplier::X4 => 4,
        }
    }
    /// The posts' range (the specification's [256, 128, 86, 64][multiplier - 1]).
    pub fn range(self) -> i32 {
        match self {
            Multiplier::X1 => 256,
            Multiplier::X2 => 128,
            Multiplier::X3 => 86,
            Multiplier::X4 => 64,
        }
    }
}

pub struct Floor1 {
    pub partition_class: Vec<u8>,
    pub classes: Vec<Class>,
    pub multiplier: Multiplier,
    /// the posts' x (in decoding order)
    pub x: Vec<i32>,
    /// for posts 2..: the low and high neighbours
    pub lo: Vec<usize>,
    pub hi: Vec<usize>,
    /// post indices in increasing x
    pub sorted: Vec<usize>,
}

/// A residue's type: how its vectors interleave.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResidueKind {
    /// each partition's values interleaved by the book's dimension
    Zero,
    /// each partition's values in order
    One,
    /// the channels interleaved into one vector
    Two,
}

pub struct Residue {
    pub kind: ResidueKind,
    pub begin: usize,
    pub end: usize,
    pub grouping: usize,
    pub classifications: usize,
    pub classbook: usize,
    /// per classification: the book of each stage
    pub books: Vec<[Option<usize>; 8]>,
    pub stages: usize,
}

pub struct Mapping {
    /// (magnitude, angle) channels
    pub coupling: Vec<(usize, usize)>,
    /// per channel: its submap
    pub mux: Vec<usize>,
    /// per submap: (floor, residue)
    pub submaps: Vec<(usize, usize)>,
}

pub struct Mode {
    pub blockflag: bool,
    pub mapping: usize,
}

/// A block's size: the stream's short or long one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Block {
    Short,
    Long,
}

impl Block {
    /// The size of a block flag (`false`: short).
    pub fn of(flag: bool) -> Block {
        if flag {
            Block::Long
        } else {
            Block::Short
        }
    }
    /// The index of the size in the stream's blocksizes.
    pub fn index(self) -> usize {
        match self {
            Block::Short => 0,
            Block::Long => 1,
        }
    }
}

impl Mode {
    pub fn block(&self) -> Block {
        Block::of(self.blockflag)
    }
}

impl Ident {
    /// The size in samples of a block.
    pub fn size(&self, b: Block) -> usize {
        self.blocksize[b.index()]
    }
}

pub struct Setup {
    pub ident: Ident,
    pub books: Vec<Codebook>,
    pub floors: Vec<Floor1>,
    pub residues: Vec<Residue>,
    pub mappings: Vec<Mapping>,
    pub modes: Vec<Mode>,
}

fn header(b: &mut Bits, kind: u32) -> Result<(), Error> {
    if b.read(8) != Some(kind) {
        return Err(Error::Header);
    }
    for &c in b"vorbis" {
        if b.read(8) != Some(u32::from(c)) {
            return Err(Error::Header);
        }
    }
    Ok(())
}

impl Ident {
    pub fn read(packet: &[u8]) -> Result<Ident, Error> {
        let e = Error::Header;
        let mut b = Bits::new(packet);
        header(&mut b, 1)?;
        if b.read(32).ok_or(e)? != 0 {
            return Err(e);
        }
        let channels = b.uz::<8>().ok_or(e)?;
        let rate = b.read(32).ok_or(e)?;
        b.read(32).ok_or(e)?;
        b.read(32).ok_or(e)?;
        b.read(32).ok_or(e)?;
        let b0 = b.read(4).ok_or(e)?;
        let b1 = b.read(4).ok_or(e)?;
        if channels == 0 || rate == 0 || !(6..=13).contains(&b0) || !(6..=13).contains(&b1) || b0 > b1 || b.read(1) != Some(1) {
            return Err(e);
        }
        Ok(Ident { channels, rate, blocksize: [1 << b0, 1 << b1] })
    }
}

impl Floor1 {
    fn read(b: &mut Bits, books: usize) -> Result<Floor1, Error> {
        let e = Error::Header;
        let parts = b.uz::<5>().ok_or(e)?;
        let mut partition_class = Vec::with_capacity(parts);
        for _ in 0..parts {
            partition_class.push(b.u8::<4>().ok_or(e)?);
        }
        let nclasses = partition_class.iter().map(|&c| usize::from(c) + 1).max().unwrap_or(0);
        // a book number + 1 (0: none)
        let book = |v: usize| -> Result<Option<usize>, Error> {
            match v.checked_sub(1) {
                None => Ok(None),
                Some(v) if v < books => Ok(Some(v)),
                Some(_) => Err(e),
            }
        };
        let mut classes = Vec::with_capacity(nclasses);
        for _ in 0..nclasses {
            let dim = b.uz::<3>().ok_or(e)? + 1;
            let sub = b.read(2).ok_or(e)?;
            let master = if sub > 0 { book(b.uz::<8>().ok_or(e)? + 1)? } else { None };
            let mut subs = Vec::with_capacity(1 << sub);
            for _ in 0..1 << sub {
                subs.push(book(b.uz::<8>().ok_or(e)?)?);
            }
            classes.push(Class { dim, sub, master, subs });
        }
        let multiplier = Multiplier::of(b.u8::<2>().ok_or(e)?);
        let rangebits = b.read(4).ok_or(e)?;
        let mut x = vec![0, 1 << rangebits];
        for &c in &partition_class {
            for _ in 0..classes[usize::from(c)].dim {
                // (rangebits < 16: a post's x fits an i32)
                x.push(i32::try_from(b.read(rangebits).ok_or(e)?).ok().ok_or(e)?);
            }
        }
        if x.len() > 65 {
            return Err(e);
        }
        let mut sorted: Vec<usize> = (0..x.len()).collect();
        sorted.sort_by_key(|&i| x[i]);
        if sorted.windows(2).any(|w| x[w[0]] == x[w[1]]) {
            return Err(e);
        }
        let (mut lo, mut hi) = (Vec::new(), Vec::new());
        for i in 2..x.len() {
            let (mut l, mut h) = (0usize, 1usize);
            let (mut lx, mut hx) = (0, 1 << 30);
            for j in 0..i {
                if x[j] > lx && x[j] < x[i] {
                    l = j;
                    lx = x[j];
                }
                if x[j] < hx && x[j] > x[i] {
                    h = j;
                    hx = x[j];
                }
            }
            lo.push(l);
            hi.push(h);
        }
        Ok(Floor1 { partition_class, classes, multiplier, x, lo, hi, sorted })
    }
}

impl Residue {
    fn read(b: &mut Bits, kind: ResidueKind, books: &[Codebook]) -> Result<Residue, Error> {
        let e = Error::Header;
        let begin = b.uz::<24>().ok_or(e)?;
        let end = b.uz::<24>().ok_or(e)?;
        let grouping = b.uz::<24>().ok_or(e)? + 1;
        let classifications = b.uz::<6>().ok_or(e)? + 1;
        let classbook = b.uz::<8>().ok_or(e)?;
        let mut cascade = Vec::with_capacity(classifications);
        for _ in 0..classifications {
            let low = b.read(3).ok_or(e)?;
            let high = if b.flag().ok_or(e)? { b.read(5).ok_or(e)? } else { 0 };
            cascade.push(high << 3 | low);
        }
        let mut list = Vec::with_capacity(classifications);
        let mut stages = 0;
        for &c in &cascade {
            let mut s = [None; 8];
            for (k, slot) in s.iter_mut().enumerate() {
                if c >> k & 1 == 1 {
                    let book = b.uz::<8>().ok_or(e)?;
                    if book >= books.len() || books[book].values.is_empty() {
                        return Err(e);
                    }
                    *slot = Some(book);
                }
            }
            stages = stages.max(us(ilog(c)));
            list.push(s);
        }
        if classbook >= books.len() || books[classbook].dim == 0 {
            return Err(e);
        }
        Ok(Residue { kind, begin, end, grouping, classifications, classbook, books: list, stages })
    }
}

impl Setup {
    pub fn read(ident: Ident, packet: &[u8]) -> Result<Setup, Error> {
        let e = Error::Header;
        let mut b = Bits::new(packet);
        header(&mut b, 5)?;
        let nbooks = b.uz::<8>().ok_or(e)? + 1;
        let mut books = Vec::with_capacity(nbooks);
        for _ in 0..nbooks {
            books.push(Codebook::read(&mut b)?);
        }
        let ntimes = b.read(6).ok_or(e)? + 1;
        for _ in 0..ntimes {
            if b.read(16).ok_or(e)? != 0 {
                return Err(e);
            }
        }
        let nfloors = b.read(6).ok_or(e)? + 1;
        let mut floors = Vec::new();
        for _ in 0..nfloors {
            match b.read(16).ok_or(e)? {
                1 => floors.push(Floor1::read(&mut b, books.len())?),
                // floor 0 (no encoder has made it since 2002): left to the game's decoder
                0 => return Err(Error::Unsupported),
                _ => return Err(e),
            }
        }
        let nres = b.read(6).ok_or(e)? + 1;
        let mut residues = Vec::new();
        for _ in 0..nres {
            let kind = match b.read(16).ok_or(e)? {
                0 => ResidueKind::Zero,
                1 => ResidueKind::One,
                2 => ResidueKind::Two,
                _ => return Err(e),
            };
            residues.push(Residue::read(&mut b, kind, &books)?);
        }
        let nmaps = b.read(6).ok_or(e)? + 1;
        let ch = ident.channels;
        let mut mappings = Vec::new();
        for _ in 0..nmaps {
            if b.read(16).ok_or(e)? != 0 {
                return Err(e);
            }
            let submaps = if b.flag().ok_or(e)? { b.uz::<4>().ok_or(e)? + 1 } else { 1 };
            let mut coupling = Vec::new();
            if b.flag().ok_or(e)? {
                let steps = b.uz::<8>().ok_or(e)? + 1;
                let bits = ilog(u32::try_from(ch - 1).ok().ok_or(e)?);
                for _ in 0..steps {
                    let m = us(b.read(bits).ok_or(e)?);
                    let a = us(b.read(bits).ok_or(e)?);
                    if m == a || m >= ch || a >= ch {
                        return Err(e);
                    }
                    coupling.push((m, a));
                }
            }
            if b.read(2).ok_or(e)? != 0 {
                return Err(e);
            }
            let mut mux = vec![0usize; ch];
            if submaps > 1 {
                for m in mux.iter_mut() {
                    *m = b.uz::<4>().ok_or(e)?;
                    if *m >= submaps {
                        return Err(e);
                    }
                }
            }
            let mut subs = Vec::with_capacity(submaps);
            for _ in 0..submaps {
                b.read(8).ok_or(e)?;
                let f = b.uz::<8>().ok_or(e)?;
                let r = b.uz::<8>().ok_or(e)?;
                if f >= floors.len() || r >= residues.len() {
                    return Err(e);
                }
                subs.push((f, r));
            }
            mappings.push(Mapping { coupling, mux, submaps: subs });
        }
        let nmodes = b.read(6).ok_or(e)? + 1;
        let mut modes = Vec::new();
        for _ in 0..nmodes {
            let blockflag = b.flag().ok_or(e)?;
            let window = b.read(16).ok_or(e)?;
            let transform = b.read(16).ok_or(e)?;
            let mapping = b.uz::<8>().ok_or(e)?;
            if window != 0 || transform != 0 || mapping >= mappings.len() {
                return Err(e);
            }
            modes.push(Mode { blockflag, mapping });
        }
        if b.read(1) != Some(1) {
            return Err(e);
        }
        Ok(Setup { ident, books, floors, residues, mappings, modes })
    }
}
