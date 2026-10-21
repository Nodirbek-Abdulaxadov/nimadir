//! The native element tree (ADR-4): what `interface dom` builds and edits.
//!
//! The guest creates elements and text, nests them and sets their attributes;
//! the host keeps the tree between frames, so only the edits cross the
//! boundary. Nothing here knows about a renderer: styling, layout and the
//! display list are computed from this tree in the core, and `gui.rs` paints
//! what they produce.
//!
//! The tree outlives every frame, so it is bounded as a whole rather than per
//! frame: `MAX_NODES`, `MAX_BYTES`, `MAX_DEPTH`. Going past one is a `Limit`,
//! which `host.rs` turns into a trap with a sentence, like overspending a
//! frame. Anything else a guest gets wrong — a number that is not a node, a
//! node put inside itself — is ignored: a stale number is a bug in the guest,
//! not worth stopping it for.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

pub mod css;
pub mod layout;
pub mod paint;
pub mod select;
pub mod style;
pub mod values;

use css::{Env, Sheet};
use select::{Matcher, Places, State};
use style::Style;

pub type NodeId = u32;

/// The page: a `body` the host makes, always there, never removed.
pub const ROOT: NodeId = 1;

/// The most nodes one document may hold: 65 536, as many as a frame's widgets.
pub const MAX_NODES: usize = 1 << 16;

/// The most text, attributes and styles one document may hold: 32 MiB. Each
/// node also counts `NODE_COST` for itself, so a million empty ones are not
/// free.
pub const MAX_BYTES: usize = 32 << 20;

/// How deep the tree may go: 256 levels. Styling and layout walk it
/// recursively, and a guest is not trusted with the depth of the host's stack.
pub const MAX_DEPTH: usize = 256;

/// The most events waiting for the guest: past it the oldest are dropped. A
/// guest that never reads them should not grow the host.
pub const MAX_EVENTS: usize = 1024;

/// What a node costs against `MAX_BYTES` before its strings.
const NODE_COST: usize = 64;

/// Which of the document's limits a guest went past.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Limit {
    Nodes,
    Bytes,
    Depth,
}

impl Limit {
    /// What went wrong, for a person: `host::explain` puts it after "it".
    pub fn sentence(self) -> String {
        match self {
            Limit::Nodes => format!("built a page of more than {MAX_NODES} nodes"),
            Limit::Bytes => format!(
                "built a page holding more than {} MiB of text, attributes and styles",
                MAX_BYTES >> 20
            ),
            Limit::Depth => format!("nested its page more than {MAX_DEPTH} levels deep"),
        }
    }
}

/// Something the user did, waiting for the guest's `events()`.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub kind: String,
    pub target: NodeId,
    pub current: NodeId,
    pub value: String,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Element(Element),
    Text(String),
}

#[derive(Clone, Debug, Default)]
pub struct Element {
    /// Lowercase. One the host has no meaning for is kept as written — a
    /// selector may still name it — and behaves as a `div`.
    pub tag: String,
    /// In the order first set; names lowercase.
    pub attrs: Vec<(String, String)>,
    /// From the `class` attribute, kept split for selector matching.
    pub classes: Vec<String>,
    /// Inline style, one property at a time, in the order set. The `style`
    /// attribute replaces all of it.
    pub style: Vec<(String, String)>,
    /// The event kinds asked for on this node.
    pub listeners: Vec<String>,
    /// A control's value as the user has left it: a field's text.
    pub value: String,
    /// A checkbox's or radio's state as the user has left it.
    pub checked: bool,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub kind: Kind,
}

impl Node {
    pub fn element(&self) -> Option<&Element> {
        match &self.kind {
            Kind::Element(e) => Some(e),
            Kind::Text(_) => None,
        }
    }

    fn element_mut(&mut self) -> Option<&mut Element> {
        match &mut self.kind {
            Kind::Element(e) => Some(e),
            Kind::Text(_) => None,
        }
    }

    /// What this node counts against `MAX_BYTES`.
    fn cost(&self) -> usize {
        NODE_COST
            + match &self.kind {
                Kind::Text(t) => t.len(),
                Kind::Element(e) => {
                    e.tag.len()
                        + e.value.len()
                        + pairs(&e.attrs)
                        + pairs(&e.style)
                        + e.listeners.iter().map(String::len).sum::<usize>()
                }
            }
    }
}

fn element_of(kind: &mut Kind) -> Option<&mut Element> {
    match kind {
        Kind::Element(e) => Some(e),
        Kind::Text(_) => None,
    }
}

fn pairs(p: &[(String, String)]) -> usize {
    p.iter().map(|(k, v)| k.len() + v.len()).sum()
}

/// The tree, its events, and what the user is doing to it.
pub struct Document {
    nodes: HashMap<NodeId, Node>,
    next_id: NodeId,
    bytes: usize,
    events: VecDeque<Event>,
    /// The control that has the keyboard.
    pub focused: Option<NodeId>,
    /// Where a link the user followed points: the shell takes it after the
    /// next frame, as it takes an `open-app`.
    pending_open: Option<String>,
    /// Bumped by every change that could change how the page looks, so a
    /// renderer knows when what it computed is stale.
    generation: u64,
    /// The app's stylesheets, by number, in the order added — read when
    /// added, with what each cost against `MAX_BYTES`.
    sheets: Vec<(u32, usize, Arc<Sheet>)>,
    next_sheet: u32,
    /// The element under the pointer, for `:hover`.
    pub hovered: Option<NodeId>,
    /// The element being pressed, for `:active`.
    pub pressed: Option<NodeId>,
    /// The last styling, and what it was computed for.
    styled: Option<Styled>,
    /// The last layout, and what it was computed for.
    laid_out: Option<LaidOut>,
    /// How far each scroll container has been scrolled, x and y. Clamped
    /// to what its content allows when drawn; drawing alone depends on it.
    pub scroll: HashMap<NodeId, (f32, f32)>,
}

struct LaidOut {
    generation: u64,
    env: Env,
    state: State,
    layout: layout::Layout,
}

/// Every element's computed style, and the document, page and pointer it
/// was computed for — any of which changing makes it stale.
struct Styled {
    generation: u64,
    env: Env,
    state: State,
    styles: HashMap<NodeId, Style>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    pub fn new() -> Self {
        let mut doc = Document {
            nodes: HashMap::new(),
            next_id: ROOT,
            bytes: 0,
            events: VecDeque::new(),
            focused: None,
            pending_open: None,
            generation: 0,
            sheets: Vec::new(),
            next_sheet: 1,
            hovered: None,
            pressed: None,
            styled: None,
            laid_out: None,
            scroll: HashMap::new(),
        };
        let root = doc
            .create_element("body")
            .expect("an empty document has room for its root");
        debug_assert_eq!(root, ROOT);
        doc
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the page has anything to show.
    pub fn is_empty(&self) -> bool {
        self.nodes.get(&ROOT).is_none_or(|r| r.children.is_empty())
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn changed(&mut self) {
        self.generation += 1;
    }

    /// Charge `add` bytes against the document, refunding `sub`, or say
    /// which limit that would pass. Called before the change it pays for.
    fn charge(&mut self, add: usize, sub: usize) -> Result<(), Limit> {
        let after = (self.bytes + add).saturating_sub(sub);
        if after > MAX_BYTES {
            return Err(Limit::Bytes);
        }
        self.bytes = after;
        Ok(())
    }

    /// Change one node, paying for whatever it grows by — or, if that would
    /// pass `MAX_BYTES`, leaving it as it was.
    fn edit<R>(&mut self, id: NodeId, f: impl FnOnce(&mut Kind) -> R) -> Result<Option<R>, Limit> {
        let Some(node) = self.nodes.get_mut(&id) else {
            return Ok(None);
        };
        let before = node.cost();
        let saved = node.kind.clone();
        let out = f(&mut node.kind);
        let after = node.cost();
        if let Err(limit) = self.charge(after, before) {
            self.nodes.get_mut(&id).expect("just edited").kind = saved;
            return Err(limit);
        }
        self.changed();
        Ok(Some(out))
    }

    fn insert(&mut self, kind: Kind) -> Result<NodeId, Limit> {
        if self.nodes.len() >= MAX_NODES {
            return Err(Limit::Nodes);
        }
        let node = Node {
            parent: None,
            children: Vec::new(),
            kind,
        };
        self.charge(node.cost(), 0)?;
        let id = self.next_id;
        // Four billion nodes made in one run: the numbers are spent, and a
        // reused one would point the guest's old handle at someone else.
        self.next_id = self.next_id.checked_add(1).ok_or(Limit::Nodes)?;
        self.nodes.insert(id, node);
        Ok(id)
    }

    pub fn create_element(&mut self, tag: &str) -> Result<NodeId, Limit> {
        self.insert(Kind::Element(Element {
            tag: tag.trim().to_ascii_lowercase(),
            ..Element::default()
        }))
    }

    pub fn create_text(&mut self, text: &str) -> Result<NodeId, Limit> {
        self.insert(Kind::Text(text.to_string()))
    }

    /// How many levels down from its tree's top `id` is: 0 for a top.
    fn depth(&self, mut id: NodeId) -> usize {
        let mut d = 0;
        while let Some(p) = self.nodes.get(&id).and_then(|n| n.parent) {
            d += 1;
            id = p;
        }
        d
    }

    /// How many levels `id` and what is under it span: 1 for a leaf.
    fn height(&self, id: NodeId) -> usize {
        let mut deepest = 0;
        let mut stack = vec![(id, 1usize)];
        while let Some((n, h)) = stack.pop() {
            deepest = deepest.max(h);
            if let Some(node) = self.nodes.get(&n) {
                stack.extend(node.children.iter().map(|&c| (c, h + 1)));
            }
        }
        deepest
    }

    fn is_ancestor_or_self(&self, ancestor: NodeId, mut id: NodeId) -> bool {
        loop {
            if id == ancestor {
                return true;
            }
            match self.nodes.get(&id).and_then(|n| n.parent) {
                Some(p) => id = p,
                None => return false,
            }
        }
    }

    /// Take `id` from under its parent, if it has one.
    fn detach(&mut self, id: NodeId) {
        let Some(parent) = self.nodes.get(&id).and_then(|n| n.parent) else {
            return;
        };
        if let Some(p) = self.nodes.get_mut(&parent) {
            p.children.retain(|&c| c != id);
        }
        if let Some(n) = self.nodes.get_mut(&id) {
            n.parent = None;
        }
        if self
            .focused
            .is_some_and(|f| self.is_ancestor_or_self(id, f))
        {
            self.focused = None;
        }
        self.changed();
    }

    /// Put `child` under `parent`, before `before` if that is one of
    /// `parent`'s children, last otherwise. Ignored for numbers that are not
    /// nodes, a parent that is text, the root, and a node put inside itself.
    pub fn insert_before(
        &mut self,
        parent: NodeId,
        child: NodeId,
        before: Option<NodeId>,
    ) -> Result<(), Limit> {
        let parent_ok = self
            .nodes
            .get(&parent)
            .is_some_and(|p| p.element().is_some());
        if !parent_ok
            || child == ROOT
            || !self.nodes.contains_key(&child)
            || self.is_ancestor_or_self(child, parent)
        {
            return Ok(());
        }
        if self.depth(parent) + self.height(child) >= MAX_DEPTH {
            return Err(Limit::Depth);
        }
        self.detach(child);
        let siblings = &mut self.nodes.get_mut(&parent).expect("checked above").children;
        let at = before
            .filter(|&b| b != child)
            .and_then(|b| siblings.iter().position(|&c| c == b))
            .unwrap_or(siblings.len());
        siblings.insert(at, child);
        self.nodes.get_mut(&child).expect("checked above").parent = Some(parent);
        self.changed();
        Ok(())
    }

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<(), Limit> {
        self.insert_before(parent, child, None)
    }

    /// Out of the tree, kept.
    pub fn remove(&mut self, id: NodeId) {
        if id != ROOT {
            self.detach(id);
        }
    }

    /// Out of the tree and forgotten, with everything under it.
    pub fn release(&mut self, id: NodeId) {
        if id == ROOT || !self.nodes.contains_key(&id) {
            return;
        }
        self.detach(id);
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            if let Some(node) = self.nodes.remove(&n) {
                self.bytes = self.bytes.saturating_sub(node.cost());
                stack.extend(node.children);
            }
        }
        self.events
            .retain(|e| self.nodes.contains_key(&e.target) && self.nodes.contains_key(&e.current));
        self.changed();
    }

    /// A text node's text; an element's children replaced by this text.
    pub fn set_text(&mut self, id: NodeId, text: &str) -> Result<(), Limit> {
        match self.nodes.get(&id).map(|n| &n.kind) {
            Some(Kind::Text(_)) => {
                self.edit(id, |k| *k = Kind::Text(text.to_string()))?;
            }
            Some(Kind::Element(_)) => {
                let children = self.nodes[&id].children.clone();
                for c in children {
                    self.release(c);
                }
                if !text.is_empty() {
                    let t = self.create_text(text)?;
                    self.append_child(id, t)?;
                }
                self.changed();
            }
            None => {}
        }
        Ok(())
    }

    pub fn set_attribute(&mut self, id: NodeId, name: &str, value: &str) -> Result<(), Limit> {
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() {
            return Ok(());
        }
        let radio_group = self.edit(id, |k| {
            let e = element_of(k)?;
            match e.attrs.iter_mut().find(|(k, _)| *k == name) {
                Some((_, v)) => *v = value.to_string(),
                None => e.attrs.push((name.clone(), value.to_string())),
            }
            match name.as_str() {
                "class" => e.classes = split_classes(value),
                "style" => e.style = parse_inline_style(value),
                "value" => e.value = value.to_string(),
                "checked" => {
                    e.checked = truthy(value);
                    if e.checked && e.is_radio() {
                        return e.attr("name").map(str::to_string);
                    }
                }
                _ => {}
            }
            None
        })?;
        if let Some(Some(group)) = radio_group {
            self.uncheck_radios(&group, id);
        }
        Ok(())
    }

    pub fn remove_attribute(&mut self, id: NodeId, name: &str) -> Result<(), Limit> {
        let name = name.trim().to_ascii_lowercase();
        self.edit(id, |k| {
            if let Some(e) = element_of(k) {
                e.attrs.retain(|(k, _)| *k != name);
                match name.as_str() {
                    "class" => e.classes.clear(),
                    "style" => e.style.clear(),
                    "value" => e.value.clear(),
                    "checked" => e.checked = false,
                    _ => {}
                }
            }
        })?;
        Ok(())
    }

    pub fn get_attribute(&self, id: NodeId, name: &str) -> Option<String> {
        let name = name.trim().to_ascii_lowercase();
        self.nodes
            .get(&id)?
            .element()?
            .attr(&name)
            .map(str::to_string)
    }

    /// Kept in step with the `class` attribute, as on the web.
    pub fn toggle_class(&mut self, id: NodeId, name: &str, on: bool) -> Result<(), Limit> {
        let name = name.trim();
        let Some(e) = self.nodes.get(&id).and_then(Node::element) else {
            return Ok(());
        };
        if name.is_empty() || name.contains(char::is_whitespace) || e.has_class(name) == on {
            return Ok(());
        }
        let mut classes = e.classes.clone();
        if on {
            classes.push(name.to_string());
        } else {
            classes.retain(|c| c != name);
        }
        self.set_attribute(id, "class", &classes.join(" "))
    }

    /// One inline property; an empty value removes it.
    pub fn set_style(&mut self, id: NodeId, property: &str, value: &str) -> Result<(), Limit> {
        let property = property.trim().to_ascii_lowercase();
        let value = value.trim();
        if property.is_empty() {
            return Ok(());
        }
        self.edit(id, |k| {
            if let Some(e) = element_of(k) {
                e.style.retain(|(k, _)| *k != property);
                if !value.is_empty() {
                    e.style.push((property.clone(), value.to_string()));
                }
                let css = e
                    .style
                    .iter()
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                match e.attrs.iter_mut().find(|(k, _)| k == "style") {
                    Some((_, v)) => *v = css,
                    None => e.attrs.push(("style".into(), css)),
                }
            }
        })?;
        Ok(())
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes.get(&id)?.parent
    }

    pub fn children(&self, id: NodeId) -> Vec<NodeId> {
        self.nodes
            .get(&id)
            .map(|n| n.children.clone())
            .unwrap_or_default()
    }

    /// A control's value, a checkbox's state, any other node's text.
    pub fn value(&self, id: NodeId) -> String {
        match self.nodes.get(&id).map(|n| &n.kind) {
            Some(Kind::Element(e)) if e.is_toggle() => e.checked.to_string(),
            Some(Kind::Element(e)) if e.is_field() => e.value.clone(),
            Some(_) => self.text_content(id),
            None => String::new(),
        }
    }

    /// The text of `id` and everything under it, in document order.
    pub fn text_content(&self, id: NodeId) -> String {
        let mut out = String::new();
        let mut stack = vec![id];
        while let Some(n) = stack.pop() {
            match self.nodes.get(&n) {
                Some(Node {
                    kind: Kind::Text(t),
                    ..
                }) => out.push_str(t),
                Some(node) => stack.extend(node.children.iter().rev()),
                None => {}
            }
        }
        out
    }

    /// The user put the keyboard in `id`: the control that had it is told
    /// `blur`, and `id` is told `focus`.
    pub fn user_focus(&mut self, id: NodeId) {
        if self.focused == Some(id) {
            return;
        }
        if let Some(old) = self.focused.take() {
            self.dispatch("blur", old, "", 0.0, 0.0);
        }
        self.focused = Some(id);
        self.dispatch("focus", id, "", 0.0, 0.0);
        self.changed();
    }

    /// The user took the keyboard out of `id`.
    pub fn user_blur(&mut self, id: NodeId) {
        if self.focused == Some(id) {
            self.focused = None;
            self.dispatch("blur", id, "", 0.0, 0.0);
            self.changed();
        }
    }

    /// The scroll containers on the page, each with the part of its padding
    /// box that shows, by the last layout.
    pub fn scroll_areas(&self) -> Vec<(NodeId, layout::Rect)> {
        match (&self.styled, &self.laid_out) {
            (Some(s), Some(l)) => paint::scroll_areas(self, &s.styles, &l.layout),
            _ => Vec::new(),
        }
    }

    /// The keyboard goes to a control in the tree; anything else is ignored.
    pub fn focus(&mut self, id: NodeId) {
        let focusable = self
            .nodes
            .get(&id)
            .and_then(Node::element)
            .is_some_and(|e| e.is_focusable())
            && self.is_ancestor_or_self(ROOT, id);
        if focusable && self.focused != Some(id) {
            self.focused = Some(id);
            self.changed();
        }
    }

    pub fn listen(&mut self, id: NodeId, kind: &str) -> Result<(), Limit> {
        let kind = kind.trim().to_ascii_lowercase();
        if kind.is_empty() {
            return Ok(());
        }
        self.edit(id, |k| {
            if let Some(e) = element_of(k) {
                if !e.listeners.contains(&kind) {
                    e.listeners.push(kind);
                }
            }
        })?;
        Ok(())
    }

    pub fn unlisten(&mut self, id: NodeId, kind: &str) -> Result<(), Limit> {
        let kind = kind.trim().to_ascii_lowercase();
        self.edit(id, |k| {
            if let Some(e) = element_of(k) {
                e.listeners.retain(|k| *k != kind);
            }
        })?;
        Ok(())
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }

    pub fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    pub fn take_pending_open(&mut self) -> Option<String> {
        self.pending_open.take()
    }

    /// Whether a link the user followed is waiting for the next frame.
    pub fn wants_navigation(&self) -> bool {
        self.pending_open.is_some()
    }

    /// An element's computed style, from the last styling.
    pub fn computed(&self, id: NodeId) -> Option<&Style> {
        self.styled.as_ref()?.styles.get(&id)
    }

    /// Queue `kind` for the nearest of `target` and its ancestors that
    /// listens for it; nothing, if none does.
    pub fn dispatch(&mut self, kind: &str, target: NodeId, value: &str, x: f32, y: f32) {
        let mut at = Some(target);
        while let Some(id) = at {
            let Some(node) = self.nodes.get(&id) else {
                return;
            };
            if node
                .element()
                .is_some_and(|e| e.listeners.iter().any(|k| k == kind))
            {
                if self.events.len() >= MAX_EVENTS {
                    self.events.pop_front();
                }
                self.events.push_back(Event {
                    kind: kind.to_string(),
                    target,
                    current: id,
                    value: value.to_string(),
                    x,
                    y,
                });
                return;
            }
            at = node.parent;
        }
    }

    /// The nearest element at or above `id`, or `None` for a node that is
    /// gone or inside something disabled.
    fn live_element(&self, id: NodeId) -> Option<NodeId> {
        let mut at = Some(id);
        let mut element = None;
        while let Some(n) = at {
            let node = self.nodes.get(&n)?;
            if let Some(e) = node.element() {
                if e.attr("disabled").is_some() {
                    return None;
                }
                element.get_or_insert(n);
            }
            at = node.parent;
        }
        element
    }

    /// The user tapped `id`: the click, and what a click on that element
    /// does by itself — a checkbox flips, a radio is picked, a link is
    /// followed, a field takes the keyboard.
    pub fn click(&mut self, id: NodeId, x: f32, y: f32) {
        let Some(id) = self.live_element(id) else {
            return;
        };
        self.dispatch("click", id, "", x, y);
        // The element that does something about a click: this one, or the
        // nearest link or control around it.
        let mut at = Some(id);
        while let Some(n) = at {
            let Some(e) = self.nodes.get(&n).and_then(Node::element).cloned() else {
                break;
            };
            if e.is_checkbox() {
                self.toggle(n, !e.checked);
                return;
            }
            if e.is_radio() {
                if !e.checked {
                    self.toggle(n, true);
                }
                return;
            }
            if e.is_field() {
                self.focus(n);
                return;
            }
            if e.tag == "a" {
                if let Some(href) = e.attr("href").filter(|h| !h.trim().is_empty()) {
                    self.pending_open = Some(href.trim().to_string());
                }
                return;
            }
            if e.tag == "button" {
                return;
            }
            at = self.nodes.get(&n).and_then(|node| node.parent);
        }
    }

    /// A checkbox or radio set to `on` by the user, and told about it.
    fn toggle(&mut self, id: NodeId, on: bool) {
        let group = match self.nodes.get_mut(&id).and_then(Node::element_mut) {
            Some(e) => {
                e.checked = on;
                e.is_radio()
                    .then(|| e.attr("name").map(str::to_string))
                    .flatten()
            }
            None => return,
        };
        if on {
            if let Some(group) = group {
                self.uncheck_radios(&group, id);
            }
        }
        self.changed();
        let value = on.to_string();
        self.dispatch("input", id, &value, 0.0, 0.0);
        self.dispatch("change", id, &value, 0.0, 0.0);
    }

    /// Every other radio named `group` is no longer the one picked.
    fn uncheck_radios(&mut self, group: &str, except: NodeId) {
        for (&id, node) in self.nodes.iter_mut() {
            if id == except {
                continue;
            }
            if let Some(e) = node.element_mut() {
                if e.is_radio() && e.attr("name") == Some(group) {
                    e.checked = false;
                }
            }
        }
        self.changed();
    }

    /// The user changed a field's text to `text`.
    pub fn type_text(&mut self, id: NodeId, text: &str) -> Result<(), Limit> {
        let Some(id) = self.live_element(id) else {
            return Ok(());
        };
        let is_field = self
            .nodes
            .get(&id)
            .and_then(Node::element)
            .is_some_and(Element::is_field);
        if !is_field {
            return Ok(());
        }
        let changed = self.edit(id, |k| {
            let e = element_of(k).expect("a field is an element");
            let changed = e.value != text;
            e.value = text.to_string();
            changed
        })?;
        if changed == Some(true) {
            self.dispatch("input", id, text, 0.0, 0.0);
        }
        Ok(())
    }

    /// The user pressed Enter, or a phone keyboard's action key, in a field.
    pub fn submit(&mut self, id: NodeId) {
        let Some(id) = self.live_element(id) else {
            return;
        };
        let value = self.value(id);
        self.dispatch("change", id, &value, 0.0, 0.0);
        self.dispatch("submit", id, &value, 0.0, 0.0);
    }

    /// A stylesheet, read now and charged against the document like its
    /// nodes. Returns its number and what in it was skipped.
    pub fn add_stylesheet(&mut self, text: &str) -> Result<(u32, Vec<String>), Limit> {
        self.charge(text.len(), 0)?;
        let sheet = css::parse(text);
        let mut warnings = sheet.warnings.clone();
        if sheet.more_warnings > 0 {
            warnings.push(format!("and {} more skipped", sheet.more_warnings));
        }
        let id = self.next_sheet;
        self.next_sheet = self.next_sheet.wrapping_add(1).max(1);
        self.sheets.push((id, text.len(), Arc::new(sheet)));
        self.changed();
        Ok((id, warnings))
    }

    pub fn remove_stylesheet(&mut self, id: u32) {
        if let Some(at) = self.sheets.iter().position(|(s, _, _)| *s == id) {
            let (_, cost, _) = self.sheets.remove(at);
            self.bytes = self.bytes.saturating_sub(cost);
            self.changed();
        }
    }

    pub fn parsed_sheets(&self) -> impl Iterator<Item = &Sheet> {
        self.sheets.iter().map(|(_, _, sheet)| sheet.as_ref())
    }

    /// What the user is doing to the page, as selectors see it.
    pub fn state(&self) -> State {
        State {
            hovered: self.hovered,
            pressed: self.pressed,
            focused: self.focused,
        }
    }

    /// Every element's computed style for a page shown on `env` — computed
    /// again only when the tree, the page or the pointer has changed.
    pub fn styles(&mut self, env: &Env) -> &HashMap<NodeId, Style> {
        let state = self.state();
        let fresh = self
            .styled
            .as_ref()
            .is_some_and(|s| s.generation == self.generation && s.env == *env && s.state == state);
        if !fresh {
            let styles = style::compute(self, env, state);
            self.styled = Some(Styled {
                generation: self.generation,
                env: *env,
                state,
                styles,
            });
        }
        &self.styled.as_ref().expect("just computed").styles
    }

    /// The page laid out on `env`, its text measured by `fonts` and its
    /// images sized by `images` — computed again only when the tree, the
    /// page or the pointer has changed. What `bounds` answers from.
    pub fn layout(
        &mut self,
        env: &Env,
        fonts: &dyn layout::Fonts,
        images: &dyn Fn(u32) -> Option<(f32, f32)>,
    ) -> &layout::Layout {
        let state = self.state();
        let fresh = self
            .laid_out
            .as_ref()
            .is_some_and(|l| l.generation == self.generation && l.env == *env && l.state == state);
        if !fresh {
            self.styles(env);
            let styles = &self.styled.as_ref().expect("just styled").styles;
            let laid = layout::layout(self, styles, fonts, images, env.width, env.height);
            self.laid_out = Some(LaidOut {
                generation: self.generation,
                env: *env,
                state,
                layout: laid,
            });
        }
        &self.laid_out.as_ref().expect("just laid out").layout
    }

    /// The last layout, whatever it was for.
    pub fn last_layout(&self) -> Option<&layout::Layout> {
        self.laid_out.as_ref().map(|l| &l.layout)
    }

    /// The last layout as things to draw.
    pub fn display_list(&self) -> Vec<paint::Item> {
        match (&self.styled, &self.laid_out) {
            (Some(s), Some(l)) => paint::display_list(self, &s.styles, &l.layout),
            _ => Vec::new(),
        }
    }

    /// The element under `(x, y)` on the page, by the last layout.
    pub fn hit(&self, fonts: &dyn layout::Fonts, x: f32, y: f32) -> Option<NodeId> {
        match (&self.styled, &self.laid_out) {
            (Some(s), Some(l)) => paint::hit(self, &s.styles, &l.layout, fonts, x, y),
            _ => None,
        }
    }

    /// Scroll whatever is under `(x, y)` by `(dx, dy)` — the innermost
    /// container that can still move that way. Whether anything moved: if
    /// not, the scroll belongs to the page around.
    pub fn scroll_at(&mut self, x: f32, y: f32, dx: f32, dy: f32) -> bool {
        let (Some(s), Some(l)) = (&self.styled, &self.laid_out) else {
            return false;
        };
        for id in paint::scrollers_at(self, &s.styles, &l.layout, x, y) {
            let (mx, my) = paint::max_scroll(&l.layout, id);
            let (sx, sy) = self.scroll.get(&id).copied().unwrap_or((0.0, 0.0));
            let (sx, sy) = (sx.clamp(0.0, mx), sy.clamp(0.0, my));
            let next = ((sx + dx).clamp(0.0, mx), (sy + dy).clamp(0.0, my));
            if next != (sx, sy) {
                self.scroll.insert(id, next);
                return true;
            }
        }
        false
    }

    /// The elements under `scope` (itself excluded) matching `selector`, in
    /// document order; none for a selector that cannot be read.
    pub fn query_all(&self, scope: NodeId, selector: &str) -> Vec<NodeId> {
        let Some(selectors) = css::selectors(css::trim(&css::tokenize(selector))) else {
            return Vec::new();
        };
        let places = Places::of(self);
        let matcher = Matcher::new(self, self.state(), &places);
        let mut out = Vec::new();
        let mut stack: Vec<NodeId> = self.children(scope).into_iter().rev().collect();
        while let Some(id) = stack.pop() {
            if selectors.iter().any(|s| matcher.matches(s, id)) {
                out.push(id);
            }
            stack.extend(self.children(id).into_iter().rev());
        }
        out
    }

    /// The nodes in the tree, in document order, each with its depth.
    pub fn walk(&self) -> Vec<(NodeId, usize)> {
        let mut out = Vec::new();
        let mut stack = vec![(ROOT, 0usize)];
        while let Some((id, depth)) = stack.pop() {
            if let Some(node) = self.nodes.get(&id) {
                out.push((id, depth));
                stack.extend(node.children.iter().rev().map(|&c| (c, depth + 1)));
            }
        }
        out
    }
}

impl Document {
    /// The tree as text, a node a line, indented two spaces a level: what the
    /// headless runner prints, and what the suite reads. Strings are quoted
    /// and escaped, so a guest's text cannot pass for a line of the shell's.
    pub fn outline(&self) -> Vec<String> {
        self.walk()
            .into_iter()
            .map(|(id, depth)| self.outline_line(id, depth))
            .collect()
    }

    /// `outline`, with each node's box from the last layout when `boxes` —
    /// ` @0,0 560x120` — and each element's computed style when `styles` —
    /// ` | display: flex; gap: 16px`.
    pub fn outline_with(&mut self, env: &Env, styles: bool, boxes: bool) -> Vec<String> {
        self.styles(env);
        let computed = &self.styled.as_ref().expect("just styled").styles;
        let laid = self.laid_out.as_ref().map(|l| &l.layout);
        let initial = Style::initial();
        self.walk()
            .into_iter()
            .map(|(id, depth)| {
                let mut line = self.outline_line(id, depth);
                if boxes {
                    if let Some(r) = laid.and_then(|l| l.rect_of(id)) {
                        line.push_str(&format!(
                            " @{},{} {}x{}",
                            css::number(r.x),
                            css::number(r.y),
                            css::number(r.w),
                            css::number(r.h)
                        ));
                    }
                }
                let node = &self.nodes[&id];
                if !styles {
                    return line;
                }
                if let (Some(e), Some(st)) = (node.element(), computed.get(&id)) {
                    let parent = node
                        .parent
                        .and_then(|p| computed.get(&p))
                        .unwrap_or(&initial);
                    let said = style::describe(st, parent, &e.tag);
                    if !said.is_empty() {
                        line.push_str(" | ");
                        line.push_str(&said);
                    }
                }
                line
            })
            .collect()
    }

    fn outline_line(&self, id: NodeId, depth: usize) -> String {
        let node = &self.nodes[&id];
        let what = match &node.kind {
            Kind::Text(t) => format!("{t:?}"),
            Kind::Element(e) => describe(e, self.focused == Some(id)),
        };
        format!("{}#{id} {what}", "  ".repeat(depth))
    }
}

/// `div#main.card.wide [type="text"] value="hi" [x] {click}`: the tag, the id
/// and classes as a selector would name them, the other attributes, a
/// control's live state, what it listens for.
fn describe(e: &Element, focused: bool) -> String {
    let mut out = e.tag.clone();
    if let Some(id) = e.attr("id") {
        out.push('#');
        out.push_str(id);
    }
    for c in &e.classes {
        out.push('.');
        out.push_str(c);
    }
    let attrs: Vec<String> = e
        .attrs
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "id" | "class" | "value" | "checked"))
        .map(|(k, v)| format!("{k}={v:?}"))
        .collect();
    if !attrs.is_empty() {
        out.push_str(&format!(" [{}]", attrs.join(" ")));
    }
    if e.is_field() {
        out.push_str(&format!(" value={:?}", e.value));
    }
    if e.is_toggle() {
        out.push_str(if e.checked { " [x]" } else { " [ ]" });
    }
    if focused {
        out.push_str(" (focused)");
    }
    if !e.listeners.is_empty() {
        out.push_str(&format!(" {{{}}}", e.listeners.join(" ")));
    }
    out
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn has_class(&self, name: &str) -> bool {
        self.classes.iter().any(|c| c == name)
    }

    /// `type`, lowercase, for an `input`; "text" when missing or unknown.
    pub fn input_type(&self) -> &str {
        match self.attr("type").map(str::trim) {
            Some(t) if t.eq_ignore_ascii_case("checkbox") => "checkbox",
            Some(t) if t.eq_ignore_ascii_case("radio") => "radio",
            Some(t) if t.eq_ignore_ascii_case("password") => "password",
            Some(t) if t.eq_ignore_ascii_case("search") => "search",
            Some(t) if t.eq_ignore_ascii_case("range") => "range",
            Some(t) if t.eq_ignore_ascii_case("number") => "number",
            _ => "text",
        }
    }

    pub fn is_checkbox(&self) -> bool {
        self.tag == "input" && self.input_type() == "checkbox"
    }

    pub fn is_radio(&self) -> bool {
        self.tag == "input" && self.input_type() == "radio"
    }

    /// A checkbox or a radio: its value is its `checked`.
    pub fn is_toggle(&self) -> bool {
        self.is_checkbox() || self.is_radio()
    }

    /// Holds a value the user edits: a text input of any kind, a range, a
    /// textarea.
    pub fn is_field(&self) -> bool {
        (self.tag == "input" && !self.is_toggle()) || self.tag == "textarea"
    }

    pub fn is_focusable(&self) -> bool {
        self.is_field() || self.is_toggle() || self.tag == "button" || self.tag == "a"
    }
}

/// HTML's boolean attributes are true by being there; "false" is the one
/// spelling a person would expect to mean otherwise, so it does.
fn truthy(value: &str) -> bool {
    !value.trim().eq_ignore_ascii_case("false")
}

fn split_classes(value: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in value.split_whitespace() {
        if !out.iter().any(|o| o == c) {
            out.push(c.to_string());
        }
    }
    out
}

/// `"padding: 4px; color: red"` into its properties, in order, later ones
/// replacing earlier. Values are kept as written; they are read when the page
/// is styled.
fn parse_inline_style(css: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for decl in css.split(';') {
        let Some((k, v)) = decl.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        if k.is_empty() || v.is_empty() {
            continue;
        }
        out.retain(|(o, _)| *o != k);
        out.push((k, v.to_string()));
    }
    out
}
