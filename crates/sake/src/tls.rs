//! TlsAlloc's slots as a value, and where a slot lives in a TEB: pure functions (k32 keeps the
//! process's one table in a Mutex and reads and writes guest memory at the places these name).

/// How many slots a process has: 64 in the TEB, 1024 in its expansion table.
pub(crate) const SLOTS: u32 = 1088;
const INLINE: u32 = 64;

/// Which slots are given out. Slot 0 never is (as on XP SP2 and later: kernel32:thread
/// test_reserved_tls).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Slots(Vec<bool>);

impl Default for Slots {
    fn default() -> Slots {
        let mut v = vec![false; 1088];
        if let Some(first) = v.first_mut() {
            *first = true;
        }
        Slots(v)
    }
}

impl Slots {
    /// The lowest free slot, now given out (None: all are).
    pub(crate) fn alloc(&mut self) -> Option<u32> {
        let i = self.0.iter().position(|used| !used)?;
        if let Some(used) = self.0.get_mut(i) {
            *used = true;
        }
        u32::try_from(i).ok()
    }

    /// Frees `i`; false when it was not given out (never slot 0).
    pub(crate) fn free(&mut self, i: u32) -> bool {
        match self.0.get_mut(kome::num::us(i)) {
            Some(used) if *used && i != 0 => {
                *used = false;
                true
            }
            _ => false,
        }
    }
}

/// Where slot `i` of a thread is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    /// at this offset in the TEB
    Teb(u32),
    /// at `off` in the expansion table, whose pointer is at `table` in the TEB (0 until made)
    Expansion { table: u32, off: u32 },
}

/// Where slot `i` (< SLOTS) is in a 32-bit or 64-bit TEB: TlsSlots, then TlsExpansionSlots.
pub(crate) fn place(x64: bool, i: u32) -> Place {
    let (slots, table, k) = if x64 { (0x1480u32, 0x1780u32, 8u32) } else { (0xe10, 0xf94, 4) };
    if i < INLINE {
        Place::Teb(slots.saturating_add(k.saturating_mul(i)))
    } else {
        Place::Expansion { table, off: k.saturating_mul(i.saturating_sub(INLINE)) }
    }
}

#[cfg(test)]
#[path = "tls_tests.rs"]
mod tests;
