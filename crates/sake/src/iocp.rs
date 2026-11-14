#![allow(non_snake_case)]
//! IO completion ports and overlapped results (kernel32): CreateIoCompletionPort,
//! Get/PostQueuedCompletionStatus(Ex), GetOverlappedResult(Ex), CancelIo(Ex),
//! SetFileCompletionNotificationModes. Files complete their IO at once (ReadFile/WriteFile
//! post here when the handle has a port); sockets (ws2.rs) complete later too.
//!
//! OVERLAPPED { Internal (the NTSTATUS), InternalHigh (bytes), Offset, OffsetHigh, hEvent };
//! an hEvent with its low bit set asks for no completion packet.

use crate::api::{implement, Ctx};
use crate::k32::last_error;
use crate::mem::{self, ps, r32, rp, w32, wp};
use crate::obj::{self, Key, Obj};
use crate::HashMap;
use std::collections::VecDeque;
use std::sync::Mutex;

pub const STATUS_PENDING: u32 = 0x103;

/// (bytes, key, overlapped, NTSTATUS)
type Packet = (u32, u64, u32, u32);

/// A completion port (the object, through whichever of its handles): only `port_of` makes one,
/// from a handle that is a port.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Port(Key);

/// State per object (a file, socket or port), as Windows keeps it with the object: every handle
/// to it shares it.
#[derive(Default)]
struct State {
    ports: HashMap<Key, VecDeque<Packet>>,
    /// file -> (its port, completion key)
    assoc: HashMap<Key, (Port, u64)>,
    /// file -> FILE_SKIP_* flags
    modes: HashMap<Key, u32>,
    /// files opened with FILE_FLAG_OVERLAPPED
    overlapped: std::collections::HashSet<Key>,
    /// the completion keys of the files BindIoCompletionCallback gave a callback (their port's
    /// pool thread calls it)
    bound: HashMap<u64, u32>,
    /// ports whose last handle was closed while waits still hold them (signaled)
    closed: std::collections::HashSet<Key>,
}
static ST: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    f(ST.lock().unwrap().get_or_insert_with(State::default))
}

/// NTSTATUS -> Win32 error, for the statuses IO here ends with
pub fn nt_to_win32(s: u32) -> u32 {
    match s {
        0 => 0,
        STATUS_PENDING => 997,
        0xC0000120 => 995,  // STATUS_CANCELLED: ERROR_OPERATION_ABORTED
        0x80000005 => 234,  // STATUS_BUFFER_OVERFLOW: ERROR_MORE_DATA
        0xC000020D => 64,   // STATUS_CONNECTION_RESET: ERROR_NETNAME_DELETED
        0xC000020C => 64,   // STATUS_CONNECTION_DISCONNECTED
        0xC0000236 => 1225, // STATUS_CONNECTION_REFUSED
        0xC000023C => 1231, // STATUS_NETWORK_UNREACHABLE
        0xC00000B5 => 121,  // STATUS_IO_TIMEOUT: ERROR_SEM_TIMEOUT
        0xC0000241 => 1236, // STATUS_CONNECTION_ABORTED
        0xC000000D => 87,
        0xC0000140 => 1229, // STATUS_INVALID_CONNECTION
        0xC00000B0 => 233,  // STATUS_PIPE_DISCONNECTED
        0xC0000011 => 38,   // STATUS_END_OF_FILE: ERROR_HANDLE_EOF
        0xC000009A => 1450, // STATUS_INSUFFICIENT_RESOURCES
        0xC0000008 => 6,
        0xC0000022 => 5,
        0xC00000A3 => 21, // STATUS_DEVICE_NOT_READY
        0xC000020A => 52, // STATUS_ADDRESS_ALREADY_EXISTS: ERROR_DUP_NAME
        0xC0000234 => 1232,
        0xC0000023 => 122, // STATUS_BUFFER_TOO_SMALL: ERROR_INSUFFICIENT_BUFFER
        0xC00000BB => 50,  // STATUS_NOT_SUPPORTED
        s => s,
    }
}

/// Queues a packet on `port` and wakes its waiters
pub fn post(port: Port, bytes: u32, key: u64, ov: u32, status: u32) {
    // (a completion that raced the port's close is dropped: making the queue again would
    // keep a dead port's queue for good and tell its waiters it lives)
    let live = with(|s| s.ports.get_mut(&port.0).map(|q| q.push_back((bytes, key, ov, status))).is_some());
    if live {
        obj::signal();
    }
}

/// IO on handle `h` with OVERLAPPED `ov` ended (status, bytes): the OVERLAPPED gets them, the
/// event is set and a packet goes to the handle's port, as Windows does. `sync`: it ended
/// within the call (FILE_SKIP_COMPLETION_PORT_ON_SUCCESS skips the packet then).
pub fn complete(h: u32, ov: u32, status: u32, bytes: u32, sync: bool) {
    if ov == 0 {
        return;
    }
    let k = ps();
    wp(ov, status);
    wp(ov + k, bytes);
    let ev = rp(ov + 2 * k + 8);
    let file = obj::key_of(h);
    let (assoc, mode) = with(|s| file.map_or((None, 0), |f| (s.assoc.get(&f).copied(), s.modes.get(&f).copied().unwrap_or(0))));
    let mut told = false;
    if ev & !1 != 0 && !(sync && mode & 2 != 0) {
        crate::k32::set_event(ev & !1);
        told = true;
    }
    if let Some((port, key)) = assoc {
        if ev & 1 == 0 && !(sync && status == 0 && mode & 1 != 0) {
            post(port, bytes, key, ov, status);
            told = true;
        }
    }
    // (no event and no port: Windows signals the file handle, which GetOverlappedResult(bWait)
    // with hEvent NULL waits on; its wait here looks at the OVERLAPPED again when woken)
    if !told {
        obj::signal();
    }
}

/// Whether completing on `h` within the call skips the port (FILE_SKIP_COMPLETION_PORT_ON_SUCCESS)
pub fn skips_port(h: u32) -> bool {
    let Some(f) = obj::key_of(h) else { return false };
    with(|s| s.modes.get(&f).is_some_and(|m| m & 1 != 0) && s.assoc.contains_key(&f))
}

/// Drops what is kept for an object that went (obj::gone_object)
pub fn forget(k: Key) {
    with(|s| {
        if let Some((_, key)) = s.assoc.remove(&k) {
            s.bound.remove(&key);
        }
        s.modes.remove(&k);
        s.ports.remove(&k);
        s.overlapped.remove(&k);
    });
}

/// CreateFile with FILE_FLAG_OVERLAPPED made `h`
pub fn mark_overlapped(h: u32) {
    if let Some(f) = obj::key_of(h) {
        with(|s| s.overlapped.insert(f));
    }
}
/// Whether CreateFile made `h` with FILE_FLAG_OVERLAPPED (its IO keeps no file pointer)
pub fn is_overlapped(h: u32) -> bool {
    obj::key_of(h).is_some_and(|f| with(|s| s.overlapped.contains(&f)))
}

/// BindIoCompletionCallback(file, callback(error, bytes, overlapped), flags): the file's IO
/// completes on a pool thread that calls `callback`; the file must be overlapped and not
/// bound yet
fn BindIoCompletionCallback(c: &mut Ctx) -> u32 {
    let (h, f) = (c.a(0), c.a(1));
    if h == 0 || h == 0xffff_ffff {
        last_error(6);
        return 0;
    }
    let Some(file) = obj::key_of(h) else {
        last_error(6);
        return 0;
    };
    if f == 0 || !with(|s| s.overlapped.contains(&file)) || with(|s| s.assoc.contains_key(&file)) {
        last_error(87);
        return 0;
    }
    // (one port for every bound file, its key the file: one IO thread for all of them hands
    // each completion to a pool worker; each file used to have a port and a guest thread of
    // its own, waking once a second)
    let Some(port) = port_of(bind_port()) else {
        last_error(8);
        return 0;
    };
    // (the completion key is the file's first handle: the IO thread finds its callback by it)
    let key = u64::from(file.first_handle());
    with(|s| {
        s.assoc.insert(file, (port, key));
        s.bound.insert(key, f);
    });
    1
}

/// The port of BindIoCompletionCallback's files, its IO thread started (0: no thread).
fn bind_port() -> u32 {
    static PORT: Mutex<u32> = Mutex::new(0);
    static ENTRY: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    let mut p = PORT.lock().unwrap();
    if *p == 0 {
        let port = new_port();
        let entry = *ENTRY.get_or_init(|| crate::api::host_fn("pool_io_thread", bind_io_thread, 4));
        if crate::spawn_service(entry, port).is_none() {
            obj::close(port);
            return 0;
        }
        *p = port;
    }
    *p
}

/// BindIoCompletionCallback's IO thread: callback(Win32 error, bytes, overlapped) of each
/// packet's file, on a pool worker
fn bind_io_thread(c: &mut Ctx) -> u32 {
    let port = c.a(0);
    loop {
        match wait_packet(c, port, u32::MAX, false) {
            Ok(((bytes, key, ov, status), _)) => {
                let Some(f) = with(|s| s.bound.get(&key).copied()) else { continue };
                let err = nt_to_win32(status);
                crate::pool::submit(Box::new(move |c| {
                    c.call(f, &[err, bytes, ov]);
                }));
            }
            Err(258) => {}
            Err(_) => return 0,
        }
    }
}

/// A port handle was closed: its packets go, its waiters end (ERROR_ABANDONED_WAIT_0)
/// The port's last handle was closed: its waiters end (GetQueuedCompletionStatus:
/// ERROR_ABANDONED_WAIT_0; a wait on the port: signaled)
pub fn closed(k: Key) {
    with(|s| {
        s.ports.remove(&k);
        s.assoc.retain(|_, (p, _)| p.0 != k);
        s.closed.insert(k);
    });
    obj::signal();
}
/// The port object went (the last wait on it ended too)
pub fn gone(k: Key) {
    with(|s| {
        s.ports.remove(&k);
        s.assoc.retain(|_, (p, _)| p.0 != k);
        s.closed.remove(&k);
    });
}

/// The port and key handle `h`'s file is tied to
pub fn association(h: u32) -> Option<(Port, u64)> {
    let f = obj::key_of(h)?;
    with(|s| s.assoc.get(&f).copied())
}

/// Whether a port has packets (a port handle is signaled then)
pub fn has_packets(k: Key) -> bool {
    with(|s| s.closed.contains(&k) || s.ports.get(&k).is_some_and(|q| !q.is_empty()))
}

/// A new port's handle
pub fn new_port() -> u32 {
    let p = obj::add(Obj::Other(crate::obj::Other::Iocp));
    if let Some(k) = obj::key_of(p) {
        with(|s| s.ports.insert(k, VecDeque::new()));
    }
    p
}

/// The port handle `h` is to (None: not a port)
pub fn port_of(h: u32) -> Option<Port> {
    obj::with(|t| match t.objs.get(&obj::norm(h)) {
        Some((Obj::Other(crate::obj::Other::Iocp), _)) => t.objs.key(&obj::norm(h)).map(Port),
        _ => None,
    })
}

/// CreateIoCompletionPort(file, port, key, threads)
fn CreateIoCompletionPort(c: &mut Ctx) -> u32 {
    let (file, port, key) = (c.a(0), c.a(1), if mem::x64() { c.a64(2) } else { c.a(2) as u64 });
    let invalid = file == u32::MAX;
    if invalid && port != 0 {
        last_error(87);
        return 0;
    }
    if port != 0 && port_of(port).is_none() {
        last_error(6);
        return 0;
    }
    let f = obj::key_of(file);
    if !invalid && f.is_none() {
        last_error(6);
        return 0;
    }
    let port = if port == 0 {
        new_port()
    } else {
        port
    };
    if let (Some(f), Some(p)) = (f, port_of(port)) {
        with(|s| s.assoc.insert(f, (p, key)));
    }
    port
}

fn take(port: Port) -> Option<Packet> {
    with(|s| s.ports.get_mut(&port.0).and_then(|q| q.pop_front()))
}

/// Waits for a packet on `port` (or an APC when alertable): Ok(packet), Err(win32 error)
fn wait_packet(c: &mut Ctx, port: u32, ms: u32, alertable: bool) -> Result<(Packet, Port), u32> {
    let Some(port) = port_of(port) else { return Err(6) };
    let tid = crate::cur().tid;
    let deadline = (ms != u32::MAX).then(|| std::time::Instant::now() + std::time::Duration::from_millis(ms as u64));
    loop {
        // (AFD polls end here: a socket that changed is looked at by the port's waiter)
        crate::afd::check();
        if let Some(p) = take(port) {
            return Ok((p, port));
        }
        // (closed while waiting)
        if !with(|s| s.ports.contains_key(&port.0)) {
            return Err(735); // ERROR_ABANDONED_WAIT_0
        }
        if alertable && crate::apc::run(c) {
            return Err(crate::apc::WAIT_IO_COMPLETION);
        }
        let left = match deadline {
            None => u32::MAX,
            Some(d) => {
                let now = std::time::Instant::now();
                if now >= d {
                    return Err(258); // WAIT_TIMEOUT
                }
                (d - now).as_millis().max(1) as u32
            }
        };
        let ready = || with(|s| s.ports.get(&port.0).is_none_or(|q| !q.is_empty())) || (alertable && crate::apc::pending(tid));
        // (AFD polls waiting: ws2 ends them as sockets change; a look now and then besides, as a
        // safety net: every 10 ms kept an idle network program's waiters from ever sleeping)
        let left = if crate::afd::pending() { left.min(250) } else { left };
        obj::wait(&[], false, left, Some(&ready));
    }
}

/// GetQueuedCompletionStatus(port, &bytes, &key, &overlapped, ms)
fn GetQueuedCompletionStatus(c: &mut Ctx) -> u32 {
    let (port, bp, kp, op, ms) = (c.a(0), c.a(1), c.a(2), c.a(3), c.a(4));
    match wait_packet(c, port, ms, false) {
        Ok(((bytes, key, ov, status), _)) => {
            mem::put32(bp, bytes);
            if kp != 0 {
                if mem::x64() {
                    mem::w64(kp, key);
                } else {
                    w32(kp, key as u32);
                }
            }
            if op != 0 {
                wp(op, ov);
            }
            if status != 0 {
                last_error(nt_to_win32(status));
                return 0;
            }
            1
        }
        Err(e) => {
            if op != 0 {
                wp(op, 0);
            }
            last_error(e);
            0
        }
    }
}

/// GetQueuedCompletionStatusEx(port, entries, count, &removed, ms, alertable):
/// OVERLAPPED_ENTRY { key, overlapped, Internal, bytes }
fn GetQueuedCompletionStatusEx(c: &mut Ctx) -> u32 {
    let (port, ents, count, rp_, ms, alertable) = (c.a(0), c.a(1), c.a(2), c.a(3), c.a(4), c.a(5) != 0);
    if count == 0 || ents == 0 {
        last_error(87);
        return 0;
    }
    let k = ps();
    match wait_packet(c, port, ms, alertable) {
        Ok((first, port)) => {
            let mut n = 0;
            let mut p = Some(first);
            while let Some((bytes, key, ov, status)) = p {
                let e = ents + 4 * k * n;
                if mem::x64() {
                    mem::w64(e, key);
                } else {
                    w32(e, key as u32);
                }
                wp(e + k, ov);
                wp(e + 2 * k, status);
                w32(e + 3 * k, bytes);
                n += 1;
                p = if n < count { take(port) } else { None };
            }
            mem::put32(rp_, n);
            1
        }
        Err(e) => {
            mem::put32(rp_, 0);
            last_error(e);
            0
        }
    }
}

/// PostQueuedCompletionStatus(port, bytes, key, overlapped)
fn PostQueuedCompletionStatus(c: &mut Ctx) -> u32 {
    let (port, bytes, key, ov) = (c.a(0), c.a(1), if mem::x64() { c.a64(2) } else { c.a(2) as u64 }, c.a(3));
    let Some(port) = port_of(port) else {
        last_error(6);
        return 0;
    };
    post(port, bytes, key, ov, 0);
    1
}

/// Waits until the OVERLAPPED is no longer pending (on its event when it has one)
pub fn wait_overlapped(c: &mut Ctx, ov: u32, ms: u32, alertable: bool) -> u32 {
    let k = ps();
    let ev = rp(ov + 2 * k + 8) & !1;
    if rp(ov) != STATUS_PENDING {
        return 0;
    }
    let done = || rp(ov) != STATUS_PENDING;
    let evs = [ev];
    let hs: &[u32] = if ev != 0 { &evs } else { &[] };
    let r = if alertable {
        crate::apc::wait(c, hs, false, ms)
    } else if ev != 0 {
        obj::wait(&[ev], false, ms, Some(&done))
    } else {
        obj::wait(&[], false, ms, Some(&done))
    };
    if rp(ov) != STATUS_PENDING {
        0
    } else {
        r
    }
}

/// GetOverlappedResult(h, ov, &bytes, wait) / GetOverlappedResultEx(h, ov, &bytes, ms, alertable)
fn overlapped_result(c: &mut Ctx, ex: bool) -> u32 {
    let (ov, bp) = (c.a(1), c.a(2));
    let (ms, alertable) = if ex { (c.a(3), c.a(4) != 0) } else { (if c.a(3) != 0 { u32::MAX } else { 0 }, false) };
    if ov == 0 {
        last_error(87);
        return 0;
    }
    if rp(ov) == STATUS_PENDING {
        if ms == 0 {
            last_error(996); // ERROR_IO_INCOMPLETE
            return 0;
        }
        let r = wait_overlapped(c, ov, ms, alertable);
        if rp(ov) == STATUS_PENDING {
            last_error(match r {
                crate::apc::WAIT_IO_COMPLETION => crate::apc::WAIT_IO_COMPLETION,
                0x102 => 258,
                _ => 996,
            });
            return 0;
        }
    }
    let k = ps();
    mem::put32(bp, rp(ov + k));
    let s = rp(ov);
    if s != 0 {
        last_error(nt_to_win32(s));
        return 0;
    }
    // (success clears the last error: tests/winref/golden/pipes-32)
    last_error(0);
    1
}

/// A wait_packet error as an NTSTATUS
fn status_of(e: u32) -> u32 {
    match e {
        735 => 0x80,  // STATUS_ABANDONED_WAIT_0
        258 => 0x102, // STATUS_TIMEOUT
        6 => 0xc000_0008,
        e => e,
    }
}

/// ntdll's completion calls: NtCreateIoCompletion(&h, access, attr, threads),
/// NtSetIoCompletion(port, key, value, status, information),
/// NtRemoveIoCompletion(port, &key, &value, iosb, timeout),
/// NtRemoveIoCompletionEx(port, FILE_IO_COMPLETION_INFORMATION[], count, &removed, timeout,
/// alertable): { key, value, iosb { status, information } }
pub fn init_nt() {
    let d = "ntdll";
    implement(d, "NtCreateIoCompletion", |c| {
        if c.a(0) == 0 {
            return 0xc000_0005;
        }
        wp(c.a(0), new_port());
        0
    });
    let set = |c: &mut Ctx| {
        let (port, key, value, status, info) = (c.a(0), if mem::x64() { c.a64(1) } else { c.a(1) as u64 }, c.a(2), c.a(3), c.a(4));
        let Some(port) = port_of(port) else { return 0xc000_0008 };
        post(port, info, key, value, status);
        0
    };
    implement(d, "NtSetIoCompletion", set);
    implement(d, "NtRemoveIoCompletion", |c| {
        let (port, pkey, pval, iosb, ms) = (c.a(0), c.a(1), c.a(2), c.a(3), crate::ntsync::timeout_ms(c.a(4)));
        match wait_packet(c, port, ms, false) {
            Ok(((bytes, key, ov, status), _)) => {
                wp(pkey, key as u32);
                if mem::x64() {
                    mem::w64(pkey, key);
                }
                wp(pval, ov);
                wp(iosb, status);
                wp(iosb + ps(), bytes);
                0
            }
            Err(e) => status_of(e),
        }
    });
    implement(d, "NtRemoveIoCompletionEx", |c| {
        let (port, info, count, removed, ms, alertable) = (c.a(0), c.a(1), c.a(2), c.a(3), crate::ntsync::timeout_ms(c.a(4)), c.a(5) & 0xff != 0);
        let k = ps();
        match wait_packet(c, port, ms, alertable) {
            Ok((first, port)) => {
                let (mut n, mut p) = (0, Some(first));
                while let Some((bytes, key, ov, status)) = p {
                    let e = info + 4 * k * n;
                    wp(e, key as u32);
                    if mem::x64() {
                        mem::w64(e, key);
                    }
                    wp(e + k, ov);
                    wp(e + 2 * k, status);
                    wp(e + 3 * k, bytes);
                    n += 1;
                    p = if n < count { take(port) } else { None };
                }
                mem::put32(removed, n);
                0
            }
            Err(e) => {
                mem::put32(removed, 0);
                if e == crate::apc::WAIT_IO_COMPLETION { 0xc0 } else { status_of(e) }
            }
        }
    });
}

pub fn init() {
    let d = "kernel32";
    implement(d, "CreateIoCompletionPort", CreateIoCompletionPort);
    implement(d, "BindIoCompletionCallback", BindIoCompletionCallback);
    implement(d, "GetQueuedCompletionStatus", GetQueuedCompletionStatus);
    implement(d, "GetQueuedCompletionStatusEx", GetQueuedCompletionStatusEx);
    implement(d, "PostQueuedCompletionStatus", PostQueuedCompletionStatus);
    implement(d, "GetOverlappedResult", |c| overlapped_result(c, false));
    implement(d, "GetOverlappedResultEx", |c| overlapped_result(c, true));
    // CancelIo(h) / CancelIoEx(h, ov): sockets' and pipes' IO waits
    implement(d, "CancelIo", |c| {
        let h = c.a(0);
        crate::ws2::cancel(h, 0, true);
        crate::pipe::cancel_io(h, 0, true);
        1
    });
    implement(d, "CancelIoEx", |c| {
        let (h, ov) = (c.a(0), c.a(1));
        if crate::ws2::cancel(h, ov, false) | crate::pipe::cancel_io(h, ov, false) {
            1
        } else {
            last_error(1168); // ERROR_NOT_FOUND
            0
        }
    });
    implement(d, "SetFileCompletionNotificationModes", |c| {
        let (h, f) = (c.a(0), c.a(1));
        if f & !3 != 0 {
            last_error(87);
            return 0;
        }
        // (only a handle that is one: a garbage value kept here would hand its FILE_SKIP_*
        // flags to the next real handle of that number)
        let Some(file) = obj::key_of(h) else {
            last_error(6);
            return 0;
        };
        with(|s| *s.modes.entry(file).or_default() |= f);
        1
    });
    let _ = r32;
}

/// Ports and associations through any handle to the object (review: they were kept by handle
/// number, so a DuplicateHandle'd port or file was another, unknown one), and what one handle
/// does pinned before the tables are keyed by obj::Key.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_guest::{call, enter, guest_thread};
    use proptest::prelude::*;

    fn port() -> u32 {
        call(CreateIoCompletionPort, &[u32::MAX, 0, 0, 1])
    }
    /// GetQueuedCompletionStatus(port, 0 ms): Some((bytes, key, overlapped)) or the error
    fn get(port: u32) -> Result<(u32, u32, u32), u32> {
        let out = mem::heap_alloc(12, true);
        let ok = call(GetQueuedCompletionStatus, &[port, out, out + 4, out + 8, 0]);
        let r = if ok != 0 { Ok((r32(out), r32(out + 4), r32(out + 8))) } else { Err(crate::k32::get_last_error()) };
        mem::heap_free(out);
        r
    }
    fn post_(port: u32, bytes: u32, key: u32, ov: u32) -> u32 {
        call(PostQueuedCompletionStatus, &[port, bytes, key, ov])
    }
    /// An OVERLAPPED without an event
    fn overlapped() -> u32 {
        mem::heap_alloc(4 * ps() + 8, true)
    }

    #[test]
    fn a_port_queues_in_order_and_times_out_empty() {
        let _g = enter();
        guest_thread();
        let p = port();
        assert_ne!(p, 0);
        assert_eq!(get(p), Err(258));
        assert_eq!(post_(p, 5, 7, 0x1234), 1);
        assert_eq!(post_(p, 6, 8, 0x1238), 1);
        assert_eq!(get(p), Ok((5, 7, 0x1234)));
        assert_eq!(get(p), Ok((6, 8, 0x1238)));
        assert_eq!(get(p), Err(258));
        assert_eq!(post_(0x7777_0000, 1, 1, 1), 0, "not a port");
        assert!(obj::close(p));
    }

    #[test]
    fn a_duplicated_port_is_the_same_port() {
        let _g = enter();
        guest_thread();
        let p = port();
        let d = obj::dup(p);
        assert_eq!(post_(d, 1, 2, 3), 1);
        assert_eq!(get(p), Ok((1, 2, 3)));
        assert_eq!(post_(p, 4, 5, 6), 1);
        // (a wait on either handle sees the packet)
        assert_eq!(obj::wait(&[d], false, 0, None), 0);
        assert_eq!(get(d), Ok((4, 5, 6)));
        // (the first handle closed: the port lives on in the other)
        assert!(obj::close(p));
        assert_eq!(post_(d, 7, 8, 9), 1);
        assert_eq!(get(d), Ok((7, 8, 9)));
        let k = obj::key_of(d).unwrap();
        assert!(obj::close(d));
        assert!(with(|s| !s.ports.contains_key(&k) && !s.closed.contains(&k)), "the port's state went with its last handle");
    }

    #[test]
    fn a_files_association_holds_through_any_of_its_handles() {
        let _g = enter();
        guest_thread();
        let f = obj::add(Obj::Other(crate::obj::Other::Nul));
        let p = call(CreateIoCompletionPort, &[f, 0, 0x55, 1]);
        assert_ne!(p, 0);
        let d = obj::dup(f);
        let ov = overlapped();
        complete(d, ov, 0, 12, false);
        assert_eq!(get(p), Ok((12, 0x55, ov)));
        // (the file's first handle closed: its other still completes on the port)
        assert!(obj::close(f));
        complete(d, ov, 0, 13, false);
        assert_eq!(get(p), Ok((13, 0x55, ov)));
        let k = obj::key_of(d).unwrap();
        assert!(obj::close(d));
        assert!(with(|s| !s.assoc.contains_key(&k)), "the association went with the file");
        assert!(obj::close(p));
        mem::heap_free(ov);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        /// Posts and gets through random handles of one port against a queue: the packets come
        /// out in order whichever handle posted or gets them.
        #[test]
        fn packets_through_any_handle_keep_their_order(ops in proptest::collection::vec((any::<bool>(), 0usize..3, 0u32..1000), 1..40)) {
            let _g = enter();
            guest_thread();
            let p = port();
            let hs = [p, obj::dup(p), obj::dup(p)];
            let mut model = std::collections::VecDeque::new();
            for (is_post, which, v) in ops {
                let h = hs[which];
                if is_post {
                    prop_assert_eq!(post_(h, v, v + 1, v + 2), 1);
                    model.push_back((v, v + 1, v + 2));
                } else {
                    prop_assert_eq!(get(h), model.pop_front().ok_or(258));
                }
            }
            let k = obj::key_of(p).unwrap();
            for h in hs {
                prop_assert!(obj::close(h));
            }
            prop_assert!(with(|s| !s.ports.contains_key(&k)));
        }
    }
}
