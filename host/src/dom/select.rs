//! Whether an element matches a selector — for the cascade, and for
//! `query-all`.
//!
//! Matching runs right to left, from the element up and back through its
//! siblings, and a descendant combinator may have to try every ancestor. A
//! guest writes the selectors, so two things bound it: a selector has at most
//! `MAX_COMPOUNDS` parts (longer ones are refused when read), and one match
//! may take `MAX_STEPS` steps before it is given up as no match.

use std::cell::Cell;
use std::collections::HashMap;

use super::css::{AttrOp, Combinator, Compound, Pseudo, Selector};
use super::{Document, Element, Kind, NodeId, ROOT};

/// The most compound selectors one selector may chain: `a b c … ` 32 deep.
pub const MAX_COMPOUNDS: usize = 32;

/// The most elements one match may visit.
const MAX_STEPS: u32 = 10_000;

/// What the user is doing to the page, for `:hover`, `:active` and `:focus`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    pub hovered: Option<NodeId>,
    pub pressed: Option<NodeId>,
    pub focused: Option<NodeId>,
}

/// Each element's place among its parent's element children — worked out
/// once per styling, so `:nth-child` over a long list is not quadratic.
#[derive(Default)]
pub struct Places(HashMap<NodeId, (usize, usize)>);

impl Places {
    pub fn of(doc: &Document) -> Self {
        let mut places = HashMap::new();
        for node in doc.nodes.values() {
            let elements: Vec<NodeId> = node
                .children
                .iter()
                .copied()
                .filter(|c| doc.nodes.get(c).is_some_and(|n| n.element().is_some()))
                .collect();
            let count = elements.len();
            for (i, c) in elements.into_iter().enumerate() {
                places.insert(c, (i, count));
            }
        }
        Places(places)
    }

    /// Index among element siblings, and how many there are.
    fn get(&self, id: NodeId) -> (usize, usize) {
        self.0.get(&id).copied().unwrap_or((0, 1))
    }
}

pub struct Matcher<'a> {
    pub doc: &'a Document,
    pub state: State,
    pub places: &'a Places,
    steps: Cell<u32>,
}

impl<'a> Matcher<'a> {
    pub fn new(doc: &'a Document, state: State, places: &'a Places) -> Self {
        Matcher {
            doc,
            state,
            places,
            steps: Cell::new(0),
        }
    }

    pub fn matches(&self, sel: &Selector, id: NodeId) -> bool {
        if sel.compounds.len() > MAX_COMPOUNDS {
            return false;
        }
        self.steps.set(0);
        self.from(sel, 0, id)
    }

    fn from(&self, sel: &Selector, i: usize, id: NodeId) -> bool {
        let steps = self.steps.get() + 1;
        self.steps.set(steps);
        if steps > MAX_STEPS || !self.compound(&sel.compounds[i], id) {
            return false;
        }
        if i + 1 == sel.compounds.len() {
            return true;
        }
        match sel.combinators[i] {
            Combinator::Child => self
                .parent_element(id)
                .is_some_and(|p| self.from(sel, i + 1, p)),
            Combinator::Descendant => {
                let mut at = self.parent_element(id);
                while let Some(a) = at {
                    if self.from(sel, i + 1, a) {
                        return true;
                    }
                    if self.steps.get() > MAX_STEPS {
                        return false;
                    }
                    at = self.parent_element(a);
                }
                false
            }
            Combinator::Next => self
                .previous_element(id)
                .is_some_and(|p| self.from(sel, i + 1, p)),
            Combinator::Later => {
                let mut at = self.previous_element(id);
                while let Some(p) = at {
                    if self.from(sel, i + 1, p) {
                        return true;
                    }
                    if self.steps.get() > MAX_STEPS {
                        return false;
                    }
                    at = self.previous_element(p);
                }
                false
            }
        }
    }

    fn element(&self, id: NodeId) -> Option<&'a Element> {
        self.doc.nodes.get(&id)?.element()
    }

    fn parent_element(&self, id: NodeId) -> Option<NodeId> {
        let p = self.doc.nodes.get(&id)?.parent?;
        self.element(p).map(|_| p)
    }

    fn previous_element(&self, id: NodeId) -> Option<NodeId> {
        let parent = self.doc.nodes.get(&id)?.parent?;
        let siblings = &self.doc.nodes.get(&parent)?.children;
        let at = siblings.iter().position(|&c| c == id)?;
        siblings[..at]
            .iter()
            .rev()
            .copied()
            .find(|&c| self.element(c).is_some())
    }

    /// Whether `id` is `ancestor` or under it.
    fn within(&self, mut id: NodeId, ancestor: NodeId) -> bool {
        loop {
            if id == ancestor {
                return true;
            }
            match self.doc.nodes.get(&id).and_then(|n| n.parent) {
                Some(p) => id = p,
                None => return false,
            }
        }
    }

    pub fn compound(&self, c: &Compound, id: NodeId) -> bool {
        let Some(e) = self.element(id) else {
            return false;
        };
        if c.tag.as_ref().is_some_and(|t| *t != e.tag) {
            return false;
        }
        if c.id.as_ref().is_some_and(|want| e.attr("id") != Some(want)) {
            return false;
        }
        if !c.classes.iter().all(|cl| e.has_class(cl)) {
            return false;
        }
        if !c.attrs.iter().all(|a| attr_matches(e, &a.name, &a.op)) {
            return false;
        }
        c.pseudos.iter().all(|p| self.pseudo(p, id, e))
    }

    fn pseudo(&self, p: &Pseudo, id: NodeId, e: &Element) -> bool {
        match p {
            Pseudo::Hover => self.state.hovered.is_some_and(|h| self.within(h, id)),
            Pseudo::Active => self.state.pressed.is_some_and(|h| self.within(h, id)),
            Pseudo::Focus => self.state.focused == Some(id),
            Pseudo::FocusWithin => self.state.focused.is_some_and(|f| self.within(f, id)),
            Pseudo::Disabled => self.disabled(id),
            Pseudo::Enabled => e.is_focusable() && !self.disabled(id),
            Pseudo::Checked => e.is_toggle() && e.checked,
            Pseudo::Root => id == ROOT,
            Pseudo::Empty => self.doc.nodes.get(&id).is_some_and(|n| {
                n.children.iter().all(|c| {
                    matches!(self.doc.nodes.get(c).map(|n| &n.kind), Some(Kind::Text(t)) if t.is_empty())
                })
            }),
            Pseudo::FirstChild => self.places.get(id).0 == 0,
            Pseudo::LastChild => {
                let (i, n) = self.places.get(id);
                i + 1 == n
            }
            Pseudo::OnlyChild => self.places.get(id).1 == 1,
            Pseudo::NthChild(a, b) => nth_matches(*a, *b, self.places.get(id).0 as i32 + 1),
            Pseudo::NthLastChild(a, b) => {
                let (i, n) = self.places.get(id);
                nth_matches(*a, *b, (n - i) as i32)
            }
            Pseudo::Not(inner) => !inner.iter().any(|c| self.compound(c, id)),
            Pseudo::Placeholder => e.is_field() && e.value.is_empty() && e.attr("placeholder").is_some(),
        }
    }

    /// Disabled itself, or inside something disabled.
    fn disabled(&self, mut id: NodeId) -> bool {
        loop {
            if self
                .element(id)
                .is_some_and(|e| e.attr("disabled").is_some())
            {
                return true;
            }
            match self.doc.nodes.get(&id).and_then(|n| n.parent) {
                Some(p) => id = p,
                None => return false,
            }
        }
    }
}

/// Whether position `pos` (from 1) is `a·n + b` for some n ≥ 0.
fn nth_matches(a: i32, b: i32, pos: i32) -> bool {
    if a == 0 {
        return pos == b;
    }
    let n = pos - b;
    n % a == 0 && n / a >= 0
}

fn attr_matches(e: &Element, name: &str, op: &AttrOp) -> bool {
    let Some(v) = e.attr(name) else {
        return false;
    };
    match op {
        AttrOp::Exists => true,
        AttrOp::Equals(w) => v == w,
        AttrOp::Word(w) => v.split_whitespace().any(|x| x == w),
        AttrOp::Dash(w) => {
            v == w
                || v.strip_prefix(w.as_str())
                    .is_some_and(|r| r.starts_with('-'))
        }
        AttrOp::Prefix(w) => !w.is_empty() && v.starts_with(w.as_str()),
        AttrOp::Suffix(w) => !w.is_empty() && v.ends_with(w.as_str()),
        AttrOp::Contains(w) => !w.is_empty() && v.contains(w.as_str()),
    }
}
