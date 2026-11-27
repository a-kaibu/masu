//! uxtheme paint buffers backed by ordinary GDI memory DCs and bitmaps.

use super::*;
use crate::hresult::{E_FAIL, E_INVALIDARG, E_POINTER};

struct Paint {
    dc: u32,
    bitmap: u32,
    target: u32,
    region: Rect,
    exclude: Option<Rect>,
    blend: Option<u32>,
}

static BUFFERS: Mutex<Option<HashMap<u32, Paint>>> = Mutex::new(None);

fn with_buffers<R>(f: impl FnOnce(&mut HashMap<u32, Paint>) -> R) -> R {
    let mut buffers = BUFFERS.lock().unwrap_or_else(|e| e.into_inner());
    f(buffers.get_or_insert_with(HashMap::default))
}

fn new_bitmap_for_paint(target: u32, region: Rect, format: u32) -> Option<Bitmap> {
    if format == 0 {
        return new_bitmap_raw(region.w(), region.h(), compatible_bitmap_depth(target), 0);
    }
    let bpp = if format == 3 { 1 } else { 32 };
    let (stride, size) = image_size(region.w(), region.h(), u32::from(bpp), 4)?;
    let bits = mem::alloc(size.max(4))?;
    mem::fill(bits, size, 0);
    Some(Bitmap {
        w: region.w(),
        h: region.h(),
        bpp,
        bits,
        stride,
        bottom_up: format == 1,
        pal: if bpp == 1 { vec![0, 0xffffff] } else { Vec::new() },
        r565: false,
        dib: true,
        own: 2,
    })
}

fn BeginBufferedPaint(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::BeginBufferedPaint>();
    let out = c.out::<4>();
    out.put(0);
    let (target, ptr, format, params) = (c.a::<0>(), c.a::<1>(), c.a::<2>(), c.a::<3>());
    if out.is_null() || format > 3 || dc_state(target).is_none() || mem::range(ptr, 16).is_none() {
        return 0;
    }
    let region = Rect::read(ptr);
    if region.empty() || region.r.checked_sub(region.l).is_none() || region.b.checked_sub(region.t).is_none() {
        return 0;
    }
    let (mut exclude, mut blend) = (None, None);
    if params != 0 {
        let size = 8 + 2 * mem::ps();
        if mem::range(params, size).is_none() || r32(params) != size {
            return 0;
        }
        let rect = mem::rp(params + 8);
        let function = mem::rp(params + 8 + mem::ps());
        if rect != 0 {
            if mem::range(rect, 16).is_none() { return 0; }
            exclude = Some(Rect::read(rect).and(&region));
        }
        if function != 0 {
            if mem::range(function, 4).is_none() { return 0; }
            blend = Some(r32(function));
        }
    }
    let id = mem::heap_alloc(mem::ps(), true);
    if id == 0 {
        return 0;
    }
    let Some(bitmap) = new_bitmap_for_paint(target, region, format) else {
        mem::heap_free(id);
        return 0;
    };
    let bitmap = add(G::Bitmap(bitmap));
    let dc = new_dc(Kind::Mem, None);
    with_dc(dc, |d| {
        d.s.bitmap = bitmap;
        d.s.win = (region.l, region.t);
    });
    with_buffers(|buffers| buffers.insert(id, Paint { dc, bitmap, target, region, exclude, blend }));
    out.put_addr(dc);
    id
}

impl Paint {
    fn copy_rect(&self, rect: Rect) {
        if rect.empty() {
            return;
        }
        let area = (rect.l, rect.t, rect.w(), rect.h());
        if let Some(blend) = self.blend {
            alpha_blend(self.target, area, self.dc, area, blend);
        } else {
            bit_blt(self.target, area, self.dc, area, 0x00cc_0020); // SRCCOPY
        }
    }

    fn copy_to_target(&self) {
        let rect = self.region;
        if let Some(exclude) = self.exclude.filter(|r| !r.empty()) {
            for piece in [
                Rect { b: exclude.t, ..rect },
                Rect { t: exclude.b, ..rect },
                Rect { t: exclude.t, b: exclude.b, r: exclude.l, ..rect },
                Rect { t: exclude.t, b: exclude.b, l: exclude.r, ..rect },
            ] {
                self.copy_rect(piece);
            }
        } else {
            self.copy_rect(rect);
        }
    }
}

fn EndBufferedPaint(c: &mut Ctx) -> u32 {
    let id = c.a(0);
    let Some(paint) = with_buffers(|buffers| buffers.remove(&id)) else { return E_INVALIDARG };
    if c.a(1) != 0 {
        paint.copy_to_target();
    }
    delete_dc(paint.dc);
    with(|t| {
        if let Some(G::Bitmap(bitmap)) = t.take(paint.bitmap) {
            free_bitmap(&bitmap);
        }
    });
    mem::heap_free(id);
    0
}

fn GetBufferedPaintDC(c: &mut Ctx) -> u32 {
    with_buffers(|buffers| buffers.get(&c.a(0)).map_or(0, |p| p.dc))
}

fn GetBufferedPaintTargetDC(c: &mut Ctx) -> u32 {
    with_buffers(|buffers| buffers.get(&c.a(0)).map_or(0, |p| p.target))
}

fn GetBufferedPaintTargetRect(c: &mut Ctx) -> u32 {
    let rect = c.a(1);
    if mem::range(rect, 16).is_none() {
        return E_POINTER;
    }
    let Some(region) = with_buffers(|buffers| buffers.get(&c.a(0)).map(|p| p.region)) else { return E_FAIL };
    region.write(rect);
    0
}

fn GetBufferedPaintBits(c: &mut Ctx) -> u32 {
    let c = &mut c.args::<crate::api::outs::GetBufferedPaintBits>();
    let (id, out, width) = (c.a::<0>(), c.out::<1>(), c.a::<2>());
    if out.is_null() || mem::range(width, 4).is_none() {
        return E_POINTER;
    }
    with_buffers(|buffers| {
        let Some(paint) = buffers.get(&id) else { return E_FAIL };
        with(|t| {
            let Some(G::Bitmap(bitmap)) = t.m.get(&paint.bitmap) else { return E_FAIL };
            if !bitmap.dib {
                return E_FAIL;
            }
            out.put_addr(bitmap.bits);
            let row_pixels = if bitmap.bpp == 1 { bitmap.stride * 8 } else { bitmap.stride / 4 };
            w32(width, row_pixels);
            0
        })
    })
}

fn BufferedPaintClear(c: &mut Ctx) -> u32 {
    let (id, rect) = (c.a(0), c.a(1));
    if rect != 0 && mem::range(rect, 16).is_none() {
        return E_POINTER;
    }
    with_buffers(|buffers| {
        let Some(paint) = buffers.get(&id) else { return E_FAIL };
        let region = paint.region;
        let area = if rect == 0 { region } else { region.and(&Rect::read(rect)) };
        with(|t| {
            let Some(G::Bitmap(bitmap)) = t.m.get(&paint.bitmap) else { return E_FAIL };
            let view = View::of(bitmap);
            for y in area.t..area.b {
                for x in area.l..area.r {
                    view.put(x - region.l, y - region.t, 0);
                }
            }
            0
        })
    })
}

pub(super) fn register() {
    reg!("uxtheme";
        BeginBufferedPaint, EndBufferedPaint, GetBufferedPaintDC, GetBufferedPaintTargetDC,
        GetBufferedPaintTargetRect, GetBufferedPaintBits, BufferedPaintClear,
    );
}
