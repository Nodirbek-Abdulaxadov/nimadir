//! The display list (ADR-4): the laid-out page as things to draw, in the
//! order to draw them — and the same walk run backwards to find what a
//! pointer is over.
//!
//! Everything a renderer should not have to decide is decided here: what is
//! visible, what clips what, how far a scroll container has scrolled, how
//! opacity multiplies down the tree, which control sits where. A renderer
//! (`gui.rs`) turns each `Item` into its own shapes and widgets.

use super::layout::{Content, Font, Fonts, Layout, Paragraph, Rect};
use super::style::{self as st, BorderStyle, Gradient, Shadow, Style};
use super::values::{self, Color};
use super::{Document, Kind, NodeId, NodeMap};

/// One thing to draw.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// Clip what follows to `rect` — already narrowed by the clip it is in —
    /// until the matching `Unclip`.
    Clip(Rect),
    Unclip,
    Shadow {
        rect: Rect,
        radius: [f32; 4],
        shadow: Shadow,
    },
    Fill {
        rect: Rect,
        radius: [f32; 4],
        color: Color,
    },
    Gradient {
        rect: Rect,
        radius: [f32; 4],
        gradient: Gradient,
    },
    /// top, right, bottom, left.
    Border {
        rect: Rect,
        radius: [f32; 4],
        widths: [f32; 4],
        colors: [Color; 4],
        style: BorderStyle,
    },
    Text {
        rect: Rect,
        paragraph: Paragraph,
    },
    Image {
        rect: Rect,
        handle: u32,
        opacity: f32,
        fit: st::ObjectFit,
    },
    /// A text field: the renderer's own editing widget goes here.
    Field {
        rect: Rect,
        node: NodeId,
        value: String,
        placeholder: String,
        font: Font,
        multiline: bool,
        password: bool,
        /// Not to be edited: `disabled`, or `readonly` — which still lets
        /// the text be selected.
        disabled: bool,
        readonly: bool,
        /// `maxlength`.
        max_chars: Option<usize>,
    },
    /// A `select`, closed: the chosen option and an arrow; its options are
    /// the renderer's to offer when it is tapped.
    Select {
        rect: Rect,
        node: NodeId,
        options: Vec<String>,
        selected: usize,
        font: Font,
        disabled: bool,
    },
    /// A checkbox's tick or a radio's dot, inside its box.
    Check {
        rect: Rect,
        checked: bool,
        radio: bool,
        mark: Color,
    },
    Range {
        rect: Rect,
        node: NodeId,
        fraction: f32,
        min: f32,
        max: f32,
        /// 0 for none.
        step: f32,
        accent: Color,
        track: Color,
        disabled: bool,
    },
    Progress {
        rect: Rect,
        radius: [f32; 4],
        fraction: f32,
        color: Color,
    },
    /// A list item's bullet or number, left of `rect`, its first line.
    Marker {
        rect: Rect,
        kind: st::ListStyle,
        ordinal: u32,
        font: Font,
        line: f32,
    },
}

/// Where the walk is: how far scrolling has moved things, what clips them,
/// how faded they are, and whether their container is visible.
#[derive(Clone, Copy)]
struct Walk {
    dx: f32,
    dy: f32,
    clip: Option<Rect>,
    opacity: f32,
    visible: bool,
}

impl Walk {
    const PAGE: Walk = Walk {
        dx: 0.0,
        dy: 0.0,
        clip: None,
        opacity: 1.0,
        visible: true,
    };

    fn place(&self, r: Rect) -> Rect {
        Rect {
            x: r.x + self.dx,
            y: r.y + self.dy,
            ..r
        }
    }
}

/// Whether an element's content is clipped to it, and whether it scrolls.
fn clips(s: &Style) -> (bool, bool) {
    let clip = s.overflow.iter().any(|o| *o != st::Overflow::Visible);
    let scroll = s
        .overflow
        .iter()
        .any(|o| matches!(o, st::Overflow::Scroll | st::Overflow::Auto));
    (clip, scroll)
}

/// Corner radii in pixels, a percentage taken of the shorter side, none
/// past half of it.
pub fn radii(s: &Style, r: &Rect) -> [f32; 4] {
    let side = r.w.min(r.h).max(0.0);
    // `max` then `min`, not `clamp`: neither panics on a NaN.
    s.radius
        .map(|l| l.resolve(side).unwrap_or(0.0).max(0.0).min(side / 2.0))
}

/// How far a scroll container can scroll, from its layout.
pub fn max_scroll(layout: &Layout, id: NodeId) -> (f32, f32) {
    let Some(&i) = layout.by_node.get(&id) else {
        return (0.0, 0.0);
    };
    let b = &layout.boxes[i];
    let p = b.padding_rect();
    ((b.extent.0 - p.w).max(0.0), (b.extent.1 - p.h).max(0.0))
}

impl Item {
    /// One line for the headless runner: what, where, and the detail that
    /// matters.
    pub fn describe(&self) -> String {
        use super::css::number as n;
        let at = |r: &Rect| format!("{},{} {}x{}", n(r.x), n(r.y), n(r.w), n(r.h));
        match self {
            Item::Clip(r) => format!("clip {}", at(r)),
            Item::Unclip => "unclip".into(),
            Item::Shadow { rect, shadow, .. } => format!(
                "shadow {} {}px {}",
                at(rect),
                n(shadow.blur),
                shadow.color.map_or(String::new(), |c| c.css())
            ),
            Item::Fill {
                rect,
                radius,
                color,
            } => format!("fill {} r{} {}", at(rect), n(radius[0]), color.css()),
            Item::Gradient { rect, gradient, .. } => {
                let stops: Vec<String> = gradient.stops.iter().map(|(c, _)| c.css()).collect();
                format!(
                    "gradient {} {}deg {}",
                    at(rect),
                    n(gradient.angle),
                    stops.join(" ")
                )
            }
            Item::Border {
                rect,
                widths,
                colors,
                ..
            } => format!("border {} {}px {}", at(rect), n(widths[0]), colors[0].css()),
            Item::Text { rect, paragraph } => format!("text {} {:?}", at(rect), paragraph.text()),
            Item::Image { rect, handle, .. } => format!("image {} #{handle}", at(rect)),
            Item::Field {
                rect, node, value, ..
            } => format!("field #{node} {} {value:?}", at(rect)),
            Item::Select {
                rect,
                node,
                options,
                selected,
                ..
            } => format!(
                "select #{node} {} {:?}",
                at(rect),
                options.get(*selected).map_or("", String::as_str)
            ),
            Item::Check {
                rect,
                checked,
                radio,
                ..
            } => format!(
                "{} {} {}",
                if *radio { "radio" } else { "check" },
                at(rect),
                if *checked { "on" } else { "off" }
            ),
            Item::Range { rect, fraction, .. } => {
                format!("range {} {}%", at(rect), n(fraction * 100.0))
            }
            Item::Progress { rect, fraction, .. } => {
                format!("progress {} {}%", at(rect), n(fraction * 100.0))
            }
            Item::Marker {
                rect,
                kind,
                ordinal,
                ..
            } => format!("marker {} {} {ordinal}", at(rect), kind.css()),
        }
    }
}

/// The page, as things to draw.
pub fn display_list(doc: &Document, styles: &NodeMap<Style>, layout: &Layout) -> Vec<Item> {
    let mut out = Vec::new();
    if !layout.boxes.is_empty() {
        paint(doc, styles, layout, 0, Walk::PAGE, &mut out);
    }
    out
}

fn paint(
    doc: &Document,
    styles: &NodeMap<Style>,
    layout: &Layout,
    i: usize,
    w: Walk,
    out: &mut Vec<Item>,
) {
    let b = &layout.boxes[i];
    let rect = w.place(b.rect);
    match &b.content {
        Content::Line => {
            for &c in &b.children {
                paint(doc, styles, layout, c, w, out);
            }
        }
        Content::Text(p) => {
            if w.visible {
                out.push(Item::Text {
                    rect: rect.inset(b.padding),
                    paragraph: faded(&recoloured(doc, styles, p), w.opacity),
                });
            }
        }
        Content::Element(id) => {
            let Some(s) = styles.get(id) else {
                return;
            };
            let opacity = w.opacity * s.opacity;
            if opacity <= 0.0 {
                return;
            }
            let visible = s.visibility == st::Visibility::Visible;
            let e = doc.nodes.get(id).and_then(|n| n.element());
            if visible {
                decorate(s, &rect, b.border, opacity, out);
                if let Some(e) = e {
                    control(doc, *id, e, s, &rect, b, opacity, out);
                }
                if s.display == st::Display::ListItem && s.list_style != st::ListStyle::None {
                    out.push(Item::Marker {
                        rect: rect.inset(b.border).inset(b.padding),
                        kind: s.list_style,
                        ordinal: b.ordinal,
                        font: fade_font(s, opacity),
                        line: s.line_height.px(s.font_size),
                    });
                }
            }
            if b.children.is_empty() {
                return;
            }
            let (clip, scroll) = clips(s);
            let mut inner = Walk {
                opacity,
                visible,
                ..w
            };
            if clip {
                let area = b.padding_rect();
                let area = w.place(area);
                let narrowed = w.clip.map_or(area, |c| c.intersect(&area));
                inner.clip = Some(narrowed);
                out.push(Item::Clip(narrowed));
            }
            if scroll {
                let (sx, sy) = doc.scroll.get(id).copied().unwrap_or((0.0, 0.0));
                let (mx, my) = max_scroll(layout, *id);
                inner.dx -= sx.clamp(0.0, mx);
                inner.dy -= sy.clamp(0.0, my);
            }
            for &c in &b.children {
                paint(doc, styles, layout, c, inner, out);
            }
            if clip {
                out.push(Item::Unclip);
            }
        }
    }
}

/// An element's shadow, background and border.
fn decorate(s: &Style, rect: &Rect, border: [f32; 4], opacity: f32, out: &mut Vec<Item>) {
    let radius = radii(s, rect);
    if let Some(sh) = s.shadow.filter(|sh| !sh.inset) {
        let color = sh.color.unwrap_or(s.color).faded(opacity);
        if !color.is_transparent() {
            out.push(Item::Shadow {
                rect: *rect,
                radius,
                shadow: Shadow {
                    color: Some(color),
                    ..sh
                },
            });
        }
    }
    if !s.background.is_transparent() {
        out.push(Item::Fill {
            rect: *rect,
            radius,
            color: s.background.faded(opacity),
        });
    }
    if let Some(g) = &s.gradient {
        out.push(Item::Gradient {
            rect: *rect,
            radius,
            gradient: Gradient {
                angle: g.angle,
                stops: g
                    .stops
                    .iter()
                    .map(|(c, at)| (c.faded(opacity), *at))
                    .collect(),
            },
        });
    }
    if border.iter().any(|w| *w > 0.0) {
        let colors = s.border_color.map(|c| c.unwrap_or(s.color).faded(opacity));
        out.push(Item::Border {
            rect: *rect,
            radius,
            widths: border,
            colors,
            style: s.border_style[0],
        });
    }
}

/// What a control draws inside its box, beyond its CSS.
#[allow(clippy::too_many_arguments)]
fn control(
    doc: &Document,
    id: NodeId,
    e: &super::Element,
    s: &Style,
    rect: &Rect,
    b: &super::layout::Placed,
    opacity: f32,
    out: &mut Vec<Item>,
) {
    let content = rect.inset(b.border).inset(b.padding);
    match e.tag.as_str() {
        "input" if e.is_toggle() => out.push(Item::Check {
            rect: rect.inset(b.border),
            checked: e.checked,
            radio: e.is_radio(),
            mark: contrast(if s.background.is_transparent() {
                s.color
            } else {
                s.background
            })
            .faded(opacity),
        }),
        "input" if e.input_type() == "range" => {
            let num = |k: &str, d: f32| {
                e.attr(k)
                    .and_then(|v| v.trim().parse::<f32>().ok())
                    .unwrap_or(d)
            };
            let (min, max) = (num("min", 0.0), num("max", 100.0));
            let value = e.value.trim().parse::<f32>().unwrap_or((min + max) / 2.0);
            let fraction = if max > min {
                ((value - min) / (max - min)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            out.push(Item::Range {
                rect: content,
                node: id,
                fraction,
                min,
                max,
                step: num("step", 1.0).max(0.0),
                accent: accent(s).faded(opacity),
                track: s.color.faded(0.25 * opacity),
                disabled: doc.disabled(id),
            });
        }
        "input" | "textarea" => out.push(Item::Field {
            rect: content,
            node: id,
            value: e.value.clone(),
            placeholder: e.attr("placeholder").unwrap_or("").to_string(),
            font: fade_font(s, opacity),
            multiline: e.tag == "textarea",
            password: e.input_type() == "password",
            disabled: doc.disabled(id),
            readonly: e.attr("readonly").is_some(),
            max_chars: e.attr("maxlength").and_then(|m| m.trim().parse().ok()),
        }),
        "select" => out.push(Item::Select {
            rect: content,
            node: id,
            options: doc
                .options(id)
                .into_iter()
                .map(|(_, label)| label)
                .collect(),
            selected: doc.selected(id),
            font: fade_font(s, opacity),
            disabled: doc.disabled(id),
        }),
        "progress" => {
            let num = |k: &str| e.attr(k).and_then(|v| v.trim().parse::<f32>().ok());
            let max = num("max").filter(|m| *m > 0.0).unwrap_or(1.0);
            let fraction = num("value").map_or(0.0, |v| (v / max).clamp(0.0, 1.0));
            out.push(Item::Progress {
                rect: rect.inset(b.border),
                radius: radii(s, rect),
                fraction,
                color: s.color.faded(opacity),
            });
        }
        "img" => {
            if let Some(handle) = e.attr("src").and_then(|v| v.trim().parse::<u32>().ok()) {
                out.push(Item::Image {
                    rect: content,
                    handle,
                    opacity,
                    fit: s.object_fit,
                });
            }
        }
        _ => {}
    }
}

/// The host's accent, as the element's variables have it.
fn accent(s: &Style) -> Color {
    s.vars
        .get("--nimadir-accent")
        .and_then(|v| values::color(v, s.color))
        .unwrap_or(s.color)
}

/// Black or white, whichever reads on `c`.
fn contrast(c: Color) -> Color {
    let [r, g, b, _] = c.0;
    let luma = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
    if luma > 150.0 {
        Color::rgb(0x202124)
    } else {
        Color::rgb(0xffffff)
    }
}

fn fade_font(s: &Style, opacity: f32) -> Font {
    Font {
        size: s.font_size,
        bold: s.font_weight >= 600,
        italic: s.italic,
        mono: s.monospace,
        color: s.color.faded(opacity),
        decoration: s.text_decoration,
        letter_spacing: s.letter_spacing,
        line: s.line_height.px(s.font_size),
        background: Color::TRANSPARENT,
    }
}

/// A paragraph in its runs' colours and decorations as styled now: the
/// layout may be from before a `:hover` that changed only them.
fn recoloured(doc: &Document, styles: &NodeMap<Style>, p: &Paragraph) -> Paragraph {
    let mut p = p.clone();
    for r in &mut p.runs {
        let element = match doc.nodes.get(&r.node).map(|n| &n.kind) {
            Some(Kind::Text(_)) => doc.parent(r.node),
            _ => Some(r.node),
        };
        if let Some(s) = element.and_then(|e| styles.get(&e)) {
            r.font.color = s.color;
            r.font.decoration = s.text_decoration;
        }
        r.font.background = inline_background(doc, styles, element);
    }
    p
}

/// The background behind a run: the nearest inline element around it that
/// has one. A block's background is its box's, painted as such.
fn inline_background(doc: &Document, styles: &NodeMap<Style>, mut at: Option<NodeId>) -> Color {
    while let Some(id) = at {
        let Some(s) = styles.get(&id) else { break };
        if !s.is_inline() {
            break;
        }
        if !s.background.is_transparent() {
            return s.background;
        }
        at = doc.parent(id);
    }
    Color::TRANSPARENT
}

fn faded(p: &Paragraph, opacity: f32) -> Paragraph {
    if opacity >= 1.0 {
        return p.clone();
    }
    let mut p = p.clone();
    for r in &mut p.runs {
        r.font.color = r.font.color.faded(opacity);
        r.font.background = r.font.background.faded(opacity);
    }
    p
}

/// The element under `(x, y)` on the page — the topmost, as drawn — or
/// `None` over nothing that takes a pointer.
pub fn hit(
    doc: &Document,
    styles: &NodeMap<Style>,
    layout: &Layout,
    fonts: &dyn Fonts,
    x: f32,
    y: f32,
) -> Option<NodeId> {
    if layout.boxes.is_empty() {
        return None;
    }
    hit_box(doc, styles, layout, fonts, 0, Walk::PAGE, x, y)
}

#[allow(clippy::too_many_arguments)]
fn hit_box(
    doc: &Document,
    styles: &NodeMap<Style>,
    layout: &Layout,
    fonts: &dyn Fonts,
    i: usize,
    w: Walk,
    x: f32,
    y: f32,
) -> Option<NodeId> {
    if w.clip.is_some_and(|c| !c.contains(x, y)) {
        return None;
    }
    let b = &layout.boxes[i];
    let rect = w.place(b.rect);
    match &b.content {
        Content::Line => b
            .children
            .iter()
            .rev()
            .find_map(|&c| hit_box(doc, styles, layout, fonts, c, w, x, y)),
        Content::Text(p) => {
            if !w.visible || !rect.contains(x, y) {
                return None;
            }
            let area = rect.inset(b.padding);
            let run = fonts.run_at(p, area.w, x - area.x, y - area.y)?;
            let node = p.runs[run].node;
            // A run is a text node's; what answers is the element around it.
            match doc.nodes.get(&node).map(|n| &n.kind) {
                Some(Kind::Text(_)) => doc.parent(node),
                _ => Some(node),
            }
        }
        Content::Element(id) => {
            let s = styles.get(id)?;
            if s.opacity * w.opacity <= 0.0 {
                return None;
            }
            let (clip, scroll) = clips(s);
            let mut inner = Walk {
                visible: s.visibility == st::Visibility::Visible,
                ..w
            };
            if clip {
                let area = w.place(b.padding_rect());
                inner.clip = Some(w.clip.map_or(area, |c| c.intersect(&area)));
            }
            if scroll {
                let (sx, sy) = doc.scroll.get(id).copied().unwrap_or((0.0, 0.0));
                let (mx, my) = max_scroll(layout, *id);
                inner.dx -= sx.clamp(0.0, mx);
                inner.dy -= sy.clamp(0.0, my);
            }
            if let Some(h) = b
                .children
                .iter()
                .rev()
                .find_map(|&c| hit_box(doc, styles, layout, fonts, c, inner, x, y))
            {
                return Some(h);
            }
            let takes = s.pointer_events == st::PointerEvents::Auto
                && s.visibility == st::Visibility::Visible;
            (takes && rect.contains(x, y)).then_some(*id)
        }
    }
}

/// The scroll containers under `(x, y)`, innermost first.
pub fn scrollers_at(
    doc: &Document,
    styles: &NodeMap<Style>,
    layout: &Layout,
    x: f32,
    y: f32,
) -> Vec<NodeId> {
    let mut out = Vec::new();
    if !layout.boxes.is_empty() {
        scrollers(doc, styles, layout, 0, Walk::PAGE, x, y, &mut out);
    }
    out.reverse();
    out
}

#[allow(clippy::too_many_arguments)]
fn scrollers(
    doc: &Document,
    styles: &NodeMap<Style>,
    layout: &Layout,
    i: usize,
    w: Walk,
    x: f32,
    y: f32,
    out: &mut Vec<NodeId>,
) {
    let b = &layout.boxes[i];
    if !w.place(b.rect).contains(x, y) {
        return;
    }
    let mut inner = w;
    if let Content::Element(id) = &b.content {
        if let Some(s) = styles.get(id) {
            if clips(s).1 {
                out.push(*id);
                let (sx, sy) = doc.scroll.get(id).copied().unwrap_or((0.0, 0.0));
                let (mx, my) = max_scroll(layout, *id);
                inner.dx -= sx.clamp(0.0, mx);
                inner.dy -= sy.clamp(0.0, my);
            }
        }
    }
    for &c in &b.children {
        scrollers(doc, styles, layout, c, inner, x, y, out);
    }
}

/// Every scroll container, with the part of its padding box that shows.
pub fn scroll_areas(
    doc: &Document,
    styles: &NodeMap<Style>,
    layout: &Layout,
) -> Vec<(NodeId, Rect)> {
    let mut out = Vec::new();
    if !layout.boxes.is_empty() {
        areas(doc, styles, layout, 0, Walk::PAGE, &mut out);
    }
    out
}

fn areas(
    doc: &Document,
    styles: &NodeMap<Style>,
    layout: &Layout,
    i: usize,
    w: Walk,
    out: &mut Vec<(NodeId, Rect)>,
) {
    let b = &layout.boxes[i];
    let mut inner = w;
    if let Content::Element(id) = &b.content {
        if let Some(s) = styles.get(id) {
            let (clip, scroll) = clips(s);
            if clip {
                let area = w.place(b.padding_rect());
                let shown = w.clip.map_or(area, |c| c.intersect(&area));
                inner.clip = Some(shown);
                if scroll {
                    out.push((*id, shown));
                    let (sx, sy) = doc.scroll.get(id).copied().unwrap_or((0.0, 0.0));
                    let (mx, my) = max_scroll(layout, *id);
                    inner.dx -= sx.clamp(0.0, mx);
                    inner.dy -= sy.clamp(0.0, my);
                }
            }
        }
    }
    for &c in &b.children {
        areas(doc, styles, layout, c, inner, out);
    }
}
