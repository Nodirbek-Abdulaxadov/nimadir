//! Tabs (ADR-5): one shown, the rest parked.
//!
//! The shown tab's `Shell` is the caller's own, so everything that drives
//! "the shell" drives the shown tab unchanged. This keeps the others —
//! each a `Shell` of its own, with whatever state the front end keeps per
//! tab (`U`: the window's widget buffers, the headless runner's typed text)
//! — and swaps one in when it is shown.
//!
//! A parked tab is frozen: nothing here calls its app. Parked apps together
//! may hold `PARKED_MEMORY` of guest memory; past it, the one shown least
//! recently is discarded — its app dropped, its title and address kept, and
//! opened afresh when it is shown again.

use crate::shell::Shell;

/// How many tabs there may be.
pub const MAX_TABS: usize = 8;
/// Guest memory the parked tabs may hold between them.
pub const PARKED_MEMORY: usize = 512 << 20;

/// `PARKED_MEMORY`, unless `NIMADIR_PARKED_MEMORY` (bytes) says less — for
/// the suite, which has no half a gigabyte to fill.
fn parked_limit() -> usize {
    std::env::var("NIMADIR_PARKED_MEMORY")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .map_or(PARKED_MEMORY, |v: usize| v.min(PARKED_MEMORY))
}

/// A tab that is not shown.
struct Parked<U> {
    shell: Shell,
    ui: U,
    /// When it was last shown, to discard the least recent first.
    seen: u64,
    /// What a discarded tab was: its title and address.
    discarded: Option<(String, String)>,
}

/// One tab as the strip shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    /// The page's title; empty for the start page, which the chrome names.
    pub title: String,
    pub shown: bool,
    /// A navigation in flight.
    pub loading: bool,
}

pub struct Tabs<U> {
    /// In the strip's order; the shown tab's place is `None`, its shell the
    /// caller's.
    slots: Vec<Option<Parked<U>>>,
    shown: usize,
    clock: u64,
}

impl<U: Default> Default for Tabs<U> {
    fn default() -> Self {
        Tabs {
            slots: vec![None],
            shown: 0,
            clock: 0,
        }
    }
}

impl<U: Default> Tabs<U> {
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// The shown tab's place in the strip, from 0.
    pub fn shown(&self) -> usize {
        self.shown
    }

    /// The strip, with `current` as the shown tab.
    pub fn labels(&self, current: &Shell) -> Vec<Label> {
        self.slots
            .iter()
            .map(|slot| match slot {
                None => label(current, None, true),
                Some(p) => label(&p.shell, p.discarded.as_ref(), false),
            })
            .collect()
    }

    /// A new tab on the start page, right of the shown one, and shown. False
    /// when there are `MAX_TABS` already.
    pub fn open(&mut self, current: &mut Shell, ui: &mut U) -> bool {
        if self.slots.len() >= MAX_TABS {
            return false;
        }
        let fresh = current.new_tab();
        self.park(current, ui, fresh, U::default());
        self.slots.insert(self.shown + 1, None);
        self.shown += 1;
        current.go_home();
        self.budget();
        true
    }

    /// Show the tab at `to`, parking the shown one.
    pub fn show(&mut self, current: &mut Shell, ui: &mut U, to: usize) {
        if to == self.shown || to >= self.slots.len() {
            return;
        }
        let Some(p) = self.slots[to].take() else {
            return;
        };
        self.park(current, ui, p.shell, p.ui);
        self.shown = to;
        if let Some((title, src)) = p.discarded {
            current.reopen(&title, &src);
        }
        self.budget();
    }

    /// The tab `by` places along, wrapping: Ctrl+Tab and Ctrl+Shift+Tab.
    pub fn cycle(&mut self, current: &mut Shell, ui: &mut U, by: isize) {
        let n = self.slots.len() as isize;
        let to = (self.shown as isize + by).rem_euclid(n) as usize;
        self.show(current, ui, to);
    }

    /// Close the tab at `at`, destroying its page. The last tab is never
    /// closed: it goes to the start page instead, afresh.
    pub fn close(&mut self, current: &mut Shell, ui: &mut U, at: usize) {
        if at >= self.slots.len() {
            return;
        }
        if self.slots.len() == 1 {
            *current = current.new_tab();
            *ui = U::default();
            current.go_home();
            return;
        }
        if at != self.shown {
            self.slots.remove(at);
            if at < self.shown {
                self.shown -= 1;
            }
            return;
        }
        // The shown one: its neighbour to the right takes its place, else
        // the one to its left.
        let next = if at + 1 < self.slots.len() {
            at + 1
        } else {
            at - 1
        };
        let p = self.slots[next].take().expect("a parked tab");
        *current = p.shell;
        *ui = p.ui;
        self.slots.remove(at);
        self.shown = if next > at { at } else { next };
        if let Some((title, src)) = p.discarded {
            current.reopen(&title, &src);
        }
    }

    /// Hand a registry that reached the shown tab to the parked ones.
    pub fn share_registry(&mut self, current: &Shell) {
        for p in self.slots.iter_mut().flatten() {
            p.shell.share_registry(current);
        }
    }

    /// Guest memory the parked tabs hold between them.
    pub fn parked_memory(&self) -> usize {
        self.slots
            .iter()
            .flatten()
            .map(|p| p.shell.memory_bytes())
            .sum()
    }

    /// Put `current` in the shown tab's place, and `next` where it was.
    fn park(&mut self, current: &mut Shell, ui: &mut U, next: Shell, next_ui: U) {
        self.clock += 1;
        let shell = std::mem::replace(current, next);
        let state = std::mem::replace(ui, next_ui);
        self.slots[self.shown] = Some(Parked {
            shell,
            ui: state,
            seen: self.clock,
            discarded: None,
        });
    }

    /// Discard parked tabs, least recently shown first, until they fit.
    fn budget(&mut self) {
        let limit = parked_limit();
        while self.parked_memory() > limit {
            let Some(oldest) = self
                .slots
                .iter_mut()
                .flatten()
                .filter(|p| p.discarded.is_none() && p.shell.memory_bytes() > 0)
                .min_by_key(|p| p.seen)
            else {
                return;
            };
            oldest.discarded = oldest.shell.discard();
            oldest.ui = U::default();
            crate::diag::line("a parked tab was discarded to keep within PARKED_MEMORY");
        }
    }
}

fn label(shell: &Shell, discarded: Option<&(String, String)>, shown: bool) -> Label {
    let title = match discarded {
        Some((title, _)) => title.clone(),
        None => shell
            .location()
            .map(|(title, _)| title.to_string())
            .unwrap_or_default(),
    };
    Label {
        title: if title == crate::shell::STORE_APP {
            String::new()
        } else {
            title
        },
        shown,
        loading: shell.loading().is_some(),
    }
}
