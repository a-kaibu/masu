//! A Windows RECT (left, top, right, bottom; right and bottom outside it) and the box arithmetic
//! sake's window manager and kura's display both do on it (they each had a copy).

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Rect {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

impl Rect {
    pub fn w(&self) -> i32 {
        self.r.wrapping_sub(self.l)
    }
    pub fn h(&self) -> i32 {
        self.b.wrapping_sub(self.t)
    }
    pub fn empty(&self) -> bool {
        self.l >= self.r || self.t >= self.b
    }
    /// the intersection (empty when they do not meet)
    pub fn and(&self, o: &Rect) -> Rect {
        Rect { l: self.l.max(o.l), t: self.t.max(o.t), r: self.r.min(o.r), b: self.b.min(o.b) }
    }
    /// the bounding box of both (an empty one adds nothing)
    pub fn or(&self, o: &Rect) -> Rect {
        if self.empty() {
            return *o;
        }
        if o.empty() {
            return *self;
        }
        Rect { l: self.l.min(o.l), t: self.t.min(o.t), r: self.r.max(o.r), b: self.b.max(o.b) }
    }
    pub fn off(&self, x: i32, y: i32) -> Rect {
        Rect { l: self.l.wrapping_add(x), t: self.t.wrapping_add(y), r: self.r.wrapping_add(x), b: self.b.wrapping_add(y) }
    }
    pub fn has(&self, x: i32, y: i32) -> bool {
        x >= self.l && x < self.r && y >= self.t && y < self.b
    }
    /// The bounding box of what is left of self once o is taken away
    pub fn minus(&self, o: &Rect) -> Rect {
        let mut r = *self;
        if r.empty() || o.l <= r.l && o.t <= r.t && o.r >= r.r && o.b >= r.b {
            return Rect::default();
        }
        if o.l <= r.l && o.r >= r.r {
            if o.t <= r.t && o.b > r.t {
                r.t = o.b;
            } else if o.b >= r.b && o.t < r.b {
                r.b = o.t;
            }
        } else if o.t <= r.t && o.b >= r.b {
            if o.l <= r.l && o.r > r.l {
                r.l = o.r;
            } else if o.r >= r.r && o.l < r.r {
                r.r = o.l;
            }
        }
        r
    }
}

#[cfg(test)]
#[path = "rect_tests.rs"]
mod tests;
