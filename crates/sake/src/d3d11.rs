//! d3d11.dll and dxgi.dll: kura's Direct3D 11 / DXGI (crates/kura/src/d3d11), the module
//! Wine's dlls/vnd3d11 and dlls/vndxgi hand their calls to. As in d3d9.rs, every vtable entry
//! is a trap whose handler passes the guest's stack arguments to the host as
//! `interface << 8 | index`.

#![allow(non_snake_case)]

use crate::hresult::{E_NOTIMPL, E_NOINTERFACE};
use std::sync::OnceLock;

use crate::api::{host_fn, implement, Ctx};
use crate::mem;
use kura::d3d11::{CREATE_DEVICE, CREATE_DEVICE10, CREATE_FACTORY};

#[path = "d3d11_methods.rs"]
mod methods;
use methods::METHODS;

const FIRST: u8 = 64;

/// A method's trap tag (crate::api::set_tag): its id, with FLOAT for a method that returns a
/// float; the tag's data is its argument count with `this`.
const FLOAT: u32 = 1 << 31;

/// A vtable trap's tag: kura's method (interface FIRST.., vtable index; the host takes
/// `iface << 8 | index`) and whether it returns a float (FLOAT: the host's u32 is its bits).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Tag {
    iface: u8,
    index: u8,
    float: bool,
}

impl Tag {
    /// the tag as the trap keeps it
    fn bits(self) -> u32 {
        self.id() | if self.float { FLOAT } else { 0 }
    }
    /// a trap's tag back, None for one `bits` didn't make
    fn of(bits: u32) -> Option<Self> {
        let [index, iface, 0, top] = bits.to_le_bytes() else { return None };
        let float = match top {
            0 => false,
            0x80 => true,
            _ => return None,
        };
        Some(Self { iface, index, float })
    }
    /// `iface << 8 | index`, as kura takes it
    fn id(self) -> u32 {
        u32::from(self.iface) << 8 | u32::from(self.index)
    }
}

// (the interfaces and their methods fit a u8, below 255 so that `(0u8..).zip` never steps past)
const fn short(l: &[&[u8]]) -> bool {
    match l {
        [] => true,
        [h, rest @ ..] => h.len() < 255 && short(rest),
    }
}
const _: () = assert!(FIRST as usize + METHODS.len() < 255 && short(&METHODS));

fn calls() -> &'static kura::d3d9::HostCalls {
    crate::d3d9::host_calls()
}

fn method(c: &mut Ctx) -> u32 {
    let (tag, nargs) = crate::api::tag_of(c.trap);
    // (every tag was made by init from a Tag)
    let Some(tag) = Tag::of(tag) else { return 0 };
    let id = tag.id();
    let r = if mem::x64() {
        let a = slots64(c, tag, nargs);
        // SAFETY: 16 slots of 8 bytes, as many as any method reads
        unsafe { kura::d3d11::call(id, a.as_ptr().cast::<u32>(), calls()) }
    } else {
        // SAFETY: the guest's stack arguments past the return address, mapped guest memory
        unsafe { kura::d3d11::call(id, (c.esp + 4) as usize as *const u32, calls()) }
    };
    // (a FLOAT method: the host's answer is the float's bits)
    if tag.float { c.ret_f32(f32::from_bits(r)) } else { r }
}

/// x64: the 8-byte slots the host reads, the method's `nargs` arguments then 0; a float among
/// the first four is XMM's (methods::FLOAT_ARGS), the others RCX RDX R8 R9 and the stack's.
fn slots64(c: &Ctx, tag: Tag, nargs: usize) -> [u64; 16] {
    let floats = methods::FLOAT_ARGS.iter().find(|f| f.0 == tag.iface && f.1 == tag.index).map_or(0, |f| f.2);
    std::array::from_fn(|i| match u32::try_from(i) {
        Ok(k) if i < nargs && k < 4 && floats >> k & 1 != 0 => c.xmm(k) & 0xffff_ffff,
        Ok(k) if i < nargs => c.a64(k),
        _ => 0,
    })
}

fn host(id: u32, args: &[u32]) -> u32 {
    init();
    if mem::x64() {
        let mut a = [0u64; 16];
        for (d, &s) in a.iter_mut().zip(args) {
            *d = u64::from(s);
        }
        return unsafe { kura::d3d11::call(id, a.as_ptr().cast::<u32>(), calls()) };
    }
    unsafe { kura::d3d11::call(id, args.as_ptr(), calls()) }
}

/// The vtables, given to the host once (Wine's d3d11.dll DllMain).
fn init() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        let n = u32::from(FIRST) + METHODS.len() as u32;
        let ps = mem::ps();
        let table = mem::heap_alloc(ps * n, true);
        for (i, m) in (FIRST..).zip(METHODS) {
            let v = mem::heap_alloc(ps * m.len() as u32, true);
            for (j, &args) in (0u8..).zip(m) {
                // (FLOAT methods take `this` and one more: ID3D11DeviceContext::GetResourceMinLOD)
                let float = args == 0;
                let args = if float { 2 } else { args };
                let bytes = 4 * u32::from(args);
                let a = host_fn(&format!("d3d11:{i}:{j}"), method, bytes);
                crate::api::set_tag(a, Tag { iface: i, index: j, float }.bits(), usize::from(args));
                if let Some((name, _)) = kura::d3d11::names::NAMES.get(usize::from(i.wrapping_sub(FIRST))) {
                    crate::api::set_com_outs(a, name, u32::from(j), crate::api::ComArgs::Words);
                }
                mem::wp(v + ps * u32::from(j), a);
            }
            mem::wp(table + ps * u32::from(i), v);
        }
        unsafe { kura::d3d11::init(table as usize as *const u32, n) };
    });
}

/// (adapter, driver type, software, flags, levels, count, sdk, swapchain desc, swapchain,
/// device, feature level, context)
fn D3D11CreateDeviceAndSwapChain(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::D3D11CreateDeviceAndSwapChain>();
    let a = crate::api::words![c.a::<0>(), c.a::<1>(), c.a::<2>(), c.a::<3>(), c.a::<4>(), c.a::<5>(), c.a::<6>(), c.a::<7>(), c.out::<8>(), c.out::<9>(), c.a::<10>(), c.out::<11>()];
    host(CREATE_DEVICE, &a)
}
fn D3D11CreateDevice(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::D3D11CreateDevice>();
    let a = crate::api::words![c.a::<0>(), c.a::<1>(), c.a::<2>(), c.a::<3>(), c.a::<4>(), c.a::<5>(), c.a::<6>(), 0, 0, c.out::<7>(), c.a::<8>(), c.out::<9>()];
    host(CREATE_DEVICE, &a)
}
/// (factory, adapter, flags, levels, count, device)
fn D3D11CoreCreateDevice(c: &mut Ctx) -> u32 {
    let a = [c.a(1), u32::from(c.a(1) == 0), 0, c.a(2), c.a(3), c.a(4), 7, 0, 0, c.a(5), 0, 0];
    host(CREATE_DEVICE, &a)
}
/// d3d10.dll / d3d10_1.dll: the D3D11 device's D3D10 face (crates/kura/src/d3d11/d3d10.rs)
/// (adapter, driver type, software, flags, sdk, out)
fn D3D10CreateDevice(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::D3D10CreateDevice>();
    host(CREATE_DEVICE10, &crate::api::words![c.a::<0>(), c.a::<1>(), c.a::<2>(), c.a::<3>(), 0xa000, c.a::<4>(), 0, 0, c.out::<5>()])
}
/// (adapter, driver type, software, flags, sdk, swap chain desc, swap chain, device)
fn D3D10CreateDeviceAndSwapChain(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::D3D10CreateDeviceAndSwapChain>();
    host(CREATE_DEVICE10, &crate::api::words![c.a::<0>(), c.a::<1>(), c.a::<2>(), c.a::<3>(), 0xa000, c.a::<4>(), c.a::<5>(), c.out::<6>(), c.out::<7>()])
}
/// (adapter, driver type, software, flags, level, sdk, out)
fn D3D10CreateDevice1(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::D3D10CreateDevice1>();
    host(CREATE_DEVICE10, &crate::api::words![c.a::<0>(), c.a::<1>(), c.a::<2>(), c.a::<3>(), c.a::<4>(), c.a::<5>(), 0, 0, c.out::<6>()])
}
/// (adapter, driver type, software, flags, level, sdk, swap chain desc, swap chain, device)
fn D3D10CreateDeviceAndSwapChain1(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::D3D10CreateDeviceAndSwapChain1>();
    let a = crate::api::words![c.a::<0>(), c.a::<1>(), c.a::<2>(), c.a::<3>(), c.a::<4>(), c.a::<5>(), c.a::<6>(), c.out::<7>(), c.out::<8>()];
    host(CREATE_DEVICE10, &a)
}
fn D3D11On12CreateDevice(_: &mut Ctx) -> u32 {
    E_NOTIMPL
}
fn CreateDXGIFactory(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::CreateDXGIFactory>();
    host(CREATE_FACTORY, &crate::api::words![0, c.a::<0>(), c.out::<1>()])
}
fn CreateDXGIFactory2(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::CreateDXGIFactory2>();
    host(CREATE_FACTORY, &crate::api::words![c.a::<0>(), c.a::<1>(), c.out::<2>()])
}
fn DXGIGetDebugInterface1(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::DXGIGetDebugInterface1>();
    c.out::<2>().put(0);
    E_NOTIMPL
}
/// d3d12.dll as on a PC whose GPU has no Direct3D 12 driver: it loads (Unity 6 calls into it
/// through a delay-load import with no __try around it: a missing d3d12.dll is c06d007e and
/// the player's crash, corpus dl_RJ01636335 and 15 more), and D3D12CreateDevice says the
/// adapter can't, so the program goes on with Direct3D 11.
/// (adapter, minimum level, riid, device**)
fn D3D12CreateDevice(c: &mut Ctx) -> u32 {
    if c.a(3) != 0 {
        mem::putp(c.a(3), 0);
    }
    DXGI_ERROR_UNSUPPORTED
}
/// (riid, out): no debug layer, no experimental features, no interfaces
fn d3d12_no_interface(c: &mut Ctx) -> u32 {
    if c.a(1) != 0 {
        mem::putp(c.a(1), 0);
    }
    E_NOINTERFACE
}
/// (desc, version, blob**, error blob**) and the deserializers (data, size, riid, out**)
fn d3d12_no_root_signature(_: &mut Ctx) -> u32 {
    E_NOTIMPL
}
const DXGI_ERROR_UNSUPPORTED: u32 = 0x887a_0004;
fn not_impl(_: &mut Ctx) -> u32 {
    E_NOTIMPL
}
fn zero(_: &mut Ctx) -> u32 {
    0
}

pub fn register() {
    implement("d3d11", "D3D11CreateDeviceAndSwapChain", D3D11CreateDeviceAndSwapChain);
    implement("d3d11", "D3D11CreateDevice", D3D11CreateDevice);
    implement("d3d11", "D3D11CoreCreateDevice", D3D11CoreCreateDevice);
    implement("d3d11", "D3D11On12CreateDevice", D3D11On12CreateDevice);
    for n in ["D3D11CoreCreateLayeredDevice", "D3D11CoreGetLayeredDeviceSize", "D3D11CoreRegisterLayers"] {
        implement("d3d11", n, not_impl);
    }
    implement("d3d10", "D3D10CreateDevice", D3D10CreateDevice);
    implement("d3d10", "D3D10CreateDeviceAndSwapChain", D3D10CreateDeviceAndSwapChain);
    implement("d3d10_1", "D3D10CreateDevice1", D3D10CreateDevice1);
    implement("d3d10_1", "D3D10CreateDeviceAndSwapChain1", D3D10CreateDeviceAndSwapChain1);
    implement("dxgi", "CreateDXGIFactory", CreateDXGIFactory);
    implement("dxgi", "CreateDXGIFactory1", CreateDXGIFactory);
    implement("dxgi", "CreateDXGIFactory2", CreateDXGIFactory2);
    implement("dxgi", "DXGIGetDebugInterface1", DXGIGetDebugInterface1);
    implement("dxgi", "DXGID3D10CreateDevice", not_impl);
    implement("dxgi", "DXGID3D10RegisterLayers", not_impl);
    implement("dxgi", "DXGIDeclareAdapterRemovalSupport", zero);
    implement("d3d12", "D3D12CreateDevice", D3D12CreateDevice);
    implement("d3d12", "D3D12GetDebugInterface", d3d12_no_interface);
    implement("d3d12", "D3D12GetInterface", |c| {
        // (clsid, riid, out)
        if c.a(2) != 0 {
            mem::putp(c.a(2), 0);
        }
        E_NOINTERFACE
    });
    implement("d3d12", "D3D12EnableExperimentalFeatures", |_| E_NOINTERFACE);
    for n in ["D3D12SerializeRootSignature", "D3D12SerializeVersionedRootSignature", "D3D12CreateRootSignatureDeserializer", "D3D12CreateVersionedRootSignatureDeserializer"] {
        implement("d3d12", n, d3d12_no_root_signature);
    }
}

/// Every D3D11/DXGI vtable trap carries its method id, FLOAT for the float ones, and its
/// argument count (method reads them without a lock: review C-2).
#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tag_tests {
    use super::*;

    #[test]
    fn traps_carry_their_ids() {
        let _g = crate::test_guest::enter();
        init();
        for (k, m) in METHODS.iter().enumerate() {
            let i = u32::from(FIRST) + k as u32;
            for (j, &args) in m.iter().enumerate() {
                let a = crate::api::export("sake", &format!("d3d11:{i}:{j}"));
                let (tag, n) = crate::api::tag_of(a);
                assert_eq!(tag & !FLOAT, i << 8 | j as u32);
                assert_eq!(tag & FLOAT != 0, args == 0);
                // (a FLOAT method takes `this` and its resource)
                assert_eq!(n, if args == 0 { 2 } else { usize::from(args) });
                assert_eq!(Tag::of(tag).map(Tag::bits), Some(tag));
            }
        }
        // (no other bits come back as a tag)
        for bits in [1 << 16, 1 << 24, 1 << 30, 0x4000_0000 | 0x4100, u32::MAX] {
            assert_eq!(Tag::of(bits), None, "{bits:#x}");
        }
    }
}

#[cfg(test)]
mod float_arg_tests {
    use super::*;

    /// The generated table names the methods that take a float by value (the depth of
    /// ClearDepthStencilView, SetResourceMinLOD's clamp), and x64 takes each from its XMM
    /// register, not the integer one at its place.
    #[test]
    fn x64_floats_come_from_xmm() {
        let name = |i: u8, j: u8| kura::d3d11::names::NAMES.get(usize::from(i - FIRST)).and_then(|(n, m)| m.get(usize::from(j)).map(|m| format!("{n}::{m}")));
        let named: Vec<String> = methods::FLOAT_ARGS.iter().filter_map(|f| name(f.0, f.1)).collect();
        assert_eq!(named, ["ID3D11DeviceContext4::ClearDepthStencilView", "ID3D11DeviceContext4::SetResourceMinLOD", "ID3D10Device1::ClearDepthStencilView"]);
        for &(iface, index, mask) in methods::FLOAT_ARGS {
            let mut cpu = koji::Cpu::default();
            for (k, r) in (0u32..).zip([koji::Reg64::Rcx, koji::Reg64::Rdx, koji::Reg64::R8, koji::Reg64::R9]) {
                cpu.set_gpr(r, 0x1111_0000 + u64::from(k));
                cpu.set_xmm(kome::num::us(k), 0xdead_beef_0000_0000_0000_0000_4000_0000u128 + u128::from(k));
            }
            let c = Ctx { cpu: &mut cpu, esp: 0, edx: None, jumped: false, st0: None, trap: 0, this: None, outs: 0 };
            let s = slots64(&c, Tag { iface, index, float: false }, 4);
            for k in 0..4u32 {
                let want = if mask >> k & 1 != 0 { 0x4000_0000 + u64::from(k) } else { 0x1111_0000 + u64::from(k) };
                assert_eq!(s.get(kome::num::us(k)).copied(), Some(want), "{:?} argument {k}", name(iface, index));
            }
        }
    }
}
