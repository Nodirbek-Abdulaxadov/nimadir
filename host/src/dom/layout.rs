//! Layout (ADR-4): the styled tree turned into boxes on the page, by taffy —
//! block, flex, grid, absolute positioning and floats.
//!
//! Text is measured through `Fonts`, which the renderer implements with its
//! own fonts (`gui.rs`), and `Approx` stands in where there is none (the
//! headless runner), so line breaks are the renderer's while layout stays in
//! the core.
//!
//! Inline content is simplified: a run of text and inline elements becomes
//! one *paragraph* — styled runs laid out together, wrapping as a whole —
//! and a run that also holds boxes (a button in a sentence) becomes a
//! wrapping row of paragraphs and those boxes. Inline elements contribute
//! their text's look, not boxes of their own.
//!
//! taffy is driven through its low-level traits rather than `TaffyTree`, so
//! that `calc()` resolves (`Calc`) and text is measured here. The taffy tree
//! lives only for one layout: its styles hold raw pointers, and what is kept
//! (`Layout`) is plain data.

use std::cell::Cell;

use taffy as tf;
use tf::{CacheTree, LayoutPartialTree, TraversePartialTree};

use super::style::{self as st, Display, Style, WhiteSpace};
use super::values::{Color, Length};
use super::{Document, Kind, NodeId, NodeMap, ROOT};

/// The look of one run of text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Font {
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    pub color: Color,
    /// `style::UNDERLINE` | `LINE_THROUGH` | `OVERLINE`.
    pub decoration: u8,
    pub letter_spacing: f32,
    /// The line height its element asks for, in pixels.
    pub line: f32,
    /// Behind the text: the background of the inline element it sits in —
    /// a `mark`, a highlighted `span` — filled in when painted.
    pub background: Color,
}

impl Font {
    fn of(s: &Style) -> Font {
        Font {
            size: s.font_size,
            bold: s.font_weight >= 600,
            italic: s.italic,
            mono: s.monospace,
            color: s.color,
            decoration: s.text_decoration,
            letter_spacing: s.letter_spacing,
            line: s.line_height.px(s.font_size),
            background: Color::TRANSPARENT,
        }
    }
}

/// A piece of text in one font, and the node it came from — what a tap on
/// it lands on.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub font: Font,
    pub node: NodeId,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// Inline content laid out as one block of text.
#[derive(Clone, Debug, PartialEq)]
pub struct Paragraph {
    pub runs: Vec<Run>,
    pub align: Align,
    pub wrap: bool,
    /// The tallest line height its runs ask for.
    pub line_height: f32,
    /// Cut with "…" where it overflows, rather than wrapped or clipped.
    pub ellipsis: bool,
}

impl Paragraph {
    pub fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}

/// How the renderer's fonts measure text.
pub trait Fonts {
    /// The size of `p` laid out at most `width` wide, or as wide as it
    /// likes for `None`.
    fn measure(&self, p: &Paragraph, width: Option<f32>) -> (f32, f32);
    /// The narrowest `p` can be: its widest word.
    fn min_width(&self, p: &Paragraph) -> f32;
    /// Which run is at `(x, y)` from the corner of `p` laid out `width`
    /// wide — what a tap there lands on.
    fn run_at(&self, p: &Paragraph, width: f32, x: f32, y: f32) -> Option<usize>;
}

/// Fonts for where there are none: each glyph a fixed share of its size,
/// lines broken at spaces. Deterministic, so the suite can read boxes.
pub struct Approx;

impl Approx {
    fn advance(c: char, f: &Font) -> f32 {
        let share = if f.mono {
            0.6
        } else if c.is_ascii_uppercase() || c == 'm' || c == 'w' {
            0.65
        } else if " .,:;'!|il".contains(c) {
            0.3
        } else {
            0.5
        };
        f.size * share * if f.bold { 1.05 } else { 1.0 } + f.letter_spacing
    }

    /// The paragraph's text as words and spaces, each with its width; a
    /// forced break as `None`.
    fn pieces(p: &Paragraph) -> Vec<Option<(bool, f32)>> {
        let mut out: Vec<Option<(bool, f32)>> = Vec::new();
        for run in &p.runs {
            for c in run.text.chars() {
                if c == '\n' {
                    out.push(None);
                    continue;
                }
                let space = c == ' ';
                let w = Self::advance(c, &run.font);
                match out.last_mut() {
                    Some(Some((s, width))) if *s == space && !space => *width += w,
                    _ => out.push(Some((space, w))),
                }
            }
        }
        out
    }
}

impl Fonts for Approx {
    fn measure(&self, p: &Paragraph, width: Option<f32>) -> (f32, f32) {
        let limit = if p.wrap { width } else { None };
        let mut widest: f32 = 0.0;
        let mut lines = 1usize;
        let mut line: f32 = 0.0;
        // The word being read: wrapped as a whole when it does not fit.
        let mut word: f32 = 0.0;
        let flush = |word: &mut f32, line: &mut f32, lines: &mut usize, widest: &mut f32| {
            if *word > 0.0 {
                if *line > 0.0 && limit.is_some_and(|l| *line + *word > l + 0.01) {
                    *widest = widest.max(*line);
                    *lines += 1;
                    *line = 0.0;
                }
                *line += *word;
                *word = 0.0;
            }
        };
        for run in &p.runs {
            for c in run.text.chars() {
                match c {
                    '\n' => {
                        flush(&mut word, &mut line, &mut lines, &mut widest);
                        widest = widest.max(line);
                        line = 0.0;
                        lines += 1;
                    }
                    ' ' => {
                        flush(&mut word, &mut line, &mut lines, &mut widest);
                        // A space that ends a line is not drawn on the next.
                        if !(line == 0.0 && lines > 1) {
                            line += Self::advance(c, &run.font);
                        }
                    }
                    _ => word += Self::advance(c, &run.font),
                }
            }
        }
        flush(&mut word, &mut line, &mut lines, &mut widest);
        widest = widest.max(line);
        if let (Some(w), true) = (width, p.ellipsis) {
            widest = widest.min(w);
        }
        (widest.ceil(), lines as f32 * p.line_height)
    }

    fn run_at(&self, p: &Paragraph, width: f32, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let want_line = (y / p.line_height) as usize;
        let mut line = 0usize;
        let mut at: f32 = 0.0;
        for (i, run) in p.runs.iter().enumerate() {
            for c in run.text.chars() {
                if c == '\n' {
                    line += 1;
                    at = 0.0;
                    continue;
                }
                let w = Self::advance(c, &run.font);
                if p.wrap && at > 0.0 && at + w > width + 0.01 && c != ' ' {
                    line += 1;
                    at = 0.0;
                }
                if line == want_line && x >= at && x < at + w {
                    return Some(i);
                }
                at += w;
            }
        }
        None
    }

    fn min_width(&self, p: &Paragraph) -> f32 {
        if !p.wrap {
            return self.measure(p, None).0;
        }
        Self::pieces(p)
            .into_iter()
            .flatten()
            .filter(|(space, _)| !space)
            .map(|(_, w)| w)
            .fold(0.0, f32::max)
            .ceil()
    }
}

/// A box on the page, in logical pixels from the page's top-left corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    /// Shrunk by `[top, right, bottom, left]`.
    pub fn inset(&self, e: [f32; 4]) -> Rect {
        Rect {
            x: self.x + e[3],
            y: self.y + e[0],
            w: (self.w - e[1] - e[3]).max(0.0),
            h: (self.h - e[0] - e[2]).max(0.0),
        }
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = (self.x + self.w).min(o.x + o.w);
        let b = (self.y + self.h).min(o.y + o.h);
        Rect {
            x,
            y,
            w: (r - x).max(0.0),
            h: (b - y).max(0.0),
        }
    }
}

/// What a box holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Element(NodeId),
    /// Anonymous: inline content, as text.
    Text(Paragraph),
    /// Anonymous: inline content with boxes in it, as a wrapping row.
    Line,
}

/// One box, laid out.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub content: Content,
    /// The border box, on the page.
    pub rect: Rect,
    /// top, right, bottom, left.
    pub border: [f32; 4],
    pub padding: [f32; 4],
    /// In the order they are drawn: the tree's, then by `z-index`.
    pub children: Vec<usize>,
    /// How far its content reaches from the padding box's corner — more
    /// than the padding box when it overflows.
    pub extent: (f32, f32),
    /// A list item's number among its siblings, from 1.
    pub ordinal: u32,
}

impl Placed {
    pub fn padding_rect(&self) -> Rect {
        self.rect.inset(self.border)
    }

    pub fn content_rect(&self) -> Rect {
        self.padding_rect().inset(self.padding)
    }
}

/// The page, laid out: boxes in a tree from `boxes[0]`, the root.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub boxes: Vec<Placed>,
    pub by_node: NodeMap<usize>,
    /// Each text node's paragraph.
    pub by_text: NodeMap<usize>,
    /// The page's size: its root's box.
    pub width: f32,
    pub height: f32,
}

impl Layout {
    /// An element's border box, or a text node's paragraph.
    pub fn rect_of(&self, id: NodeId) -> Option<Rect> {
        self.by_node
            .get(&id)
            .or_else(|| self.by_text.get(&id))
            .map(|&i| self.boxes[i].rect)
    }
}

/// What a leaf element measures as: the controls and images that have a
/// size of their own.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Replaced {
    None,
    /// A text field: `lines` of text, `width` wide before CSS says.
    Field {
        lines: f32,
        line: f32,
        width: f32,
    },
    Image {
        w: f32,
        h: f32,
    },
    Fixed {
        w: f32,
        h: f32,
    },
}

/// A `calc()` value, by pointer, for taffy: `px + percent% of the basis`.
/// Eight-aligned, as taffy keeps tag bits in the pointer's low three.
#[repr(align(8))]
struct Calc {
    px: f32,
    percent: f32,
}

fn calc_value(ptr: *const (), basis: f32) -> f32 {
    // SAFETY: every calc pointer taffy holds comes from `Tree::calc`, which
    // boxes the value in `Tree::calcs`; the tree outlives the layout pass,
    // and its boxes are never moved or freed during it.
    let c = unsafe { &*(ptr as *const Calc) };
    c.px + basis * c.percent / 100.0
}

/// A width asked for, rounded, and the size the text came to at it.
type Measured = (Option<f32>, (f32, f32));

struct TBox {
    content: Content,
    style: tf::Style,
    children: Vec<usize>,
    hoisted: Vec<tf::NodeId>,
    cache: tf::Cache,
    unrounded: tf::Layout,
    layout: tf::Layout,
    replaced: Replaced,
    min_width: Cell<Option<f32>>,
    /// What its text measured at the widths asked so far: taffy asks the same
    /// width again under other constraints, and a measure is the dear part.
    measured: std::cell::RefCell<Vec<Measured>>,
    ordinal: u32,
    /// Paint before or after its siblings: `z-index`, and whether it is
    /// positioned at all.
    z: (i32, bool),
}

struct Tree<'a> {
    boxes: Vec<TBox>,
    /// Boxed so each stays where taffy's pointer to it says as the list
    /// grows: a `Vec<Calc>` would move them when it reallocates.
    #[allow(clippy::vec_box)]
    calcs: Vec<Box<Calc>>,
    fonts: &'a dyn Fonts,
    by_node: NodeMap<usize>,
}

/// An item of a block container's content, before it is grouped.
enum Item {
    Block(NodeId),
    Inline(Piece),
}

/// A piece of inline content.
enum Piece {
    Text {
        text: String,
        font: Font,
        node: NodeId,
        ws: WhiteSpace,
        transform: st::TextTransform,
    },
    Break {
        font: Font,
        node: NodeId,
    },
    Atomic(NodeId),
}

/// Lay out `doc` on a page `width` × `height`, with `styles` computed for it.
/// `images` gives an image's size by its `image-load` number.
pub fn layout(
    doc: &Document,
    styles: &NodeMap<Style>,
    fonts: &dyn Fonts,
    images: &dyn Fn(u32) -> Option<(f32, f32)>,
    width: f32,
    height: f32,
) -> Layout {
    let mut tree = Tree {
        // A box is large — taffy's cache is in it — so the list is sized
        // once rather than moved as it grows.
        boxes: Vec::with_capacity(doc.len() + doc.len() / 4),
        calcs: Vec::new(),
        fonts,
        by_node: NodeMap::default(),
    };
    let Some(root_style) = styles.get(&ROOT) else {
        return Layout::default();
    };
    let root = tree.element(doc, styles, images, ROOT, root_style, false);
    tf::compute_root_layout(
        &mut tree,
        tf::NodeId::from(root),
        tf::Size {
            width: tf::AvailableSpace::Definite(width),
            height: tf::AvailableSpace::Definite(height),
        },
    );
    tf::round_layout(&mut tree, tf::NodeId::from(root));

    // Where each box is on the page — the way taffy rounded it: in-flow
    // children from their parent, out-of-flow ones from the containing
    // block that took them.
    let mut rects = vec![Rect::default(); tree.boxes.len()];
    let mut stack = vec![(root, 0.0f32, 0.0f32)];
    while let Some((i, px, py)) = stack.pop() {
        let l = &tree.boxes[i].layout;
        let (x, y) = (finite(px + l.location.x), finite(py + l.location.y));
        rects[i] = Rect {
            x,
            y,
            w: finite(l.size.width).max(0.0),
            h: finite(l.size.height).max(0.0),
        };
        for &c in &tree.boxes[i].children {
            if !tree.boxes[c].style.position.is_out_of_flow() {
                stack.push((c, x, y));
            }
        }
        for h in &tree.boxes[i].hoisted {
            stack.push((usize::from(*h), x, y));
        }
    }

    let boxes = tree
        .boxes
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let l = &b.layout;
            let mut children = b.children.clone();
            children.sort_by_key(|&c| tree.boxes[c].z);
            let ov = l.scrollable_overflow_rect;
            Placed {
                content: b.content.clone(),
                rect: rects[i],
                border: [l.border.top, l.border.right, l.border.bottom, l.border.left].map(finite),
                padding: [
                    l.padding.top,
                    l.padding.right,
                    l.padding.bottom,
                    l.padding.left,
                ]
                .map(finite),
                children,
                extent: (
                    finite(ov.right - l.border.left).max(0.0),
                    finite(ov.bottom - l.border.top).max(0.0),
                ),
                ordinal: b.ordinal,
            }
        })
        .collect::<Vec<_>>();
    let mut by_text = NodeMap::default();
    for (i, b) in boxes.iter().enumerate() {
        if let Content::Text(p) = &b.content {
            for r in &p.runs {
                by_text.entry(r.node).or_insert(i);
            }
        }
    }
    let (w, h) = (rects[root].w, rects[root].h);
    Layout {
        boxes,
        by_node: tree.by_node,
        by_text,
        width: w,
        height: h,
    }
}

/// A number layout produced, or 0 if its arithmetic ran off the end: what
/// is drawn, scrolled and hit-tested is never NaN or infinite.
fn finite(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

impl Tree<'_> {
    fn push(&mut self, content: Content, style: tf::Style, replaced: Replaced) -> usize {
        self.boxes.push(TBox {
            content,
            style,
            children: Vec::new(),
            hoisted: Vec::new(),
            cache: tf::Cache::new(),
            unrounded: tf::Layout::with_order(0),
            layout: tf::Layout::with_order(0),
            replaced,
            min_width: Cell::new(None),
            measured: std::cell::RefCell::new(Vec::new()),
            ordinal: 0,
            z: (0, false),
        });
        self.boxes.len() - 1
    }

    /// The box of element `id` and what is under it. `item` is whether its
    /// parent lays it out as a flex or grid item — which makes an inline
    /// element a block.
    fn element(
        &mut self,
        doc: &Document,
        styles: &NodeMap<Style>,
        images: &dyn Fn(u32) -> Option<(f32, f32)>,
        id: NodeId,
        s: &Style,
        _item: bool,
    ) -> usize {
        let e = doc.nodes[&id].element().expect("an element");
        let replaced = replaced(doc, id, e, s, images);
        let mut ts = self.taffy_style(s);
        if let Replaced::Image { w, h } = replaced {
            if s.aspect_ratio.is_none() && w > 0.0 && h > 0.0 {
                ts.aspect_ratio = Some(w / h);
            }
            ts.item_is_replaced = true;
        }
        let at = self.push(Content::Element(id), ts, replaced);
        self.by_node.insert(id, at);
        let positioned = !matches!(s.position, st::Position::Static);
        self.boxes[at].z = (s.z_index.unwrap_or(0), positioned);
        if replaced != Replaced::None || is_leaf_control(e) {
            return at;
        }
        let children = match s.display {
            Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid => {
                self.items(doc, styles, images, id, s)
            }
            _ => self.blocks(doc, styles, images, id, s),
        };
        // Number list items among their siblings.
        let mut n = 0;
        for &c in &children {
            if let Content::Element(cid) = self.boxes[c].content {
                if styles
                    .get(&cid)
                    .is_some_and(|cs| cs.display == Display::ListItem)
                {
                    n += 1;
                    self.boxes[c].ordinal = n;
                }
            }
        }
        self.boxes[at].children = children;
        at
    }

    /// The children of `id`, with `display: contents` taken out of the way
    /// and `display: none` left out.
    fn flat_children(doc: &Document, styles: &NodeMap<Style>, id: NodeId, out: &mut Vec<NodeId>) {
        for &c in &doc.nodes[&id].children {
            match styles.get(&c).map(|s| s.display) {
                Some(Display::None) => {}
                Some(Display::Contents) => Self::flat_children(doc, styles, c, out),
                _ => out.push(c),
            }
        }
    }

    /// A flex or grid container's items: each element a box, each run of
    /// text between them a paragraph.
    fn items(
        &mut self,
        doc: &Document,
        styles: &NodeMap<Style>,
        images: &dyn Fn(u32) -> Option<(f32, f32)>,
        id: NodeId,
        s: &Style,
    ) -> Vec<usize> {
        let mut kids = Vec::new();
        Self::flat_children(doc, styles, id, &mut kids);
        let mut out = Vec::new();
        let mut pending: Vec<Piece> = Vec::new();
        for c in kids {
            match &doc.nodes[&c].kind {
                Kind::Text(t) => pending.push(text_piece(t, s, c)),
                Kind::Element(_) => {
                    if let Some(b) = self.paragraph_box(std::mem::take(&mut pending), s, true, true)
                    {
                        out.push(b);
                    }
                    let cs = &styles[&c];
                    out.push(self.element(doc, styles, images, c, cs, true));
                }
            }
        }
        if let Some(b) = self.paragraph_box(pending, s, true, true) {
            out.push(b);
        }
        out
    }

    /// A block container's children: block-level boxes, with the inline
    /// content between them gathered into paragraphs and lines.
    fn blocks(
        &mut self,
        doc: &Document,
        styles: &NodeMap<Style>,
        images: &dyn Fn(u32) -> Option<(f32, f32)>,
        id: NodeId,
        s: &Style,
    ) -> Vec<usize> {
        let mut kids = Vec::new();
        Self::flat_children(doc, styles, id, &mut kids);
        let mut items = Vec::new();
        for c in kids {
            classify(doc, styles, c, s, &mut items);
        }
        let mut out = Vec::new();
        let mut run: Vec<Piece> = Vec::new();
        for item in items {
            match item {
                Item::Inline(p) => run.push(p),
                Item::Block(b) => {
                    self.flush(doc, styles, images, std::mem::take(&mut run), s, &mut out);
                    let bs = &styles[&b];
                    out.push(self.element(doc, styles, images, b, bs, false));
                }
            }
        }
        self.flush(doc, styles, images, run, s, &mut out);
        out
    }

    /// Inline content as boxes: one paragraph, or a wrapping row of
    /// paragraphs and the boxes among them.
    fn flush(
        &mut self,
        doc: &Document,
        styles: &NodeMap<Style>,
        images: &dyn Fn(u32) -> Option<(f32, f32)>,
        run: Vec<Piece>,
        s: &Style,
        out: &mut Vec<usize>,
    ) {
        if run.is_empty() {
            return;
        }
        if !run.iter().any(|p| matches!(p, Piece::Atomic(_))) {
            if let Some(b) = self.paragraph_box(run, s, true, true) {
                out.push(b);
            }
            return;
        }
        let line_style = tf::Style {
            display: tf::Display::Flex,
            flex_wrap: tf::FlexWrap::Wrap,
            align_items: tf::AlignItems::CENTER,
            justify_content: match s.text_align {
                st::TextAlign::Center => tf::JustifyContent::CENTER,
                st::TextAlign::Right | st::TextAlign::End => tf::JustifyContent::FLEX_END,
                _ => tf::JustifyContent::FLEX_START,
            },
            ..tf::Style::DEFAULT
        };
        let line = self.push(Content::Line, line_style, Replaced::None);
        let mut children = Vec::new();
        let mut segment: Vec<Piece> = Vec::new();
        let mut first = true;
        for p in run {
            match p {
                Piece::Atomic(a) => {
                    if let Some(b) =
                        self.paragraph_box(std::mem::take(&mut segment), s, first, false)
                    {
                        children.push(b);
                    }
                    first = false;
                    let a_style = &styles[&a];
                    children.push(self.element(doc, styles, images, a, a_style, true));
                }
                other => segment.push(other),
            }
        }
        if let Some(b) = self.paragraph_box(segment, s, first, true) {
            children.push(b);
        }
        self.boxes[line].children = children;
        out.push(line);
    }

    /// A paragraph's box, or nothing if the text is only spaces that
    /// collapse away. `first` and `last` say whether its edges are the edges
    /// of a line, where spaces go.
    fn paragraph_box(
        &mut self,
        pieces: Vec<Piece>,
        s: &Style,
        first: bool,
        last: bool,
    ) -> Option<usize> {
        let p = paragraph(pieces, s, first, last)?;
        let style = tf::Style {
            display: tf::Display::Block,
            ..tf::Style::DEFAULT
        };
        Some(self.push(Content::Text(p), style, Replaced::None))
    }

    /// A `calc()` value taffy can hold.
    fn calc(&mut self, px: f32, percent: f32) -> *const () {
        let b = Box::new(Calc { px, percent });
        let ptr = &*b as *const Calc as *const ();
        self.calcs.push(b);
        ptr
    }

    fn lpa(&mut self, l: Length) -> tf::LengthPercentageAuto {
        match l {
            Length::Auto => tf::LengthPercentageAuto::auto(),
            Length::Px(p) => tf::LengthPercentageAuto::length(p),
            Length::Percent(p) => tf::LengthPercentageAuto::percent(p / 100.0),
            Length::Calc { px, percent } => tf::LengthPercentageAuto::calc(self.calc(px, percent)),
        }
    }

    fn lp(&mut self, l: Length) -> tf::LengthPercentage {
        match l {
            Length::Auto => tf::LengthPercentage::length(0.0),
            Length::Px(p) => tf::LengthPercentage::length(p),
            Length::Percent(p) => tf::LengthPercentage::percent(p / 100.0),
            Length::Calc { px, percent } => tf::LengthPercentage::calc(self.calc(px, percent)),
        }
    }

    fn dim(&mut self, l: Length) -> tf::Dimension {
        match l {
            Length::Auto => tf::Dimension::auto(),
            Length::Px(p) => tf::Dimension::length(p),
            Length::Percent(p) => tf::Dimension::percent(p / 100.0),
            Length::Calc { px, percent } => tf::Dimension::calc(self.calc(px, percent)),
        }
    }

    fn min_track(&mut self, t: &st::Track) -> tf::MinTrackSizingFunction {
        use tf::MinTrackSizingFunction as M;
        match t {
            st::Track::Fixed(Length::Px(p)) => M::length(*p),
            st::Track::Fixed(Length::Percent(p)) => M::percent(p / 100.0),
            st::Track::Fixed(Length::Calc { px, percent }) => M::calc(self.calc(*px, *percent)),
            st::Track::MinContent => M::min_content(),
            st::Track::MaxContent => M::max_content(),
            _ => M::auto(),
        }
    }

    fn max_track(&mut self, t: &st::Track) -> tf::MaxTrackSizingFunction {
        use tf::MaxTrackSizingFunction as M;
        match t {
            st::Track::Fixed(Length::Px(p)) => M::length(*p),
            st::Track::Fixed(Length::Percent(p)) => M::percent(p / 100.0),
            st::Track::Fixed(Length::Calc { px, percent }) => M::calc(self.calc(*px, *percent)),
            st::Track::Fr(f) => M::fr(*f),
            st::Track::MinContent => M::min_content(),
            st::Track::MaxContent => M::max_content(),
            st::Track::FitContent(Length::Px(p)) => M::fit_content_px(*p),
            st::Track::FitContent(Length::Percent(p)) => M::fit_content_percent(p / 100.0),
            _ => M::auto(),
        }
    }

    fn track(&mut self, t: &st::Track) -> tf::TrackSizingFunction {
        match t {
            st::Track::MinMax(a, b) => tf::MinMax {
                min: self.min_track(a),
                max: self.max_track(b),
            },
            other => tf::MinMax {
                min: self.min_track(other),
                max: self.max_track(other),
            },
        }
    }

    fn template(&mut self, list: &[st::TrackEntry]) -> Vec<tf::GridTemplateComponent<String>> {
        list.iter()
            .map(|e| match e {
                st::TrackEntry::One(t) => tf::GridTemplateComponent::Single(self.track(t)),
                st::TrackEntry::Repeat(count, tracks) => {
                    let tracks: Vec<_> = tracks.iter().map(|t| self.track(t)).collect();
                    let n = tracks.len();
                    tf::GridTemplateComponent::Repeat(tf::GridTemplateRepetition {
                        count: match count {
                            st::Repeat::Count(c) => tf::RepetitionCount::Count(*c),
                            st::Repeat::AutoFill => tf::RepetitionCount::AutoFill,
                            st::Repeat::AutoFit => tf::RepetitionCount::AutoFit,
                        },
                        tracks,
                        line_names: vec![Vec::new(); n + 1],
                    })
                }
            })
            .collect()
    }

    /// The computed style, as taffy reads it.
    fn taffy_style(&mut self, s: &Style) -> tf::Style {
        let display = match s.display {
            Display::None => tf::Display::None,
            Display::Flex | Display::InlineFlex => tf::Display::Flex,
            Display::Grid | Display::InlineGrid => tf::Display::Grid,
            _ => tf::Display::Block,
        };
        let overflow = |o: st::Overflow| match o {
            st::Overflow::Visible => tf::Overflow::Visible,
            st::Overflow::Hidden => tf::Overflow::Hidden,
            st::Overflow::Clip => tf::Overflow::Clip,
            st::Overflow::Scroll | st::Overflow::Auto => tf::Overflow::Scroll,
        };
        let items = |a: st::Align| -> Option<tf::AlignItems> {
            Some(match a {
                st::Align::Start | st::Align::Left => tf::AlignItems::START,
                st::Align::End | st::Align::Right => tf::AlignItems::END,
                st::Align::FlexStart => tf::AlignItems::FLEX_START,
                st::Align::FlexEnd => tf::AlignItems::FLEX_END,
                st::Align::SelfStart => tf::AlignItems::SELF_START,
                st::Align::SelfEnd => tf::AlignItems::SELF_END,
                st::Align::Center => tf::AlignItems::CENTER,
                st::Align::Stretch => tf::AlignItems::STRETCH,
                st::Align::Baseline => tf::AlignItems::BASELINE,
                _ => return None,
            })
        };
        let content = |a: st::Align| -> Option<tf::AlignContent> {
            Some(match a {
                st::Align::Start | st::Align::Left => tf::AlignContent::START,
                st::Align::End | st::Align::Right => tf::AlignContent::END,
                st::Align::FlexStart => tf::AlignContent::FLEX_START,
                st::Align::FlexEnd => tf::AlignContent::FLEX_END,
                st::Align::Center => tf::AlignContent::CENTER,
                st::Align::Stretch => tf::AlignContent::STRETCH,
                st::Align::SpaceBetween => tf::AlignContent::SPACE_BETWEEN,
                st::Align::SpaceAround => tf::AlignContent::SPACE_AROUND,
                st::Align::SpaceEvenly => tf::AlignContent::SPACE_EVENLY,
                _ => return None,
            })
        };
        let line = |l: st::Line| match l {
            st::Line::Auto => tf::GridPlacement::Auto,
            st::Line::Index(i) => tf::prelude::line(i),
            st::Line::Span(n) => tf::GridPlacement::Span(n),
        };
        let border = |w: f32| tf::LengthPercentage::length(w);
        tf::Style {
            display,
            box_sizing: match s.box_sizing {
                st::BoxSizing::BorderBox => tf::BoxSizing::BorderBox,
                st::BoxSizing::ContentBox => tf::BoxSizing::ContentBox,
            },
            overflow: tf::Point {
                x: overflow(s.overflow[0]),
                y: overflow(s.overflow[1]),
            },
            scrollbar_width: 0.0,
            float: match s.float {
                st::Float::Left => tf::Float::Left,
                st::Float::Right => tf::Float::Right,
                st::Float::None => tf::Float::None,
            },
            clear: match s.clear {
                st::Clear::Left => tf::Clear::Left,
                st::Clear::Right => tf::Clear::Right,
                st::Clear::Both => tf::Clear::Both,
                st::Clear::None => tf::Clear::None,
            },
            position: match s.position {
                st::Position::Static => tf::Position::Static,
                st::Position::Relative | st::Position::Sticky => tf::Position::Relative,
                st::Position::Absolute | st::Position::Fixed => tf::Position::Absolute,
            },
            inset: tf::Rect {
                top: self.lpa(s.inset[0]),
                right: self.lpa(s.inset[1]),
                bottom: self.lpa(s.inset[2]),
                left: self.lpa(s.inset[3]),
            },
            size: tf::Size {
                width: self.dim(s.width),
                height: self.dim(s.height),
            },
            min_size: tf::Size {
                width: self.lpa(s.min_width),
                height: self.lpa(s.min_height),
            },
            max_size: tf::Size {
                width: self.lpa(s.max_width),
                height: self.lpa(s.max_height),
            },
            aspect_ratio: s.aspect_ratio,
            margin: tf::Rect {
                top: self.lpa(s.margin[0]),
                right: self.lpa(s.margin[1]),
                bottom: self.lpa(s.margin[2]),
                left: self.lpa(s.margin[3]),
            },
            padding: tf::Rect {
                top: self.lp(s.padding[0]),
                right: self.lp(s.padding[1]),
                bottom: self.lp(s.padding[2]),
                left: self.lp(s.padding[3]),
            },
            border: tf::Rect {
                top: border(s.border_width[0]),
                right: border(s.border_width[1]),
                bottom: border(s.border_width[2]),
                left: border(s.border_width[3]),
            },
            align_items: items(s.align_items).unwrap_or(tf::AlignItems::NORMAL),
            align_self: items(s.align_self),
            justify_items: items(s.justify_items).unwrap_or(tf::AlignItems::NORMAL),
            justify_self: items(s.justify_self),
            align_content: content(s.align_content).unwrap_or(tf::AlignContent::NORMAL),
            justify_content: content(s.justify_content).unwrap_or(tf::AlignContent::NORMAL),
            gap: tf::Size {
                width: self.lp(s.gap[1]),
                height: self.lp(s.gap[0]),
            },
            flex_direction: match s.flex_direction {
                st::FlexDirection::Row => tf::FlexDirection::Row,
                st::FlexDirection::RowReverse => tf::FlexDirection::RowReverse,
                st::FlexDirection::Column => tf::FlexDirection::Column,
                st::FlexDirection::ColumnReverse => tf::FlexDirection::ColumnReverse,
            },
            flex_wrap: match s.flex_wrap {
                st::FlexWrap::NoWrap => tf::FlexWrap::NoWrap,
                st::FlexWrap::Wrap => tf::FlexWrap::Wrap,
                st::FlexWrap::WrapReverse => tf::FlexWrap::WrapReverse,
            },
            flex_basis: self.dim(s.flex_basis),
            flex_grow: s.flex_grow,
            flex_shrink: s.flex_shrink,
            grid_template_columns: self.template(&s.grid_template_columns),
            grid_template_rows: self.template(&s.grid_template_rows),
            grid_auto_columns: s.grid_auto_columns.iter().map(|t| self.track(t)).collect(),
            grid_auto_rows: s.grid_auto_rows.iter().map(|t| self.track(t)).collect(),
            grid_auto_flow: match (s.grid_auto_flow_column, s.grid_auto_flow_dense) {
                (false, false) => tf::GridAutoFlow::Row,
                (true, false) => tf::GridAutoFlow::Column,
                (false, true) => tf::GridAutoFlow::RowDense,
                (true, true) => tf::GridAutoFlow::ColumnDense,
            },
            grid_row: tf::Line {
                start: line(s.grid_row.start),
                end: line(s.grid_row.end),
            },
            grid_column: tf::Line {
                start: line(s.grid_column.start),
                end: line(s.grid_column.end),
            },
            ..tf::Style::DEFAULT
        }
    }

    /// Lay out one box, as taffy asks.
    fn compute(
        &mut self,
        node: tf::NodeId,
        inputs: tf::LayoutInput,
        block_ctx: Option<&mut tf::BlockContext<'_>>,
    ) -> tf::LayoutOutput {
        if inputs.run_mode == tf::RunMode::PerformHiddenLayout {
            return tf::compute_hidden_layout(self, node);
        }
        tf::compute_cached_layout(self, node, inputs, |tree, node, inputs| {
            let i = usize::from(node);
            let display = tree.boxes[i].style.display;
            let has_children = !tree.boxes[i].children.is_empty();
            let mut output = match (display, has_children) {
                (tf::Display::None, _) => tf::compute_hidden_layout(tree, node),
                (tf::Display::Flex, true) => tf::compute_flexbox_layout(tree, node, inputs),
                (tf::Display::Grid, true) => tf::compute_grid_layout(tree, node, inputs),
                (_, true) => tf::compute_block_layout(tree, node, inputs, block_ctx),
                (_, false) => {
                    let b = &tree.boxes[i];
                    let fonts = tree.fonts;
                    tf::compute_leaf_layout(inputs, &b.style, calc_value, |known, available| {
                        measure(b, fonts, known, available)
                    })
                }
            };
            if inputs.run_mode == tf::RunMode::PerformLayout {
                tf::compute_oof_layout(tree, node, &mut output);
            }
            output
        })
    }
}

/// `measure` at `width`, from the box's memo when it has been asked before.
fn memo(b: &TBox, width: Option<f32>, measure: impl FnOnce() -> (f32, f32)) -> (f32, f32) {
    let key = width.map(|w| (w * 64.0).round() / 64.0);
    if let Some(&(_, size)) = b.measured.borrow().iter().find(|(k, _)| *k == key) {
        return size;
    }
    let size = measure();
    let mut m = b.measured.borrow_mut();
    if m.len() >= 8 {
        m.remove(0);
    }
    m.push((key, size));
    size
}

/// What a leaf measures as, given what taffy already knows.
fn measure(
    b: &TBox,
    fonts: &dyn Fonts,
    known: tf::Size<Option<f32>>,
    available: tf::Size<tf::AvailableSpace>,
) -> tf::Size<f32> {
    if let (Some(width), Some(height)) = (known.width, known.height) {
        return tf::Size { width, height };
    }
    match &b.content {
        Content::Text(p) => {
            let min = || {
                if let Some(m) = b.min_width.get() {
                    return m;
                }
                let m = fonts.min_width(p);
                b.min_width.set(Some(m));
                m
            };
            let width = known.width.or(match available.width {
                tf::AvailableSpace::Definite(w) => Some(w),
                tf::AvailableSpace::MinContent => Some(min()),
                tf::AvailableSpace::MaxContent => None,
            });
            let (w, h) = memo(b, width, || fonts.measure(p, width));
            tf::Size {
                width: known.width.unwrap_or(match available.width {
                    tf::AvailableSpace::MinContent => min(),
                    _ => w,
                }),
                height: known.height.unwrap_or(h),
            }
        }
        _ => {
            let (w, h) = match b.replaced {
                Replaced::None => (0.0, 0.0),
                Replaced::Field { lines, line, width } => (width, lines * line),
                Replaced::Fixed { w, h } => (w, h),
                Replaced::Image { w, h } => match (known.width, known.height) {
                    (Some(kw), None) if w > 0.0 => (kw, kw * h / w),
                    (None, Some(kh)) if h > 0.0 => (kh * w / h, kh),
                    _ => (w, h),
                },
            };
            tf::Size {
                width: known.width.unwrap_or(w),
                height: known.height.unwrap_or(h),
            }
        }
    }
}

/// Controls whose content is the host's to draw, not their children.
fn is_leaf_control(e: &super::Element) -> bool {
    matches!(
        e.tag.as_str(),
        "input" | "textarea" | "img" | "progress" | "select" | "br" | "hr"
    )
}

fn replaced(
    doc: &Document,
    id: NodeId,
    e: &super::Element,
    s: &Style,
    images: &dyn Fn(u32) -> Option<(f32, f32)>,
) -> Replaced {
    let line = s.line_height.px(s.font_size);
    match e.tag.as_str() {
        // As wide as its longest option, roughly; the arrow is in its padding.
        "select" => Replaced::Field {
            lines: 1.0,
            line,
            width: doc
                .options(id)
                .iter()
                .map(|(_, label)| label.chars().count())
                .max()
                .unwrap_or(4) as f32
                * s.font_size
                * 0.55,
        },
        "img" => match e
            .attr("src")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .and_then(images)
        {
            Some((w, h)) => Replaced::Image { w, h },
            None => Replaced::Fixed { w: 0.0, h: 0.0 },
        },
        "textarea" => Replaced::Field {
            lines: e
                .attr("rows")
                .and_then(|r| r.parse::<f32>().ok())
                .unwrap_or(3.0)
                .clamp(1.0, 100.0),
            line,
            width: s.font_size * 16.0,
        },
        "input" if e.is_toggle() => Replaced::Fixed { w: 0.0, h: 0.0 },
        "input" if e.input_type() == "range" => Replaced::Fixed { w: 160.0, h: 16.0 },
        "input" => Replaced::Field {
            lines: 1.0,
            line,
            width: s.font_size * 11.0,
        },
        "progress" => Replaced::Fixed { w: 160.0, h: 8.0 },
        _ => Replaced::None,
    }
}

/// A text node as a piece of inline content, in its parent's style.
fn text_piece(t: &str, parent: &Style, node: NodeId) -> Piece {
    Piece::Text {
        text: t.to_string(),
        font: Font::of(parent),
        node,
        ws: parent.white_space,
        transform: parent.text_transform,
    }
}

/// Where a block container's child goes: a block of its own, or into the
/// inline content around it — an inline element as the pieces of its text.
fn classify(
    doc: &Document,
    styles: &NodeMap<Style>,
    id: NodeId,
    parent: &Style,
    out: &mut Vec<Item>,
) {
    match &doc.nodes[&id].kind {
        Kind::Text(t) => out.push(Item::Inline(text_piece(t, parent, id))),
        Kind::Element(e) => {
            let Some(s) = styles.get(&id) else {
                return;
            };
            let out_of_flow = s.float != st::Float::None
                || matches!(s.position, st::Position::Absolute | st::Position::Fixed);
            match s.display {
                Display::None => {}
                Display::Contents => {
                    for &c in &doc.nodes[&id].children {
                        classify(doc, styles, c, s, out);
                    }
                }
                _ if out_of_flow => out.push(Item::Block(id)),
                Display::Inline if e.tag == "br" => out.push(Item::Inline(Piece::Break {
                    font: Font::of(s),
                    node: id,
                })),
                Display::Inline if !is_leaf_control(e) => {
                    for &c in &doc.nodes[&id].children {
                        classify(doc, styles, c, s, out);
                    }
                }
                Display::Inline
                | Display::InlineBlock
                | Display::InlineFlex
                | Display::InlineGrid => out.push(Item::Inline(Piece::Atomic(id))),
                _ => out.push(Item::Block(id)),
            }
        }
    }
}

/// Inline pieces as a paragraph — spaces collapsed as each piece's
/// `white-space` says, and text transformed — or `None` if nothing is left.
fn paragraph(pieces: Vec<Piece>, container: &Style, first: bool, last: bool) -> Option<Paragraph> {
    let mut runs: Vec<Run> = Vec::new();
    // At a line's start, a collapsible space goes.
    let mut prev_space = first;
    for p in pieces {
        match p {
            Piece::Atomic(_) => {}
            Piece::Break { font, node } => {
                trim_end(&mut runs);
                runs.push(Run {
                    text: "\n".into(),
                    font,
                    node,
                });
                prev_space = true;
            }
            Piece::Text {
                text,
                font,
                node,
                ws,
                transform,
            } => {
                let mut out = String::new();
                match ws {
                    WhiteSpace::Pre | WhiteSpace::PreWrap => {
                        out = text.replace('\t', "    ");
                        prev_space = out.ends_with([' ', '\n']);
                    }
                    WhiteSpace::PreLine | WhiteSpace::Normal | WhiteSpace::NoWrap => {
                        let keep_newlines = ws == WhiteSpace::PreLine;
                        for c in text.chars() {
                            if c == '\n' && keep_newlines {
                                while out.ends_with(' ') {
                                    out.pop();
                                }
                                out.push('\n');
                                prev_space = true;
                            } else if c.is_whitespace() {
                                if !prev_space {
                                    out.push(' ');
                                    prev_space = true;
                                }
                            } else {
                                out.push(c);
                                prev_space = false;
                            }
                        }
                    }
                }
                let out = match transform {
                    st::TextTransform::None => out,
                    st::TextTransform::Uppercase => out.to_uppercase(),
                    st::TextTransform::Lowercase => out.to_lowercase(),
                    st::TextTransform::Capitalize => capitalize(&out),
                };
                if !out.is_empty() {
                    runs.push(Run {
                        text: out,
                        font,
                        node,
                    });
                }
            }
        }
    }
    if last {
        trim_end(&mut runs);
    }
    if runs.iter().all(|r| r.text.trim().is_empty()) && !runs.iter().any(|r| r.text.contains('\n'))
    {
        // Spaces alone between two boxes still part them, like a word.
        if first || last || runs.is_empty() {
            return None;
        }
    }
    let line_height = runs
        .iter()
        .map(|r| r.font.line)
        .fold(container.line_height.px(container.font_size), f32::max);
    Some(Paragraph {
        runs,
        align: match container.text_align {
            st::TextAlign::Center => Align::Center,
            st::TextAlign::Right | st::TextAlign::End => Align::Right,
            _ => Align::Left,
        },
        wrap: !matches!(container.white_space, WhiteSpace::NoWrap | WhiteSpace::Pre),
        line_height,
        ellipsis: container.text_overflow == st::TextOverflow::Ellipsis
            && container.overflow[0] != st::Overflow::Visible,
    })
}

/// The trailing collapsible space of a line goes.
fn trim_end(runs: &mut Vec<Run>) {
    while let Some(last) = runs.last_mut() {
        let trimmed = last.text.trim_end_matches(' ').len();
        last.text.truncate(trimmed);
        if last.text.is_empty() {
            runs.pop();
        } else {
            break;
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start = true;
    for c in s.chars() {
        if start && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            start = false;
        } else {
            out.push(c);
            if c.is_whitespace() {
                start = true;
            }
        }
    }
    out
}

struct ChildIter<'a>(std::slice::Iter<'a, usize>);

impl Iterator for ChildIter<'_> {
    type Item = tf::NodeId;
    fn next(&mut self) -> Option<tf::NodeId> {
        self.0.next().map(|&c| tf::NodeId::from(c))
    }
}

impl TraversePartialTree for Tree<'_> {
    type ChildIter<'a>
        = ChildIter<'a>
    where
        Self: 'a;

    fn child_ids(&self, node: tf::NodeId) -> Self::ChildIter<'_> {
        ChildIter(self.boxes[usize::from(node)].children.iter())
    }

    fn child_count(&self, node: tf::NodeId) -> usize {
        self.boxes[usize::from(node)].children.len()
    }

    fn get_child_id(&self, node: tf::NodeId, index: usize) -> tf::NodeId {
        tf::NodeId::from(self.boxes[usize::from(node)].children[index])
    }
}

impl tf::TraverseTree for Tree<'_> {}

impl LayoutPartialTree for Tree<'_> {
    type CoreContainerStyle<'a>
        = &'a tf::Style
    where
        Self: 'a;
    type CustomIdent = String;

    fn get_core_container_style(&self, node: tf::NodeId) -> Self::CoreContainerStyle<'_> {
        &self.boxes[usize::from(node)].style
    }

    fn resolve_calc_value(&self, val: *const (), basis: f32) -> f32 {
        calc_value(val, basis)
    }

    fn set_unrounded_layout(&mut self, node: tf::NodeId, layout: &tf::Layout) {
        self.boxes[usize::from(node)].unrounded = *layout;
    }

    fn compute_child_layout(
        &mut self,
        node: tf::NodeId,
        inputs: tf::LayoutInput,
    ) -> tf::LayoutOutput {
        self.compute(node, inputs, None)
    }
}

impl tf::LayoutContainingBlock for Tree<'_> {
    type OofItemStyle<'a>
        = &'a tf::Style
    where
        Self: 'a;

    fn get_oof_item_style(&self, node: tf::NodeId) -> Self::OofItemStyle<'_> {
        &self.boxes[usize::from(node)].style
    }

    fn clear_hoisted_children(&mut self, node: tf::NodeId) {
        self.boxes[usize::from(node)].hoisted.clear();
    }

    fn add_hoisted_children(&mut self, node: tf::NodeId, hoisted: &[tf::NodeId]) {
        self.boxes[usize::from(node)]
            .hoisted
            .extend_from_slice(hoisted);
    }
}

impl CacheTree for Tree<'_> {
    fn cache_get(
        &mut self,
        node: tf::NodeId,
        inputs: &tf::LayoutInput,
    ) -> Option<tf::LayoutOutput> {
        self.boxes[usize::from(node)].cache.get(inputs)
    }

    fn cache_store(
        &mut self,
        node: tf::NodeId,
        inputs: &tf::LayoutInput,
        output: tf::LayoutOutput,
    ) {
        self.boxes[usize::from(node)].cache.store(inputs, output)
    }

    fn cache_clear(&mut self, node: tf::NodeId) {
        self.boxes[usize::from(node)].cache.clear();
    }
}

impl tf::LayoutBlockContainer for Tree<'_> {
    type BlockContainerStyle<'a>
        = &'a tf::Style
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a tf::Style
    where
        Self: 'a;

    fn get_block_container_style(&self, node: tf::NodeId) -> Self::BlockContainerStyle<'_> {
        &self.boxes[usize::from(node)].style
    }

    fn get_block_child_style(&self, child: tf::NodeId) -> Self::BlockItemStyle<'_> {
        &self.boxes[usize::from(child)].style
    }

    fn compute_block_child_layout(
        &mut self,
        node: tf::NodeId,
        inputs: tf::LayoutInput,
        block_ctx: Option<&mut tf::BlockContext<'_>>,
    ) -> tf::LayoutOutput {
        self.compute(node, inputs, block_ctx)
    }
}

impl tf::LayoutFlexboxContainer for Tree<'_> {
    type FlexboxContainerStyle<'a>
        = &'a tf::Style
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a tf::Style
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, node: tf::NodeId) -> Self::FlexboxContainerStyle<'_> {
        &self.boxes[usize::from(node)].style
    }

    fn get_flexbox_child_style(&self, child: tf::NodeId) -> Self::FlexboxItemStyle<'_> {
        &self.boxes[usize::from(child)].style
    }
}

impl tf::LayoutGridContainer for Tree<'_> {
    type GridContainerStyle<'a>
        = &'a tf::Style
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a tf::Style
    where
        Self: 'a;

    fn get_grid_container_style(&self, node: tf::NodeId) -> Self::GridContainerStyle<'_> {
        &self.boxes[usize::from(node)].style
    }

    fn get_grid_child_style(&self, child: tf::NodeId) -> Self::GridItemStyle<'_> {
        &self.boxes[usize::from(child)].style
    }
}

impl tf::RoundTree for Tree<'_> {
    fn get_unrounded_layout(&self, node: tf::NodeId) -> tf::Layout {
        self.boxes[usize::from(node)].unrounded
    }

    fn set_final_layout(&mut self, node: tf::NodeId, layout: &tf::Layout) {
        self.boxes[usize::from(node)].layout = *layout;
    }

    fn is_out_of_flow(&self, node: tf::NodeId) -> bool {
        self.boxes[usize::from(node)]
            .style
            .position
            .is_out_of_flow()
    }

    fn hoisted_child_count(&self, node: tf::NodeId) -> usize {
        self.boxes[usize::from(node)].hoisted.len()
    }

    fn get_hoisted_child_id(&self, node: tf::NodeId, index: usize) -> tf::NodeId {
        self.boxes[usize::from(node)].hoisted[index]
    }
}
