//! Where a resource's newest bytes are, as versions (review of 6852be1, stage 4): the guest's
//! copy and the page's copy each carry the version they hold, a write makes a new version on
//! its side, and a transfer moves the version it was asked for, only when it comes. A transfer
//! that fails changes nothing, so the newest bytes are never lost track of; and the states two
//! flags could contradict each other in (to upload, yet the page newer) cannot be written.

/// The versions of a resource's two copies.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Content {
    guest: u64,
    page: u64,
}

/// A readback asked for: the page's version it reads (one readback, one `read_back`).
#[must_use]
#[derive(Debug, PartialEq, Eq)]
pub struct Asked(u64);

impl Content {
    /// New bytes in the guest's memory, none on the page yet.
    pub const fn new() -> Content {
        Content { guest: 1, page: 0 }
    }

    fn next(&self) -> u64 {
        self.guest.max(self.page).saturating_add(1)
    }

    /// The guest wrote its copy: the newest, to go up before the page reads it.
    pub fn guest_wrote(&mut self) {
        self.guest = self.next();
    }

    /// The page wrote its copy (stream output, a shader's writes, a copy on the page): the
    /// newest, to come back before the guest reads it.
    pub fn page_wrote(&mut self) {
        self.page = self.next();
    }

    /// The guest's copy went up: the page holds its version.
    pub fn uploaded(&mut self) {
        self.page = self.page.max(self.guest);
    }

    /// The page's copy is gone (a lost device): the guest's is what there is, to go up again.
    pub fn page_lost(&mut self) {
        self.guest = self.guest.max(1);
        self.page = 0;
    }

    /// The guest's copy is newer than the page's.
    pub fn upload_needed(&self) -> bool {
        self.guest > self.page
    }

    /// The guest's copy is the newest (what it reads is the resource's bytes).
    pub fn guest_current(&self) -> bool {
        self.guest >= self.page
    }

    /// The readback the guest's copy needs before it is read (none: it is current).
    pub fn readback(&self) -> Option<Asked> {
        (self.page > self.guest).then_some(Asked(self.page))
    }

    /// A readback came: the guest holds the version it read, unless the page wrote again since
    /// it was asked for (then the page's stays the newer).
    pub fn read_back(&mut self, a: Asked) {
        self.guest = self.guest.max(a.0.min(self.page));
    }
}

impl Default for Content {
    fn default() -> Content {
        Content::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[derive(Clone, Copy, Debug)]
    enum Ev {
        Guest,
        Page,
        Upload,
        Lost,
        /// a readback asked for now and answered (true) or failed, after the page wrote again
        /// (second bool) or the guest did (third) or not
        Read(bool, bool, bool),
    }

    fn ev() -> impl Strategy<Value = Ev> {
        prop_oneof![Just(Ev::Guest), Just(Ev::Page), Just(Ev::Upload), Just(Ev::Lost), (any::<bool>(), any::<bool>(), any::<bool>()).prop_map(|(a, b, g)| Ev::Read(a, b, g))]
    }

    /// Where the newest bytes are, followed by hand: which side wrote last, and whether the
    /// other side has those bytes too.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Model {
        /// the guest's are the newest; the page has them too (true) or not
        Guest(bool),
        /// the page's are the newest; the guest has them too (true) or not
        Page(bool),
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        fn the_newest_bytes_are_never_lost_track_of(evs in proptest::collection::vec(ev(), 0..60)) {
            let mut c = Content::new();
            let mut m = Model::Guest(false);
            for e in evs {
                match e {
                    Ev::Guest => {
                        c.guest_wrote();
                        m = Model::Guest(false);
                    }
                    Ev::Page => {
                        c.page_wrote();
                        m = Model::Page(false);
                    }
                    Ev::Upload => {
                        // (an upload goes only when one is needed)
                        if c.upload_needed() {
                            c.uploaded();
                            m = Model::Guest(true);
                        }
                    }
                    Ev::Lost => {
                        c.page_lost();
                        m = Model::Guest(false);
                    }
                    Ev::Read(came, again, guest) => {
                        if let Some(a) = c.readback() {
                            if again {
                                c.page_wrote();
                                m = Model::Page(false);
                            }
                            if guest {
                                c.guest_wrote();
                                m = Model::Guest(false);
                            }
                            if came {
                                c.read_back(a);
                                if !again && !guest {
                                    m = Model::Page(true);
                                }
                            }
                        }
                    }
                }
                let (upload, current) = match m {
                    Model::Guest(both) => (!both, true),
                    Model::Page(both) => (false, both),
                };
                prop_assert_eq!(c.upload_needed(), upload, "{:?} after {:?}: {:?}", m, e, c);
                prop_assert_eq!(c.guest_current(), current, "{:?} after {:?}: {:?}", m, e, c);
                prop_assert_eq!(c.readback().is_some(), !current);
            }
        }
    }
}
