//! What a screen reader is told about a page on the tree (S9-4).
//!
//! The web's accessibility tree, cut down to what a reader needs: for each
//! element that means something to someone who cannot see it, a role and a
//! name. The role comes from `role` when it names one this knows, else from
//! the tag. The name comes from `aria-label`, then `aria-labelledby`, then
//! what the element offers itself — an image's `alt`, a control's `<label>`,
//! a link's, a button's or a heading's text — then `title`, then a field's
//! `placeholder`. Text not taken up as a block's name is read as it stands,
//! a paragraph at a time. `aria-hidden="true"` takes an element out with
//! everything in it, as `display: none` does; an image with `alt=""` is
//! decoration and is left out.

use std::collections::HashSet;

use super::layout::{Layout, Rect};
use super::{Document, Kind, NodeId, ROOT};

/// What an element is, to a screen reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// With its level, 1 to 6.
    Heading(u8),
    Link,
    Button,
    TextBox,
    SearchBox,
    /// A text box whose value is never read out.
    Password,
    CheckBox,
    Radio,
    Slider,
    ComboBox,
    Image,
    List,
    ListItem,
    ProgressBar,
    Navigation,
    Main,
    /// A paragraph of text, read as it stands.
    Text,
}

impl Role {
    /// As the headless runner prints it.
    pub fn label(self) -> String {
        match self {
            Role::Heading(n) => format!("heading {n}"),
            Role::Link => "link".into(),
            Role::Button => "button".into(),
            Role::TextBox => "textbox".into(),
            Role::SearchBox => "searchbox".into(),
            Role::Password => "password".into(),
            Role::CheckBox => "checkbox".into(),
            Role::Radio => "radio".into(),
            Role::Slider => "slider".into(),
            Role::ComboBox => "combobox".into(),
            Role::Image => "image".into(),
            Role::List => "list".into(),
            Role::ListItem => "listitem".into(),
            Role::ProgressBar => "progressbar".into(),
            Role::Navigation => "navigation".into(),
            Role::Main => "main".into(),
            Role::Text => "text".into(),
        }
    }

    /// Whether a reader can act on it: press it, or put the keyboard in it.
    pub fn is_control(self) -> bool {
        matches!(
            self,
            Role::Link
                | Role::Button
                | Role::TextBox
                | Role::SearchBox
                | Role::Password
                | Role::CheckBox
                | Role::Radio
                | Role::Slider
                | Role::ComboBox
        )
    }

    /// Named by the text inside it when nothing names it otherwise.
    fn named_by_content(self) -> bool {
        matches!(
            self,
            Role::Heading(_) | Role::Link | Role::Button | Role::ListItem
        )
    }
}

/// One element, or one paragraph of text, as a screen reader has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Accessible {
    pub node: NodeId,
    pub role: Role,
    pub name: String,
    /// A field's text, a select's choice, a slider's value — never a
    /// password's.
    pub value: Option<String>,
    /// A checkbox's or radio's state.
    pub checked: Option<bool>,
    pub disabled: bool,
    /// A slider's or progress bar's minimum, maximum and value.
    pub range: Option<(f64, f64, f64)>,
    /// Where it is on the page.
    pub rect: Rect,
}

impl Accessible {
    /// A line of the headless runner's `--a11y`: `#5 searchbox "Search" = ""`.
    pub fn describe(&self) -> String {
        let mut out = format!("#{} {} {:?}", self.node, self.role.label(), self.name);
        if let Some(v) = &self.value {
            out += &format!(" = {v:?}");
        }
        if let Some(c) = self.checked {
            out += if c { " (checked)" } else { " (unchecked)" };
        }
        if let Some((lo, hi, now)) = self.range {
            out += &format!(" [{lo}..{hi}: {now}]");
        }
        if self.disabled {
            out += " (disabled)";
        }
        out
    }
}

/// The page as a screen reader reads it, in document order.
pub fn tree(doc: &Document, layout: &Layout) -> Vec<Accessible> {
    let mut out = Vec::new();
    // Paragraphs already read, by layout box: several text nodes and inline
    // elements share one.
    let mut read: HashSet<usize> = HashSet::new();
    let mut stack = vec![ROOT];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.nodes.get(&id) else {
            continue;
        };
        match &node.kind {
            Kind::Text(_) => {
                let Some(&i) = layout.by_text.get(&id) else {
                    continue;
                };
                if !read.insert(i) {
                    continue;
                }
                let p = &layout.boxes[i];
                let text = match &p.content {
                    super::layout::Content::Text(p) => words(&p.text()),
                    _ => continue,
                };
                if !text.is_empty() {
                    out.push(Accessible {
                        node: id,
                        role: Role::Text,
                        name: text,
                        value: None,
                        checked: None,
                        disabled: false,
                        range: None,
                        rect: p.rect,
                    });
                }
            }
            Kind::Element(e) => {
                if e.attr("aria-hidden").is_some_and(|v| v.trim() == "true") {
                    continue;
                }
                // Not drawn — `display: none`, `hidden`, or inside one.
                let Some(rect) = rect_of(doc, layout, id) else {
                    continue;
                };
                if e.tag == "img" && e.attr("alt").is_some_and(|a| a.trim().is_empty()) {
                    continue;
                }
                let role = role_of(doc, id);
                if let Some(role) = role {
                    let name = name_of(doc, layout, id, role);
                    out.push(Accessible {
                        node: id,
                        role,
                        name,
                        value: value_of(doc, id, role),
                        checked: checked_of(doc, id, role),
                        disabled: doc.disabled(id)
                            || e.attr("aria-disabled").is_some_and(|v| v.trim() == "true"),
                        range: range_of(doc, id, role),
                        rect,
                    });
                    // A block that took its text as its name is not read
                    // twice; text around an inline link still is.
                    if named_by_content(doc, id, role) && layout.by_node.contains_key(&id) {
                        for t in texts(doc, id) {
                            if let Some(&i) = layout.by_text.get(&t) {
                                read.insert(i);
                            }
                        }
                    }
                }
                // A control's insides are its own: an option is read as the
                // select's value, not as text.
                if !matches!(
                    role,
                    Some(Role::ComboBox | Role::TextBox | Role::Password | Role::SearchBox)
                ) {
                    stack.extend(node.children.iter().rev());
                }
            }
        }
    }
    out
}

/// An element's box on the page, or, for one laid out inline, the paragraph
/// its first text is in.
fn rect_of(doc: &Document, layout: &Layout, id: NodeId) -> Option<Rect> {
    layout.rect_of(id).or_else(|| {
        texts(doc, id)
            .into_iter()
            .find_map(|t| layout.by_text.get(&t).map(|&i| layout.boxes[i].rect))
    })
}

/// The text nodes under `id`, in document order, leaving out what
/// `aria-hidden` hides.
fn texts(doc: &Document, id: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![id];
    while let Some(n) = stack.pop() {
        match doc.nodes.get(&n).map(|node| (node, &node.kind)) {
            Some((_, Kind::Text(_))) => out.push(n),
            Some((_, Kind::Element(e)))
                if e.attr("aria-hidden").is_some_and(|v| v.trim() == "true") => {}
            Some((node, _)) => stack.extend(node.children.iter().rev()),
            None => {}
        }
    }
    out
}

fn role_of(doc: &Document, id: NodeId) -> Option<Role> {
    let e = doc.nodes.get(&id)?.element()?;
    if let Some(r) = e.attr("role") {
        let r = r.trim().to_ascii_lowercase();
        let explicit = match r.as_str() {
            "presentation" | "none" => return None,
            "heading" => Some(Role::Heading(
                e.attr("aria-level")
                    .and_then(|l| l.trim().parse().ok())
                    .unwrap_or(2)
                    .clamp(1, 6),
            )),
            "link" => Some(Role::Link),
            "button" => Some(Role::Button),
            "textbox" => Some(Role::TextBox),
            "searchbox" => Some(Role::SearchBox),
            "checkbox" | "switch" => Some(Role::CheckBox),
            "radio" => Some(Role::Radio),
            "slider" => Some(Role::Slider),
            "combobox" | "listbox" => Some(Role::ComboBox),
            "img" | "image" => Some(Role::Image),
            "list" => Some(Role::List),
            "listitem" => Some(Role::ListItem),
            "progressbar" => Some(Role::ProgressBar),
            "navigation" => Some(Role::Navigation),
            "main" => Some(Role::Main),
            _ => None,
        };
        if explicit.is_some() {
            return explicit;
        }
    }
    Some(match e.tag.as_str() {
        "a" if e.attr("href").is_some() => Role::Link,
        "button" => Role::Button,
        "input" => match e.input_type() {
            "checkbox" => Role::CheckBox,
            "radio" => Role::Radio,
            "range" => Role::Slider,
            "search" => Role::SearchBox,
            "password" => Role::Password,
            _ => Role::TextBox,
        },
        "textarea" => Role::TextBox,
        "select" => Role::ComboBox,
        "img" => Role::Image,
        "ul" | "ol" => Role::List,
        "li" => Role::ListItem,
        "progress" => Role::ProgressBar,
        "nav" => Role::Navigation,
        "main" => Role::Main,
        t if t.len() == 2 && t.starts_with('h') => {
            let level = t[1..].parse::<u8>().ok().filter(|l| (1..=6).contains(l))?;
            Role::Heading(level)
        }
        _ => return None,
    })
}

fn name_of(doc: &Document, layout: &Layout, id: NodeId, role: Role) -> String {
    let Some(e) = doc.nodes.get(&id).and_then(|n| n.element()) else {
        return String::new();
    };
    let given = |name: &str| e.attr(name).map(words).filter(|s| !s.is_empty());
    if let Some(n) = given("aria-label") {
        return n;
    }
    if let Some(ids) = e.attr("aria-labelledby") {
        let n = words(
            &ids.split_whitespace()
                .filter_map(|i| doc.element_by_id(i))
                .map(|n| doc.text_content(n))
                .collect::<Vec<_>>()
                .join(" "),
        );
        if !n.is_empty() {
            return n;
        }
    }
    let own = match role {
        Role::Image => given("alt"),
        r if named_by_content(doc, id, r) => {
            Some(spoken(doc, layout, id)).filter(|s| !s.is_empty())
        }
        r if r.is_control() => label_of(doc, layout, id),
        _ => None,
    };
    own.or_else(|| given("title"))
        .or_else(|| {
            matches!(role, Role::TextBox | Role::SearchBox | Role::Password)
                .then(|| given("placeholder"))
                .flatten()
        })
        .unwrap_or_default()
}

/// Whether `id` is named by the text inside it: a link, a button, a heading
/// or a list item, and a checkbox or radio made of an element that is not
/// an input (`<div role="checkbox">Wi-Fi</div>`).
fn named_by_content(doc: &Document, id: NodeId, role: Role) -> bool {
    role.named_by_content()
        || (matches!(role, Role::CheckBox | Role::Radio)
            && doc
                .nodes
                .get(&id)
                .and_then(|n| n.element())
                .is_some_and(|e| !e.is_toggle()))
}

/// The text of the `<label>` that names a control: the one whose `for` is
/// its `id`, else the one it sits in.
fn label_of(doc: &Document, layout: &Layout, id: NodeId) -> Option<String> {
    let e = doc.nodes.get(&id)?.element()?;
    let by_for = e.attr("id").filter(|i| !i.is_empty()).and_then(|own| {
        doc.walk().into_iter().find_map(|(n, _)| {
            let l = doc.nodes.get(&n)?.element()?;
            (l.tag == "label" && l.attr("for") == Some(own)).then_some(n)
        })
    });
    let label = by_for.or_else(|| {
        let mut at = doc.parent(id);
        while let Some(p) = at {
            if doc.nodes.get(&p)?.element()?.tag == "label" {
                return Some(p);
            }
            at = doc.parent(p);
        }
        None
    })?;
    Some(spoken(doc, layout, label)).filter(|s| !s.is_empty())
}

fn value_of(doc: &Document, id: NodeId, role: Role) -> Option<String> {
    let e = doc.nodes.get(&id)?.element()?;
    match role {
        Role::TextBox | Role::SearchBox if e.is_field() => Some(e.value.clone()),
        Role::ComboBox if e.is_select() => doc
            .options(id)
            .get(doc.selected(id))
            .map(|(_, l)| l.clone()),
        Role::Slider if e.is_field() => Some(e.value.clone()),
        _ => None,
    }
}

fn checked_of(doc: &Document, id: NodeId, role: Role) -> Option<bool> {
    let e = doc.nodes.get(&id)?.element()?;
    match role {
        Role::CheckBox | Role::Radio if e.is_toggle() => Some(e.checked),
        Role::CheckBox | Role::Radio => {
            Some(e.attr("aria-checked").is_some_and(|v| v.trim() == "true"))
        }
        _ => None,
    }
}

fn range_of(doc: &Document, id: NodeId, role: Role) -> Option<(f64, f64, f64)> {
    let e = doc.nodes.get(&id)?.element()?;
    let num = |name: &str| e.attr(name).and_then(|v| v.trim().parse::<f64>().ok());
    match role {
        Role::Slider => {
            let (lo, hi) = (num("min").unwrap_or(0.0), num("max").unwrap_or(100.0));
            let now = e
                .value
                .trim()
                .parse()
                .ok()
                .or_else(|| num("aria-valuenow"))
                .unwrap_or(lo);
            Some((lo, hi, now))
        }
        Role::ProgressBar => {
            let hi = num("max").unwrap_or(1.0);
            Some((
                0.0,
                hi,
                num("value").or_else(|| num("aria-valuenow")).unwrap_or(0.0),
            ))
        }
        _ => None,
    }
}

/// The text inside `id` as it is read: a space between what is laid out
/// apart — a badge, a name and a line under it are three words, not one —
/// and none inside a paragraph, where `<b>un</b>done` is one word.
fn spoken(doc: &Document, layout: &Layout, id: NodeId) -> String {
    let mut out = String::new();
    let mut last = None;
    for t in texts(doc, id) {
        let Some(Kind::Text(s)) = doc.nodes.get(&t).map(|n| &n.kind) else {
            continue;
        };
        let at = layout.by_text.get(&t).copied();
        if at.is_none() {
            continue;
        }
        if last.is_some() && at != last {
            out.push(' ');
        }
        out.push_str(s);
        last = at;
    }
    words(&out)
}

/// Whitespace collapsed, as text is read out.
fn words(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
