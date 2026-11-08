//! Bounded ambiguity: the live parse versions.
//!
//! A GLR parser may be exploring several stacks at once. Unbounded that is a
//! denial of service, so the runtime keeps [`MAX_LIVE`] versions plus
//! [`MAX_OVERFLOW`] of slack. Past that, the most expensive version is dropped
//! and the drop is counted, never silent.

use deep_abi::{NodeId, Offset};

/// Versions that are always allowed.
pub const MAX_LIVE: usize = 6;
/// Extra versions tolerated before something has to go.
pub const MAX_OVERFLOW: usize = 4;
/// The hard cap on versions in flight.
pub const MAX_VERSIONS: usize = MAX_LIVE + MAX_OVERFLOW;

/// Which recovery a fork is exploring. Only diagnostics, but it makes a failing
/// test say what died.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lane {
    Insert,
    Skip,
    Cascade,
}

/// One parse in progress.
#[derive(Clone, Debug)]
pub struct Version {
    /// Stack tops, all at the same input position.
    pub frontier: Vec<u32>,
    /// Index of the next token to consume.
    pub pos: usize,
    /// Errors so far. The drop rule compares this first.
    pub cost: u32,
    /// Inserted nodes not yet consumed by a reduction.
    pub pending: Vec<(NodeId, Offset, Offset)>,
    /// A skip run that has not become one ERROR node yet.
    pub skip: Option<(Offset, Offset)>,
    /// The node a successful parse ends on.
    pub root: NodeId,
    /// Which recovery produced this version.
    pub lane: Lane,
    pub dead: bool,
    pub accepted: bool,
    /// Consecutive missing-node inserts at the current `pos`.
    inserts_here: u32,
    /// The position this version last forked at, so one version cannot clone
    /// itself at the same error forever.
    forked_at: Option<usize>,
}

impl Version {
    pub fn new(frontier: Vec<u32>, pos: usize) -> Version {
        Version {
            frontier,
            pos,
            cost: 0,
            pending: Vec::new(),
            skip: None,
            root: deep_abi::NONE,
            lane: Lane::Skip,
            dead: false,
            accepted: false,
            inserts_here: 0,
            forked_at: None,
        }
    }

    pub fn fork(&self, lane: Lane) -> Version {
        let mut copy = self.clone();
        copy.lane = lane;
        copy
    }

    /// Consecutive inserts of missing input, per input position.
    pub fn inserts_here(&self) -> u32 {
        self.inserts_here
    }

    pub fn note_insert(&mut self) {
        self.inserts_here += 1;
    }

    pub fn reset_inserts(&mut self) {
        self.inserts_here = 0;
    }

    /// Whether this version may fork at the current error.
    pub fn may_fork(&self) -> bool {
        self.forked_at != Some(self.pos)
    }

    pub fn note_fork(&mut self) {
        self.forked_at = Some(self.pos);
    }

    pub fn forked_at_set(&mut self, pos: usize) {
        self.forked_at = Some(pos);
    }

    /// Cost ranking for the drop rule: errors dominate, then how far the
    /// version still has to read.
    fn rank(&self) -> (u32, usize) {
        (self.cost, self.pos)
    }
}

#[derive(Default)]
pub struct Set {
    versions: Vec<Version>,
    /// Times a version was refused or replaced at the hard cap.
    drops: u32,
    /// Times the set grew past the always-allowed six.
    overflow_events: u32,
    /// Most versions alive at one time.
    peak: usize,
}

impl Set {
    pub fn new() -> Set {
        Set::default()
    }

    pub fn len(&self) -> usize {
        self.versions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }

    pub fn peak(&self) -> usize {
        self.peak
    }

    /// The caller-visible counter: every branch the cap destroyed.
    pub fn drops(&self) -> u32 {
        self.drops
    }

    /// Every time the set held more than [`MAX_LIVE`] versions.
    pub fn overflow_events(&self) -> u32 {
        self.overflow_events
    }

    pub fn get(&self, at: usize) -> &Version {
        &self.versions[at]
    }

    pub fn get_mut(&mut self, at: usize) -> &mut Version {
        &mut self.versions[at]
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Version> {
        self.versions.iter()
    }

    /// Add a version, enforcing the caps.
    ///
    /// Below the hard cap nothing is lost. At the cap the highest-cost version
    /// is dropped for the newcomer when the newcomer is cheaper, and otherwise
    /// the newcomer is the one dropped; either way `drops` moves.
    pub fn push(&mut self, version: Version) {
        self.push_protected(version, usize::MAX)
    }

    /// `push`, but never evict the version at `protect` — the driver is holding
    /// that slot for the branch it is about to continue.
    ///
    /// At the hard cap the most expensive version loses. Ties go to the
    /// newcomer, which is what keeps a fork storm finite: a fresh alternative is
    /// discarded rather than replacing an established version with a copy of
    /// itself at an earlier position.
    pub fn push_protected(&mut self, version: Version, protect: usize) {
        if self.versions.len() < MAX_VERSIONS {
            self.versions.push(version);
        } else {
            let mut worst = self.versions.len();
            for at in 0..self.versions.len() {
                if at == protect {
                    continue;
                }
                let worse = match worst {
                    candidate if candidate == self.versions.len() => true,
                    candidate => self.versions[at].rank() > self.versions[candidate].rank(),
                };
                if worse {
                    worst = at;
                }
            }
            let replace = worst != self.versions.len()
                && version.rank() < self.versions[worst].rank();
            if replace {
                self.versions[worst] = version;
            }
            self.drops += 1;
        }
        if self.versions.len() > MAX_LIVE {
            self.overflow_events += 1;
        }
        if self.versions.len() > self.peak {
            self.peak = self.versions.len();
        }
    }

    /// Remove versions that are dead or already accepted, keeping at least one
    /// so the driver always has something to report.
    pub fn sweep(&mut self) {
        if self.versions.len() <= 1 {
            return;
        }
        if self.versions.iter().any(|version| !version.dead) {
            self.versions.retain(|version| !version.dead);
            return;
        }
        // Everything died. Keep the cheapest one so the caller still gets a
        // tree with error nodes instead of nothing at all.
        let mut best = 0usize;
        for at in 1..self.versions.len() {
            if self.versions[at].rank() < self.versions[best].rank() {
                best = at;
            }
        }
        let revived = self.versions.swap_remove(best);
        self.versions.clear();
        self.versions.push(revived);
        self.versions[0].dead = false;
    }

    /// The cheapest version that finished, or None.
    pub fn winner(&self) -> Option<usize> {
        let mut best: Option<usize> = None;
        for at in 0..self.versions.len() {
            let version = &self.versions[at];
            if !version.accepted || version.dead {
                continue;
            }
            match best {
                None => best = Some(at),
                Some(current) => {
                    if version.rank() < self.versions[current].rank() {
                        best = Some(at);
                    }
                }
            }
        }
        best
    }

    /// The cheapest live version at all, accepted or not.
    pub fn cheapest(&self) -> Option<usize> {
        let mut best: Option<usize> = None;
        for at in 0..self.versions.len() {
            if self.versions[at].dead {
                continue;
            }
            match best {
                None => best = Some(at),
                Some(current) => {
                    if self.versions[at].rank() < self.versions[current].rank() {
                        best = Some(at);
                    }
                }
            }
        }
        best
    }
}
