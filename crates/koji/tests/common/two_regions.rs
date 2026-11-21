//! The memory the x87 differential tests run in (diff_x87, diff_x87_80): the code at 0x1000, a
//! data page at `data_base` (both where the host's own run put them), nothing else.
use koji::{Fault, Memory};

pub struct TwoRegions {
    pub code: Vec<u8>,
    pub data_base: u32,
    pub data: Vec<u8>,
}

impl Memory for TwoRegions {
    fn read8(&mut self, a: u32) -> Result<u8, Fault> {
        if (0x1000..0x1000 + self.code.len() as u32).contains(&a) {
            return Ok(self.code[(a - 0x1000) as usize]);
        }
        let o = a.wrapping_sub(self.data_base) as usize;
        self.data.get(o).copied().ok_or(Fault { addr: a, write: false })
    }
    fn write8(&mut self, a: u32, v: u8) -> Result<(), Fault> {
        let o = a.wrapping_sub(self.data_base) as usize;
        match self.data.get_mut(o) {
            Some(b) => { *b = v; Ok(()) }
            None => Err(Fault { addr: a, write: true }),
        }
    }
}
