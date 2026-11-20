//! COM objects from the generated vtable tables (com_methods.rs, from Microsoft's Win32 metadata:
//! tools/win32md): an object names its interface and the methods it implements; every other slot is
//! a stub that logs "Iface::Method" once and fails (E_NOTIMPL, 0.0 for float results, the hidden
//! result pointer for structure results). QueryInterface, AddRef and Release come for free:
//! QueryInterface answers the interface and every interface it derives from.

use kome::locks::Lock as _;
use crate::api::{host_fn, Ctx, Handler};
use crate::mem;
use std::collections::HashMap;
use std::sync::Mutex;

#[path = "com_methods.rs"]
pub(crate) mod methods;
pub use methods::Guid;

pub use crate::hresult::E_NOTIMPL;
pub use crate::hresult::E_NOINTERFACE;

/// An interface com_methods.rs has a vtable for. Only `com::iface!(IName)` makes one, and the
/// compiler checks the name there: a misspelt interface does not build (it panicked at the first
/// object of it).
#[derive(Clone, Copy)]
pub struct Iface(&'static Table);

impl Iface {
    #[doc(hidden)]
    pub const fn checked(name: &'static str) -> Iface {
        let t = find(name);
        assert!(t.is_some(), "com: no such interface in com_methods.rs");
        // (the assert stops the build first: com::iface! evaluates this in a const)
        const NONE: Table = ("", &[], &[]);
        match t {
            Some(t) => Iface(t),
            None => Iface(&NONE),
        }
    }
    pub const fn name(self) -> &'static str {
        self.0 .0
    }
}
impl PartialEq for Iface {
    fn eq(&self, o: &Iface) -> bool {
        self.name() == o.name()
    }
}
impl Eq for Iface {}
impl std::hash::Hash for Iface {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.name().hash(h)
    }
}
impl std::fmt::Debug for Iface {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Iface({})", self.name())
    }
}

/// A method of interface `of` (its own or one it derives from), as `com::impls!` names it: the
/// compiler checks it is one, so a misspelt method does not build (it was a silent E_NOTIMPL
/// stub). `slot`: that vtable slot only, for an interface with two methods of one name
/// (ID2D1DeviceContext's CreateBitmap at 4 and 57).
#[derive(Clone, Copy, Debug)]
pub struct Method {
    of: Iface,
    name: &'static str,
    slot: Option<usize>,
}

impl Method {
    #[doc(hidden)]
    pub const fn checked(of: Iface, name: &'static str, slot: Option<usize>) -> Method {
        let ms = of.0 .1;
        match slot {
            Some(i) => assert!(slot_is(ms, i, name), "com: the interface has no such method at that slot"),
            None => {
                let mut rest = ms;
                let mut found = false;
                while let [(m, _, _), more @ ..] = rest {
                    if str_eq(m, name) {
                        found = true;
                        break;
                    }
                    rest = more;
                }
                assert!(found, "com: the interface has no such method");
            }
        }
        Method { of, name, slot }
    }
    pub const fn name(self) -> &'static str {
        self.name
    }
}

/// (for the macros: a handler as a fn pointer, whichever function it is)
#[doc(hidden)]
pub const fn handler(f: Handler) -> Handler {
    f
}

/// (const: slice patterns, no index that could be out of range)
const fn str_eq(a: &str, b: &str) -> bool {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    loop {
        match (a, b) {
            ([], []) => return true,
            ([x, ra @ ..], [y, rb @ ..]) if *x == *y => {
                a = ra;
                b = rb;
            }
            _ => return false,
        }
    }
}
/// Whether method slot `i` of `ms` is named `name`
const fn slot_is(ms: &[(&str, u8, u8)], i: usize, name: &str) -> bool {
    let mut rest = ms;
    let mut k = 0;
    while let [(m, _, _), more @ ..] = rest {
        if k == i {
            return str_eq(m, name);
        }
        k = k.saturating_add(1);
        rest = more;
    }
    false
}

type Table = (&'static str, &'static [(&'static str, u8, u8)], &'static [&'static str]);

const fn find(name: &str) -> Option<&'static Table> {
    let mut rest: &'static [Table] = &methods::IFACES;
    while let [t, more @ ..] = rest {
        if str_eq(t.0, name) {
            return Some(t);
        }
        rest = more;
    }
    None
}

/// `com::iface!(IWICBitmap)`: that interface, checked when it compiles.
macro_rules! iface {
    ($i:ident) => {{
        const I: $crate::com::Iface = $crate::com::Iface::checked(stringify!($i));
        I
    }};
}
pub(crate) use iface;

/// `com::impls!(IWICBitmapSource { GetSize: src_size, CreateBitmap@57: f })`: methods of that
/// interface (or of the ones it derives from) and their handlers, every name checked when it
/// compiles. An object of a derived interface takes them (com::vtable: of another, it panics).
macro_rules! impls {
    ($i:ident { $($m:ident $(@ $slot:literal)? : $h:expr),* $(,)? }) => {
        [$((
            {
                const M: $crate::com::Method = $crate::com::Method::checked($crate::com::iface!($i), stringify!($m), $crate::com::impls!(@slot $($slot)?));
                M
            },
            $crate::com::handler($h),
        )),*]
    };
    (@slot) => { None };
    (@slot $s:literal) => { Some($s) };
}
pub(crate) use impls;

/// `com::method!(IDWriteFactory::CreateMonitorRenderingParams)`: that method, checked when it
/// compiles (for a handler shared by several methods that asks which one it is).
macro_rules! method {
    ($i:ident :: $m:ident) => {{
        const M: $crate::com::Method = $crate::com::Method::checked($crate::com::iface!($i), stringify!($m), None);
        M
    }};
}
pub(crate) use method;

/// A vtable's methods: com::impls! lists.
pub type Impls<'a> = &'a [(Method, Handler)];

struct Obj {
    iface: Iface,
    refs: u32,
    /// what goes with the object at its last Release (its module's state), before its memory
    free: Option<fn(u32)>,
    /// a singleton the module hands out again (never freed)
    kept: bool,
}

/// Blocks of released objects, freed QUARANTINE releases later: a guest that calls through a
/// pointer it has just released still finds a vtable (whose methods find no object).
const QUARANTINE: usize = 256;
static FREED: Mutex<std::collections::VecDeque<u32>> = Mutex::new(std::collections::VecDeque::new());

static OBJS: Mutex<Option<HashMap<u32, Obj>>> = Mutex::new(None);
/// trap address -> (interface, method, result kind)
static STUBS: Mutex<Option<HashMap<u32, (&'static str, &'static str, u8)>>> = Mutex::new(None);
/// trap address -> (method, stack argument dwords), for handlers shared by several methods
static NAMES: Mutex<Option<HashMap<u32, (&'static str, u8)>>> = Mutex::new(None);

/// The name of the method whose trap `trap` is.
pub fn method_name(trap: u32) -> Option<&'static str> {
    NAMES.locked().as_ref()?.get(&trap).map(|m| m.0)
}
/// Its argument count (dwords, with `this` and a hidden result pointer).
pub fn method_dwords(trap: u32) -> Option<u32> {
    NAMES.locked().as_ref()?.get(&trap).map(|m| u32::from(m.1))
}
static VTBLS: Mutex<Option<HashMap<Iface, u32>>> = Mutex::new(None);

fn objs<R>(f: impl FnOnce(&mut HashMap<u32, Obj>) -> R) -> R {
    f(OBJS.locked().get_or_insert_with(HashMap::new))
}

fn table(iface: Iface) -> &'static Table {
    iface.0
}

/// The IIDs an interface answers QueryInterface for (itself and its bases), worked out once for
/// all interfaces: QueryInterface did two linear scans of ~270 names per base under the object
/// lock.
fn iids_of(iface: &str) -> &'static [Guid] {
    static M: std::sync::OnceLock<HashMap<&'static str, Vec<Guid>>> = std::sync::OnceLock::new();
    M.get_or_init(|| methods::IFACES.iter().map(|t| (t.0, t.2.iter().filter_map(|n| iid(n)).collect())).collect())
        .get(iface)
        .map_or(&[], |v| v.as_slice())
}

pub fn iid(iface: &str) -> Option<Guid> {
    methods::IIDS.iter().find(|t| t.0 == iface).map(|t| t.1)
}

pub fn read_guid(p: u32) -> Guid {
    kome::guid::fields(&mem::array::<16>(p))
}


pub fn warn_once(msg: &str) {
    static SEEN: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);
    if SEEN.locked().get_or_insert_with(Default::default).insert(msg.to_string()) {
        eprintln!("{msg}");
    }
}

/// A stub method's trap tag (crate::api::set_tag): STUB_TAG | its kind, STUB_WARNED once said
/// (a title that calls one every frame paid a lock, a format and a hash for each: review C-2).
const STUB_TAG: u32 = 1 << 31;
const STUB_WARNED: u32 = 1 << 30;

fn stub(c: &mut Ctx) -> u32 {
    let (tag, _) = crate::api::tag_of(c.trap);
    let kind = if tag & STUB_WARNED != 0 {
        kome::num::lo8(tag)
    } else {
        let (iface, name, kind) = STUBS.locked().as_ref().and_then(|m| m.get(&c.trap).copied()).unwrap_or(("?", "?", 0));
        warn_once(&format!("sake: com {iface}::{name} not implemented"));
        crate::api::set_tag(c.trap, STUB_TAG | STUB_WARNED | u32::from(kind), 0);
        kind
    };
    match kind {
        1 => {
            c.st0 = Some(0.0);
            0
        }
        2 => c.a(1),
        _ => E_NOTIMPL,
    }
}

fn query_interface(c: &mut Ctx) -> u32 {
    let (this, riid, out) = (c.a(0), c.a(1), c.a(2));
    if out == 0 {
        return 0x8000_4003;
    }
    let want = read_guid(riid);
    let found = objs(|m| {
        let o = m.get_mut(&this)?;
        let hit = iids_of(o.iface.name()).contains(&want).then_some(this);
        if hit.is_some() {
            o.refs = o.refs.saturating_add(1);
        }
        hit
    });
    match found {
        Some(p) => {
            mem::putp(out, p);
            0
        }
        None => {
            let iface = objs(|m| m.get(&this).map_or("?", |o| o.iface.name()));
            warn_once(&format!("sake: com {iface}::QueryInterface {{{}}}: no", kome::guid::text(&kome::guid::guid(want.0, want.1, want.2, want.3))));
            mem::putp(out, 0);
            E_NOINTERFACE
        }
    }
}

fn add_ref(c: &mut Ctx) -> u32 {
    let this = c.a(0);
    objs(|m| m.get_mut(&this).map_or(1, |o| {
        o.refs = o.refs.saturating_add(1);
        o.refs
    }))
}

fn release(c: &mut Ctx) -> u32 {
    release_obj(c.a(0))
}

/// Release on `obj` (for objects whose own Release frees more): the references left. At the
/// last one the object's `on_free` runs and its block goes (after the quarantine).
pub fn release_obj(this: u32) -> u32 {
    let gone = objs(|m| match m.get_mut(&this) {
        Some(o) if o.refs > 1 || o.kept => {
            o.refs = o.refs.saturating_sub(1);
            Err(o.refs)
        }
        Some(_) => Ok(m.remove(&this).and_then(|o| o.free)),
        None => Err(0),
    });
    match gone {
        Err(n) => n,
        Ok(free) => {
            // (outside the table's lock: freeing an object releases what it held)
            if let Some(f) = free {
                f(this);
            }
            free_later(this);
            0
        }
    }
}

/// Frees guest heap block `p` after the quarantine (QUARANTINE later frees): for objects of
/// modules with their own reference counts (dmime) that go the same way.
pub fn free_later(p: u32) {
    let old = {
        let mut q = FREED.lock().unwrap_or_else(|e| e.into_inner());
        q.push_back(p);
        if q.len() > QUARANTINE { q.pop_front() } else { None }
    };
    if let Some(p) = old {
        mem::heap_free(p);
    }
}

/// `f` runs when `obj`'s last reference goes (its module's state for it, what it holds).
pub fn on_free(obj: u32, f: fn(u32)) {
    objs(|m| {
        if let Some(o) = m.get_mut(&obj) {
            o.free = Some(f);
        }
    });
}

/// `obj` is a singleton handed out again: its Release never frees it.
pub fn keep(obj: u32) {
    objs(|m| {
        if let Some(o) = m.get_mut(&obj) {
            o.kept = true;
        }
    });
}

/// The references `obj` has (None: not a live object).
pub fn refs(obj: u32) -> Option<u32> {
    objs(|m| m.get(&obj).map(|o| o.refs))
}

/// Whether trap `a` is a COM method sake does not implement.
pub fn is_stub(a: u32) -> bool {
    STUBS.locked().as_ref().is_some_and(|m| m.contains_key(&a))
}

/// What slot `i` (method `name`) of a vtable gets from `impls`: an entry for that slot (where an
/// interface has two methods of one name, as ID2D1DeviceContext's CreateBitmap with
/// D2D1_BITMAP_PROPERTIES1 at 57 beside 4's), else the first entry named `name` without a slot;
/// None for IUnknown's own or a stub.
fn slot_impl<'a>(i: usize, name: &str, impls: Impls<'a>) -> Option<&'a (Method, Handler)> {
    let named = |m: &&(Method, Handler)| m.0.name == name;
    impls.iter().filter(named).find(|m| m.0.slot == Some(i)).or_else(|| impls.iter().filter(named).find(|m| m.0.slot.is_none()))
}
/// Slot `i`'s handler: `impls`' or, with none, IUnknown's own for 0..3 and a stub after.
fn slot_handler(i: usize, found: Option<Handler>) -> Handler {
    match (found, i) {
        (Some(f), _) => f,
        (None, 0) => query_interface,
        (None, 1) => add_ref,
        (None, 2) => release,
        (None, _) => stub,
    }
}

/// The vtable of `iface` with the methods `impls` names; built once per interface. Every method
/// must be one of `iface`'s own or of an interface it derives from (a list for another
/// interface panics here, at its first object).
pub fn vtable(iface: Iface, impls: Impls) -> u32 {
    #[cfg(test)]
    CALLS.with(|c| c.borrow_mut().push((iface, impls.to_vec())));
    if let Some(&v) = VTBLS.locked().get_or_insert_with(HashMap::new).get(&iface) {
        return v;
    }
    // (one builder at a time, looking again under it: two threads making the first object of an
    // interface at once each built a table, and the loser's and its host functions stayed)
    static BUILD: Mutex<()> = Mutex::new(());
    let _b = BUILD.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&v) = VTBLS.locked().get_or_insert_with(HashMap::new).get(&iface) {
        return v;
    }
    let t = table(iface);
    for m in impls {
        assert!(t.2.contains(&m.0.of.name()), "com: {}::{} is not a method of {}", m.0.of.name(), m.0.name, iface.name());
    }
    let k = mem::ps();
    let v = mem::heap_alloc(k.saturating_mul(kome::num::u32_of(t.1.len())), true);
    // (its out-parameters given once the table is there and BUILD let go)
    let mut outs = Vec::new();
    for (i, &(name, dwords, kind)) in t.1.iter().enumerate() {
        let found = slot_impl(i, name, impls).map(|m| m.1);
        let f = slot_handler(i, found);
        let a = host_fn(&format!("{}::{i}:{name}", iface.name()), f, 4 * u32::from(dwords));
        outs.push((a, kome::num::u32_of(i)));
        if i > 2 && found.is_none() {
            STUBS.locked().get_or_insert_with(HashMap::new).insert(a, (iface.name(), name, kind));
        }
        NAMES.locked().get_or_insert_with(HashMap::new).insert(a, (name, dwords));
        crate::mem::wp(mem::ga(v).at(kome::num::u32_of(i), k), a);
    }
    VTBLS.locked().get_or_insert_with(HashMap::new).insert(iface, v);
    drop(_b);
    for (a, i) in outs {
        crate::api::set_com_outs(a, iface.name(), i, crate::api::ComArgs::Typed);
    }
    v
}

/// (tests) Every vtable asked for on this thread (a test's own objects, whatever other tests
/// built first): its interface and the methods it was asked with.
#[cfg(test)]
type Calls = Vec<(Iface, Vec<(Method, Handler)>)>;
#[cfg(test)]
thread_local!(static CALLS: std::cell::RefCell<Calls> = const { std::cell::RefCell::new(Vec::new()) });

/// (tests) What fills each slot of these interfaces' vtables (as this thread last asked for
/// them), one line per slot that is not a stub: its handler named by the first slot (in this
/// order) it filled, so that two slots sharing a handler, and which list's handler won, show. A
/// module's object constructors are pinned with this before their method lists change shape
/// (MASU_UPDATE_GOLDEN=1 writes the file).
#[cfg(test)]
pub(crate) fn check_vtables(golden: &str, ifaces: &[Iface]) {
    let calls = CALLS.with(|c| c.borrow().clone());
    // (slots are named by the list entry that fills them, not by the handler's address: an
    // optimized build merges identical handlers such as `|_| 0`)
    let mut seen: Vec<((&str, &str, Option<usize>), String)> = Vec::new();
    let mut out = String::new();
    for &i in ifaces {
        let Some((_, impls)) = calls.iter().rev().find(|b| b.0 == i) else { panic!("com: no vtable asked for {}", i.name()) };
        let (t, i) = (table(i), i.name());
        for (k, &(name, _, _)) in t.1.iter().enumerate() {
            let label = match (slot_impl(k, name, impls), k) {
                (Some(m), _) => {
                    let key = (m.0.of.name(), m.0.name, m.0.slot);
                    match seen.iter().find(|s| s.0 == key) {
                        Some(s) => s.1.clone(),
                        None => {
                            let l = format!("{i}.{k}");
                            seen.push((key, l.clone()));
                            l
                        }
                    }
                }
                (None, 0) => "QueryInterface".into(),
                (None, 1) => "AddRef".into(),
                (None, 2) => "Release".into(),
                (None, _) => "stub".to_string(),
            };
            if label != "stub" {
                out += &format!("{i} {k} {name}: {label}\n");
            }
        }
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(golden);
    kome::golden::check(&path, &out);
}

/// A new object (refcount 1) of `size` bytes, the vtable pointer first.
pub fn object(iface: Iface, impls: Impls, size: u32) -> u32 {
    let vt = vtable(iface, impls);
    let o = mem::heap_alloc(size.max(mem::ps()), true);
    mem::wp(o, vt);
    objs(|m| m.insert(o, Obj { iface, refs: 1, free: None, kept: false }));
    o
}

/// AddRef from the host (an object handed out again).
pub fn add_ref_obj(obj: u32) {
    objs(|m| {
        if let Some(o) = m.get_mut(&obj) {
            o.refs = o.refs.saturating_add(1);
        }
    });
}

pub fn is(obj: u32, iface: Iface) -> bool {
    objs(|m| m.get(&obj).is_some_and(|o| o.iface == iface))
}

// ---- WIC ----

pub const CLSID_WIC_FACTORY: u32 = 0xcaca_f262;
pub const CLSID_WIC_FACTORY2: u32 = 0x317d_06e8;

/// CoCreateInstance of the WIC imaging factory.
pub fn create_instance(clsid: u32) -> Option<u32> {
    match mem::r32(clsid) {
        CLSID_WIC_FACTORY | CLSID_WIC_FACTORY2 => Some(crate::wic::factory()),
        0x4590_f811 => Some(crate::wmi::locator()), // CLSID_WbemLocator
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::methods::IFACES;

    fn slot(iface: &str, m: &str) -> (u8, u8) {
        let t = IFACES.iter().find(|t| t.0 == iface).unwrap();
        t.1.iter().find(|x| x.0 == m).map(|x| (x.1, x.2)).unwrap()
    }

    /// Integers wider or narrower than a dword come back in EAX (EDX:EAX), not through a hidden
    /// result pointer: the tables from Wine's IDL had these as structures, so their stubs popped
    /// one dword too many.
    #[test]
    fn integer_results_are_not_structures() {
        for (i, m) in [("IDWriteFontFace", "GetGlyphCount"), ("ID2D1Device", "GetMaximumTextureMemory"), ("ID2D1Device4", "GetMaximumColorGlyphCacheMemory"), ("IDWriteFontDownloadQueue", "GetGenerationCount"), ("IDWriteFontFaceReference", "GetFileSize"), ("IDWriteFontFaceReference", "GetLocalFileSize")] {
            assert_eq!(slot(i, m), (1, 0), "{i}::{m}");
        }
        // (a structure result: the hidden pointer counts)
        assert_eq!(slot("ID2D1RenderTarget", "GetSize"), (2, 2));
        assert_eq!(slot("ID2D1StrokeStyle", "GetMiterLimit"), (1, 1));
    }

    /// ID2D1DeviceContext (every version) has CreateBitmap twice: ID2D1RenderTarget's at 4
    /// (D2D1_BITMAP_PROPERTIES) and its own at 57 (D2D1_BITMAP_PROPERTIES1, whose options make a
    /// bitmap a target): d2d.rs implements "CreateBitmap#57" for the second alone.
    #[test]
    fn device_context_create_bitmap_slots() {
        for t in IFACES.iter().filter(|t| t.0.starts_with("ID2D1DeviceContext")) {
            let at: Vec<usize> = t.1.iter().enumerate().filter(|x| x.1 .0 == "CreateBitmap").map(|x| x.0).collect();
            assert_eq!(at, [4, 57], "{}", t.0);
        }
        for i in ["ID2D1RenderTarget", "ID2D1BitmapRenderTarget", "ID2D1HwndRenderTarget"] {
            let t = IFACES.iter().find(|t| t.0 == i).unwrap();
            assert_eq!(t.1.iter().filter(|x| x.0 == "CreateBitmap").count(), 1, "{i}");
        }
    }

    /// Overloads in one interface keep distinct names (the earlier get "_"s) and the vtable
    /// inherits every base method first.
    #[test]
    fn overloads_and_bases() {
        let t = IFACES.iter().find(|t| t.0 == "IDWriteFontSet").unwrap();
        let names: Vec<&str> = t.1.iter().map(|x| x.0).collect();
        assert_eq!(&names[..3], &["QueryInterface", "AddRef", "Release"]);
        assert_eq!(&names[7..10], &["GetPropertyValues__", "GetPropertyValues_", "GetPropertyValues"]);
        let s = IFACES.iter().find(|t| t.0 == "IWICStream").unwrap();
        assert_eq!(s.2, &["IWICStream", "IStream", "ISequentialStream", "IUnknown"]);
        assert_eq!(s.1.len(), 18);
    }
}

/// Object lifetimes (review B-1): the last Release runs the object's on_free once and frees its
/// block after the quarantine; kept singletons are never freed.
#[cfg(test)]
mod lifetime_tests {
    use super::*;
    use crate::test_guest::enter;
    use proptest::prelude::*;

    static FREED_OBJS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
    fn note(o: u32) {
        FREED_OBJS.locked().push(o);
    }
    fn freed(o: u32) -> usize {
        FREED_OBJS.locked().iter().filter(|&&x| x == o).count()
    }
    fn new() -> u32 {
        let o = object(iface!(ID2D1StrokeStyle1), &[], 8);
        on_free(o, note);
        o
    }

    #[test]
    fn last_release_frees_after_the_quarantine() {
        let _g = enter();
        FREED_OBJS.locked().clear();
        let o = new();
        add_ref_obj(o);
        assert_eq!(release_obj(o), 1);
        assert_eq!(freed(o), 0);
        assert_eq!(release_obj(o), 0);
        assert_eq!(freed(o), 1);
        assert_eq!(refs(o), None);
        // (still a block with its vtable for a stale call; a Release more does nothing)
        assert!(mem::heap_owns(o));
        assert_eq!(release_obj(o), 0);
        assert_eq!(freed(o), 1);
        for _ in 0..QUARANTINE {
            let x = new();
            release_obj(x);
        }
        assert!(!mem::heap_owns(o));
    }

    #[test]
    fn kept_objects_stay() {
        let _g = enter();
        FREED_OBJS.locked().clear();
        let o = new();
        keep(o);
        assert_eq!(release_obj(o), 0);
        assert_eq!(release_obj(o), 0);
        assert_eq!(refs(o), Some(0));
        assert_eq!(freed(o), 0);
        add_ref_obj(o);
        assert_eq!(refs(o), Some(1));
    }

    proptest! {
        /// Any AddRef/Release sequence over a few objects agrees with a count per object: on_free
        /// runs exactly when (and once) an object's count reaches 0, and the table forgets it.
        #[test]
        fn counts_as_a_model(ops in proptest::collection::vec((0usize..4, any::<bool>()), 0..60)) {
            let _g = enter();
            FREED_OBJS.locked().clear();
            let objs: Vec<u32> = (0..4).map(|_| new()).collect();
            let mut model = [1u32; 4];
            for (i, up) in ops {
                let o = objs[i];
                if model[i] == 0 {
                    continue;
                }
                if up {
                    add_ref_obj(o);
                    model[i] += 1;
                } else {
                    model[i] -= 1;
                    prop_assert_eq!(release_obj(o), model[i]);
                }
                prop_assert_eq!(refs(o), (model[i] > 0).then_some(model[i]));
                prop_assert_eq!(freed(o), usize::from(model[i] == 0));
            }
            for (i, &o) in objs.iter().enumerate() {
                while model[i] > 0 {
                    model[i] -= 1;
                    release_obj(o);
                }
                prop_assert_eq!(freed(o), 1);
            }
        }
    }
}

/// Review b05: QueryInterface's IID sets are the per-base lookups they replace, for every
/// interface; the first objects of an interface made on several threads at once share one vtable.
#[cfg(test)]
mod review_tests {
    use super::*;

    #[test]
    fn iid_sets_are_the_base_lookups() {
        for t in methods::IFACES.iter() {
            let want: Vec<Guid> = t.2.iter().filter_map(|n| iid(n)).collect();
            assert_eq!(iids_of(t.0), &want[..], "{}", t.0);
            for g in &want {
                assert!(t.2.iter().any(|n| iid(n) == Some(*g)));
            }
        }
        assert!(iids_of("no such interface").is_empty());
    }

    /// A list for one interface given to an object of another (not derived from it) is refused
    /// at its first vtable: its methods would sit in other slots.
    #[test]
    #[should_panic(expected = "is not a method of IWICBitmapLock")]
    fn methods_of_another_interface_panic() {
        let _g = crate::test_guest::enter();
        fn h(_: &mut Ctx) -> u32 {
            0
        }
        vtable(iface!(IWICBitmapLock), &impls!(ID2D1Bitmap { GetSize: h }));
    }

    #[test]
    fn first_vtables_race_to_one() {
        let _g = crate::test_guest::enter();
        // (interfaces from the end of the table: the least likely to be built by other tests)
        let names: Vec<Iface> = methods::IFACES.iter().rev().take(6).map(Iface).collect();
        for name in names {
            let ts: Vec<_> = (0..8).map(|_| std::thread::spawn(move || vtable(name, &[]))).collect();
            let vs: Vec<u32> = ts.into_iter().map(|t| t.join().unwrap()).collect();
            assert!(vs.iter().all(|&v| v == vs[0] && v != 0), "{name:?}: {vs:x?}");
            assert_eq!(vtable(name, &[]), vs[0]);
        }
    }
}
