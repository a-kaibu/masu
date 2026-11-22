//! GDI handles and PEB->GdiSharedHandleTable as Windows has them (Wine's gdi32:gdiobj
//! test_shared_handle_table, which Windows passes): any sequence of objects made and deleted
//! leaves each live handle's slot entry with the handle's Unique, its type in the handle's
//! high word, distinct handles, a deleted handle naming nothing (its slot's next object has
//! another generation), and the stock objects' handles with the stock flag.

use super::*;
use crate::test_guest::{call, enter, guest_thread};
use proptest::prelude::*;

/// (what to make, NTGDI_OBJ_* >> 16)
fn make(k: u8) -> (u32, u8) {
    match k % 6 {
        0 => (call(CreateSolidBrush, &[0x123456]), 0x10),
        1 => (call(CreatePen, &[0, 3, 0]), 0x30),
        2 => {
            let lb = mem::heap_alloc(12, true);
            (call(ExtCreatePen, &[0x10000, 3, lb, 0, 0]), 0x50)
        }
        3 => (call(CreateRectRgn, &[1, 2, 30, 40]), 0x04),
        4 => (call(CreateBitmap, &[4, 4, 1, 32, 0]), 0x09),
        _ => (call(CreateCompatibleDC, &[0]), 0x01),
    }
}

fn delete(h: u32, ty: u8) -> u32 {
    if ty == 0x01 { call(DeleteDC, &[h]) } else { call(DeleteObject, &[h]) }
}

fn entry(h: u32) -> (u16, u8, u32) {
    let table = with(|t| t.slots.table);
    let e = table + u32::from(handle_index(h)) * ENTRY;
    (r16(e + 12), r8(e + 14), r32(e))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    #[test]
    fn the_table_names_each_live_object(ops in prop::collection::vec((any::<bool>(), 0u8..6, any::<prop::sample::Index>()), 1..60)) {
        let _g = enter();
        guest_thread();
        init();
        if with(|t| t.slots.table) == 0 {
            let peb = mem::heap_alloc(0x200, true);
            share_table(peb);
            prop_assert_eq!(r32(peb + 0x94), with(|t| t.slots.table));
        }
        let mut live: Vec<(u32, u8)> = Vec::new();
        let mut dead: Vec<u32> = Vec::new();
        for (add, k, pick) in ops {
            if add || live.is_empty() {
                let (h, ty) = make(k);
                prop_assert!(h != 0);
                prop_assert_eq!((h >> 16) as u8 & 0x7f, ty, "type in the handle {:#x}", h);
                prop_assert_eq!((h >> 16) & 0x80, 0, "no stock flag on {:#x}", h);
                prop_assert!(!live.iter().any(|&(x, _)| x == h), "{:#x} twice", h);
                live.push((h, ty));
            } else {
                let (h, ty) = live.swap_remove(pick.index(live.len()));
                prop_assert_eq!(delete(h, ty), 1);
                dead.push(h);
            }
            for &(h, ty) in &live {
                let (unique, base, object) = entry(h);
                prop_assert_eq!(u32::from(unique), h >> 16);
                prop_assert_eq!(base, ty & 0x1f);
                prop_assert!(object != 0);
            }
        }
        for h in dead {
            prop_assert!(!live.iter().any(|&(x, _)| x == h), "a deleted {:#x} came back", h);
            prop_assert!(!is_object(h));
        }
        for (h, ty) in live {
            delete(h, ty);
        }
    }
}

#[test]
fn stock_objects_have_the_stock_flag_and_their_type() {
    let _g = enter();
    guest_thread();
    init();
    // (WHITE_BRUSH, WHITE_PEN, SYSTEM_FONT, DEFAULT_PALETTE, STOCK_LAST + 1: the default bitmap)
    for (i, ty) in [(0, 0x10), (6, 0x30), (13, 0x0a), (15, 0x08), (20, 0x09)] {
        let h = call(GetStockObject, &[i]);
        assert_eq!(h >> 16 & 0xff, 0x80 | ty, "stock {i}: {h:#x}");
        assert_eq!(stock_index(h), Some(i));
        assert_eq!(call(DeleteObject, &[h]), 1);
        assert!(is_object(h));
    }
    assert_eq!(call(GetStockObject, &[20]), DEFAULT_BITMAP);
    assert_eq!(call(GetStockObject, &[21]), 0);
}
