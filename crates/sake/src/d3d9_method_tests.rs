//! Method ids against the code as it was before they became a type: every id's argument
//! count and Ex-ness, and the named methods' numbers.
use super::*;

fn old_args_of(id: u32) -> usize {
    let (i, m) = (id >> 8, (id & 0xff) as usize);
    let list = if i >= 1 && i <= NUM_IFACES { Some(METHODS[i as usize - 1]) } else { METHODS8.iter().find(|x| u32::from(x.0) == i).map(|x| x.1) };
    list.and_then(|l| l.get(m).copied()).unwrap_or(16).min(16) as usize
}
fn old_is_ex(id: u32) -> bool {
    matches!(id, 0x100 | 0x200 | 0x111..=0x115 | 0x277..=0x285)
}

#[test]
fn ids_as_before() {
    for id in 0..0x1_0000u32 {
        let m = Method::of(id).unwrap();
        assert_eq!(m.id(), id);
        assert_eq!(args_of(m), old_args_of(id), "{id:#x}");
        assert_eq!(is_ex(m), old_is_ex(id), "{id:#x}");
    }
    assert_eq!(Method::of(0x1_0000), None);
    let named = [
        (GETNPATCHMODE, 2 << 8 | 80),
        (SURFACE_GETDC, 4 << 8 | 15),
        (SURFACE_RELEASEDC, 4 << 8 | 16),
        (CREATE_DEVICE9, 1 << 8 | 16),
        (CREATE_DEVICE8, 17 << 8 | 15),
        (D3D9_QI, 1 << 8),
        (DEVICE_QI, 2 << 8),
    ];
    for (m, id) in named {
        assert_eq!(m.id(), id);
    }
    assert_eq!(FLOAT_ARGS.map(|(m, i)| (m.id(), i)), [(2 << 8 | 79, 1)]);
}
