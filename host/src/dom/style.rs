//! The cascade (ADR-4): which declarations reach each element, in what
//! order, and the computed style they leave it with.
//!
//! Three origins, as on the web: the host's user-agent sheet (`UA_CSS`), the
//! app's sheets in the order added, and each element's inline `style`. Within
//! them, `!important` first, then specificity, then order. Inherited
//! properties start from the parent's values; custom properties (`--x`)
//! inherit too, and `var()` is put in before a value is read. Relative units
//! are resolved here, so layout sees pixels — and percentages, which only
//! layout can resolve.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use super::css::{self, Declaration, Env, Media, Selector, Sheet, Token};
use super::select::{Matcher, Places, State};
use super::values::{self, words, Color, Ctx, Length};
use super::{Document, Element, NodeMap, ROOT};

/// The most tokens one value may grow to as `var()`s are put in — a custom
/// property can name another twice, which names another twice, and so on.
const MAX_VALUE_TOKENS: usize = 4096;

/// How deep `var()` may refer through other variables.
const MAX_VAR_DEPTH: usize = 16;

/// The host's own stylesheet: what every element looks like before the app
/// says otherwise, in the system's light or dark theme. The colours are the
/// window's palette (`gui.rs`, `mod palette`), exposed as variables the app
/// may use as well: `--nimadir-text`, `--nimadir-accent`, …
pub const UA_CSS: &str = r#"
:root {
  --nimadir-bg: #ffffff; --nimadir-surface: #f1f3f4; --nimadir-border: #dadce0;
  --nimadir-text: #202124; --nimadir-muted: #5f6368; --nimadir-accent: #1a73e8;
  --nimadir-accent-text: #ffffff; --nimadir-accent-wash: #e8f0fe;
  --nimadir-danger: #d93025; --nimadir-field: #ffffff;
  color: var(--nimadir-text); font-size: 15px; line-height: 1.4;
}
@media (prefers-color-scheme: dark) {
  :root {
    --nimadir-bg: #202124; --nimadir-surface: #303134; --nimadir-border: #5f6368;
    --nimadir-text: #e8eaed; --nimadir-muted: #9aa0a6; --nimadir-accent: #8ab4f8;
    --nimadir-accent-text: #202124; --nimadir-accent-wash: #2a374f;
    --nimadir-danger: #f28b82; --nimadir-field: #202124;
  }
}
h1 { font-size: 2em; font-weight: 700; margin: 0.67em 0; line-height: 1.2; }
h2 { font-size: 1.5em; font-weight: 700; margin: 0.75em 0; line-height: 1.25; }
h3 { font-size: 1.25em; font-weight: 700; margin: 0.8em 0; }
h4 { font-size: 1.1em; font-weight: 700; margin: 0.9em 0; }
h5, h6 { font-size: 1em; font-weight: 700; margin: 1em 0; }
p, blockquote, figure, pre, dl { margin: 0 0 0.8em; }
blockquote { padding-left: 12px; border-left: 3px solid var(--nimadir-border); color: var(--nimadir-muted); }
ul, ol { margin: 0 0 0.8em; padding-left: 24px; }
ol { list-style-type: decimal; }
b, strong, th { font-weight: 700; }
i, em, cite { font-style: italic; }
u, ins { text-decoration: underline; }
s, del, strike { text-decoration: line-through; }
small { font-size: 0.85em; }
code, kbd, pre, samp { font-family: monospace; }
pre { white-space: pre; }
mark { background: #fde293; color: #202124; }
a { color: var(--nimadir-accent); text-decoration: underline; cursor: pointer; }
hr { border: none; border-top: 1px solid var(--nimadir-border); margin: 8px 0; }
button {
  padding: 8px 16px; border-radius: 8px; border: 1px solid var(--nimadir-border);
  background: var(--nimadir-surface); color: var(--nimadir-text);
  font-weight: 600; cursor: pointer; text-align: center;
}
button:hover { background: var(--nimadir-accent-wash); }
button:active { border-color: var(--nimadir-accent); }
input, textarea {
  padding: 8px 10px; border: 1px solid var(--nimadir-border); border-radius: 8px;
  background: var(--nimadir-field); color: var(--nimadir-text); cursor: text;
}
/* As in browsers: a search field set to 100% fits, padding and all. */
input[type=search] { box-sizing: border-box; }
input:focus, textarea:focus, select:focus { border-color: var(--nimadir-accent); }
select {
  padding: 8px 32px 8px 10px; border: 1px solid var(--nimadir-border); border-radius: 8px;
  background: var(--nimadir-field); color: var(--nimadir-text); cursor: pointer;
}
input[type=checkbox], input[type=radio] {
  width: 18px; height: 18px; padding: 0; margin: 0 4px 0 0; cursor: pointer;
  border: 2px solid var(--nimadir-muted); border-radius: 4px;
}
input[type=radio] { border-radius: 50%; }
input[type=checkbox]:checked, input[type=radio]:checked {
  background: var(--nimadir-accent); border-color: var(--nimadir-accent);
}
input[type=range] { padding: 0; border: none; height: 20px; background: transparent; }
textarea { min-height: 64px; }
progress { width: 160px; height: 8px; border-radius: 4px; background: var(--nimadir-surface); color: var(--nimadir-accent); }
table { border-spacing: 0; }
td, th { padding: 4px 8px; }
/* `hidden` hides, whatever display a sheet gives the element: a browser lets
   `.card { display: flex }` undo it, which is a trap, not a feature. */
[hidden] { display: none !important; }
:disabled { opacity: 0.5; cursor: default; }
"#;

/// Declares a keyword property's values: its enum, `parse` from a CSS word,
/// and `css` back.
macro_rules! keywords {
    ($(#[$m:meta])* $name:ident { $($variant:ident = $css:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub fn parse(word: &str) -> Option<Self> {
                match word { $($css => Some(Self::$variant),)+ _ => None }
            }
            pub fn css(self) -> &'static str {
                match self { $(Self::$variant => $css),+ }
            }
        }
    };
}

keywords!(Display {
    None = "none", Block = "block", Inline = "inline", InlineBlock = "inline-block",
    Flex = "flex", InlineFlex = "inline-flex", Grid = "grid", InlineGrid = "inline-grid",
    ListItem = "list-item", Contents = "contents",
});
keywords!(Position {
    Static = "static", Relative = "relative", Absolute = "absolute", Fixed = "fixed",
    Sticky = "sticky",
});
keywords!(Overflow { Visible = "visible", Hidden = "hidden", Clip = "clip", Scroll = "scroll", Auto = "auto" });
keywords!(FlexDirection {
    Row = "row", RowReverse = "row-reverse", Column = "column", ColumnReverse = "column-reverse",
});
keywords!(FlexWrap { NoWrap = "nowrap", Wrap = "wrap", WrapReverse = "wrap-reverse" });
keywords!(
    /// One spelling for every alignment property; each reads the ones that
    /// mean something to it.
    Align {
        Normal = "normal", Auto = "auto", Start = "start", End = "end",
        FlexStart = "flex-start", FlexEnd = "flex-end", SelfStart = "self-start",
        SelfEnd = "self-end", Left = "left", Right = "right", Center = "center",
        Stretch = "stretch", Baseline = "baseline", SpaceBetween = "space-between",
        SpaceAround = "space-around", SpaceEvenly = "space-evenly",
    }
);
keywords!(BoxSizing { ContentBox = "content-box", BorderBox = "border-box" });
keywords!(TextAlign {
    Start = "start", End = "end", Left = "left", Right = "right", Center = "center",
    Justify = "justify",
});
keywords!(WhiteSpace {
    Normal = "normal", NoWrap = "nowrap", Pre = "pre", PreWrap = "pre-wrap", PreLine = "pre-line",
});
keywords!(TextTransform {
    None = "none", Uppercase = "uppercase", Lowercase = "lowercase", Capitalize = "capitalize",
});
keywords!(Float { None = "none", Left = "left", Right = "right" });
keywords!(Clear { None = "none", Left = "left", Right = "right", Both = "both" });
keywords!(BorderStyle {
    None = "none", Hidden = "hidden", Solid = "solid", Dashed = "dashed", Dotted = "dotted",
    Double = "double", Groove = "groove", Ridge = "ridge", Inset = "inset", Outset = "outset",
});
keywords!(Cursor {
    Auto = "auto", Default = "default", Pointer = "pointer", Text = "text",
    NotAllowed = "not-allowed", Grab = "grab", Grabbing = "grabbing", Move = "move",
    Wait = "wait", Progress = "progress", Help = "help", Crosshair = "crosshair",
    None = "none",
});
keywords!(ListStyle { Disc = "disc", Circle = "circle", Square = "square", Decimal = "decimal", None = "none" });
keywords!(Visibility { Visible = "visible", Hidden = "hidden", Collapse = "collapse" });
keywords!(TextOverflow { Clip = "clip", Ellipsis = "ellipsis" });
keywords!(PointerEvents { Auto = "auto", None = "none" });
keywords!(ObjectFit { Fill = "fill", Contain = "contain", Cover = "cover", None = "none", ScaleDown = "scale-down" });

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    Normal,
    /// A multiple of the font size, inherited as the multiple.
    Factor(f32),
    Px(f32),
}

impl LineHeight {
    pub fn px(self, font_size: f32) -> f32 {
        match self {
            LineHeight::Normal => font_size * 1.25,
            LineHeight::Factor(f) => font_size * f,
            LineHeight::Px(p) => p,
        }
    }
}

/// One grid track's size.
#[derive(Clone, Debug, PartialEq)]
pub enum Track {
    Fixed(Length),
    Fr(f32),
    Auto,
    MinContent,
    MaxContent,
    MinMax(Box<Track>, Box<Track>),
    FitContent(Length),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Repeat {
    Count(u16),
    AutoFill,
    AutoFit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TrackEntry {
    One(Track),
    Repeat(Repeat, Vec<Track>),
}

/// One end of a grid item's placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Line {
    Auto,
    Index(i16),
    Span(u16),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub start: Line,
    pub end: Line,
}

impl Placement {
    pub const AUTO: Placement = Placement {
        start: Line::Auto,
        end: Line::Auto,
    };
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    /// CSS's angle: 0 points up, 90 right.
    pub angle: f32,
    /// Each colour and where it sits, 0–1, when the sheet said.
    pub stops: Vec<(Color, Option<f32>)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
    /// `None` is `currentcolor`, until `finish` puts it in.
    pub color: Option<Color>,
    pub inset: bool,
}

pub const UNDERLINE: u8 = 1;
pub const LINE_THROUGH: u8 = 2;
pub const OVERLINE: u8 = 4;

pub type Vars = Arc<HashMap<String, Arc<Vec<Token>>>>;

/// An element's computed style.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    // The box — not inherited.
    pub display: Display,
    pub position: Position,
    /// top, right, bottom, left.
    pub inset: [Length; 4],
    pub width: Length,
    pub height: Length,
    /// `auto` is 0.
    pub min_width: Length,
    pub min_height: Length,
    /// `auto` is `none`.
    pub max_width: Length,
    pub max_height: Length,
    pub margin: [Length; 4],
    pub padding: [Length; 4],
    pub border_width: [f32; 4],
    pub border_style: [BorderStyle; 4],
    /// `None` is `currentcolor`, until `finish` puts it in.
    pub border_color: [Option<Color>; 4],
    /// top-left, top-right, bottom-right, bottom-left; a percentage is of
    /// the box's shorter side.
    pub radius: [Length; 4],
    pub box_sizing: BoxSizing,
    /// x, then y.
    pub overflow: [Overflow; 2],
    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub justify_content: Align,
    pub align_items: Align,
    pub align_self: Align,
    pub align_content: Align,
    pub justify_items: Align,
    pub justify_self: Align,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Length,
    pub order: i32,
    /// row, then column.
    pub gap: [Length; 2],
    pub grid_template_columns: Vec<TrackEntry>,
    pub grid_template_rows: Vec<TrackEntry>,
    pub grid_auto_columns: Vec<Track>,
    pub grid_auto_rows: Vec<Track>,
    pub grid_auto_flow_column: bool,
    pub grid_auto_flow_dense: bool,
    pub grid_column: Placement,
    pub grid_row: Placement,
    pub aspect_ratio: Option<f32>,
    pub float: Float,
    pub clear: Clear,
    pub z_index: Option<i32>,
    pub background: Color,
    pub gradient: Option<Gradient>,
    pub opacity: f32,
    pub shadow: Option<Shadow>,
    pub text_overflow: TextOverflow,
    pub object_fit: ObjectFit,
    // Text — inherited.
    pub color: Color,
    pub font_size: f32,
    pub font_weight: u16,
    pub italic: bool,
    pub monospace: bool,
    pub line_height: LineHeight,
    pub text_align: TextAlign,
    pub white_space: WhiteSpace,
    pub text_transform: TextTransform,
    pub letter_spacing: f32,
    /// `UNDERLINE` | `LINE_THROUGH` | `OVERLINE`. Not inherited in CSS, but
    /// drawn through every descendant's text, which comes to the same.
    pub text_decoration: u8,
    pub visibility: Visibility,
    pub cursor: Cursor,
    pub list_style: ListStyle,
    pub pointer_events: PointerEvents,
    pub vars: Vars,
}

impl Style {
    /// Every property at its initial value.
    pub fn initial() -> Style {
        Style {
            display: Display::Inline,
            position: Position::Static,
            inset: [Length::Auto; 4],
            width: Length::Auto,
            height: Length::Auto,
            min_width: Length::Auto,
            min_height: Length::Auto,
            max_width: Length::Auto,
            max_height: Length::Auto,
            margin: [Length::ZERO; 4],
            padding: [Length::ZERO; 4],
            border_width: [0.0; 4],
            border_style: [BorderStyle::None; 4],
            border_color: [None; 4],
            radius: [Length::ZERO; 4],
            box_sizing: BoxSizing::ContentBox,
            overflow: [Overflow::Visible; 2],
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::NoWrap,
            justify_content: Align::Normal,
            align_items: Align::Normal,
            align_self: Align::Auto,
            align_content: Align::Normal,
            justify_items: Align::Normal,
            justify_self: Align::Auto,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Length::Auto,
            order: 0,
            gap: [Length::ZERO; 2],
            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            grid_auto_columns: Vec::new(),
            grid_auto_rows: Vec::new(),
            grid_auto_flow_column: false,
            grid_auto_flow_dense: false,
            grid_column: Placement::AUTO,
            grid_row: Placement::AUTO,
            aspect_ratio: None,
            float: Float::None,
            clear: Clear::None,
            z_index: None,
            background: Color::TRANSPARENT,
            gradient: None,
            opacity: 1.0,
            shadow: None,
            text_overflow: TextOverflow::Clip,
            object_fit: ObjectFit::Fill,
            color: Color::BLACK,
            font_size: 16.0,
            font_weight: 400,
            italic: false,
            monospace: false,
            line_height: LineHeight::Normal,
            text_align: TextAlign::Start,
            white_space: WhiteSpace::Normal,
            text_transform: TextTransform::None,
            letter_spacing: 0.0,
            text_decoration: 0,
            visibility: Visibility::Visible,
            cursor: Cursor::Auto,
            list_style: ListStyle::Disc,
            pointer_events: PointerEvents::Auto,
            vars: Arc::new(HashMap::new()),
        }
    }

    /// A child of `parent`: inherited properties from it, the rest initial,
    /// and the display its tag has before any sheet says otherwise.
    pub fn child_of(parent: &Style, tag: &str) -> Style {
        let mut s = Style::initial();
        s.copy_inherited(parent);
        s.display = default_display(tag);
        s
    }

    fn copy_inherited(&mut self, p: &Style) {
        self.color = p.color;
        self.font_size = p.font_size;
        self.font_weight = p.font_weight;
        self.italic = p.italic;
        self.monospace = p.monospace;
        self.line_height = p.line_height;
        self.text_align = p.text_align;
        self.white_space = p.white_space;
        self.text_transform = p.text_transform;
        self.letter_spacing = p.letter_spacing;
        self.text_decoration = p.text_decoration;
        self.visibility = p.visibility;
        self.cursor = p.cursor;
        self.list_style = p.list_style;
        self.pointer_events = p.pointer_events;
        self.vars = p.vars.clone();
    }

    /// Whether `other` lays out the same — differing, if at all, only in
    /// what is drawn: colours, backgrounds, shadows, opacity, visibility,
    /// the cursor. A pointer moving over a page changes those, and the
    /// page need not be laid out again for it.
    pub fn same_layout(&self, other: &Style) -> bool {
        let mut a = self.clone();
        a.color = other.color;
        a.background = other.background;
        a.gradient.clone_from(&other.gradient);
        a.border_color = other.border_color;
        a.border_style = other.border_style;
        a.shadow = other.shadow;
        a.opacity = other.opacity;
        a.visibility = other.visibility;
        a.cursor = other.cursor;
        a.pointer_events = other.pointer_events;
        a.text_decoration = other.text_decoration;
        a.list_style = other.list_style;
        a.object_fit = other.object_fit;
        a.vars = other.vars.clone();
        a == *other
    }

    /// Whether the element is laid out as a block-level box, as opposed to
    /// flowing as text.
    pub fn is_inline(&self) -> bool {
        self.display == Display::Inline
    }
}

/// What a tag is before any stylesheet: text-level tags flow inline, a tag
/// with no meaning here is a block — a `div` — and the rest are blocks too.
pub fn default_display(tag: &str) -> Display {
    match tag {
        "span" | "a" | "b" | "strong" | "i" | "em" | "u" | "s" | "del" | "ins" | "strike"
        | "code" | "kbd" | "samp" | "small" | "sub" | "sup" | "mark" | "abbr" | "cite" | "q"
        | "time" | "var" | "label" | "br" => Display::Inline,
        "button" | "input" | "textarea" | "select" | "img" | "progress" => Display::InlineBlock,
        "li" => Display::ListItem,
        "head" | "script" | "style" | "template" | "title" | "meta" | "link" | "option"
        | "optgroup" => Display::None,
        _ => Display::Block,
    }
}

/// One rule's selector, ready to match, with what it declares.
struct Entry {
    selector: Selector,
    declarations: Arc<Vec<Declaration>>,
    media: Arc<Vec<Media>>,
    origin: u8,
    order: u32,
}

/// Where a declaration stands in the cascade: important, origin,
/// specificity, order — sorted, the last wins.
type Key = (u8, u8, u32, u32);

const ORIGIN_UA: u8 = 0;
const ORIGIN_AUTHOR: u8 = 1;
const ORIGIN_INLINE: u8 = 2;

/// Every rule, filed under the id, class or tag its selector must match on
/// the element itself, so an element only tries the rules that could apply.
#[derive(Default)]
struct Index {
    entries: Vec<Entry>,
    by_id: HashMap<String, Vec<usize>>,
    by_class: HashMap<String, Vec<usize>>,
    by_tag: HashMap<String, Vec<usize>>,
    any: Vec<usize>,
}

impl Index {
    fn add(&mut self, sheet: &Sheet, origin: u8) {
        for rule in &sheet.rules {
            let declarations = Arc::new(rule.declarations.clone());
            let media = Arc::new(rule.media.clone());
            for selector in &rule.selectors {
                let at = self.entries.len();
                let subject = &selector.compounds[0];
                if let Some(id) = &subject.id {
                    self.by_id.entry(id.clone()).or_default().push(at);
                } else if let Some(class) = subject.classes.first() {
                    self.by_class.entry(class.clone()).or_default().push(at);
                } else if let Some(tag) = &subject.tag {
                    self.by_tag.entry(tag.clone()).or_default().push(at);
                } else {
                    self.any.push(at);
                }
                self.entries.push(Entry {
                    selector: selector.clone(),
                    declarations: declarations.clone(),
                    media: media.clone(),
                    origin,
                    order: at as u32,
                });
            }
        }
    }

    fn candidates<'a>(&'a self, e: &'a Element) -> impl Iterator<Item = &'a Entry> + 'a {
        let id = e.attr("id").and_then(|i| self.by_id.get(i));
        let classes = e.classes.iter().filter_map(|c| self.by_class.get(c));
        let tag = self.by_tag.get(&e.tag);
        id.into_iter()
            .chain(classes)
            .chain(tag)
            .chain(std::iter::once(&self.any))
            .flatten()
            .map(|&i| &self.entries[i])
    }
}

/// The user-agent sheet, read once.
pub fn ua_sheet() -> &'static Sheet {
    static SHEET: OnceLock<Sheet> = OnceLock::new();
    SHEET.get_or_init(|| css::parse(UA_CSS))
}

/// Every element in the tree, styled.
pub fn compute(doc: &Document, env: &Env, state: State) -> NodeMap<Style> {
    let mut index = Index::default();
    index.add(ua_sheet(), ORIGIN_UA);
    for sheet in doc.parsed_sheets() {
        index.add(sheet, ORIGIN_AUTHOR);
    }
    let places = Places::of(doc);
    let matcher = Matcher::new(doc, state, &places);
    let root_initial = Style::initial();

    let mut styles: NodeMap<Style> =
        NodeMap::with_capacity_and_hasher(doc.len(), Default::default());
    let mut rem = 16.0;
    for (id, _) in doc.walk() {
        let node = &doc.nodes[&id];
        let Some(e) = node.element() else {
            continue;
        };
        let parent = node
            .parent
            .and_then(|p| styles.get(&p))
            .unwrap_or(&root_initial);
        let mut style = Style::child_of(parent, &e.tag);

        // What applies, in cascade order.
        let mut found: Vec<(Key, &Declaration)> = Vec::new();
        for entry in index.candidates(e) {
            if !entry.media.iter().all(|m| m.matches(env)) || !matcher.matches(&entry.selector, id)
            {
                continue;
            }
            for d in entry.declarations.iter() {
                found.push((
                    (
                        d.important as u8,
                        entry.origin,
                        entry.selector.specificity,
                        entry.order,
                    ),
                    d,
                ));
            }
        }
        for d in e.inline.iter() {
            found.push(((d.important as u8, ORIGIN_INLINE, u32::MAX, 0), d));
        }
        found.sort_by_key(|(key, _)| *key);

        // Custom properties first: every other value may refer to them.
        let mut vars: Option<HashMap<String, Arc<Vec<Token>>>> = None;
        for (_, d) in found.iter().filter(|(_, d)| d.name.starts_with("--")) {
            let map = vars.get_or_insert_with(|| (*style.vars).clone());
            let resolved = substitute(&d.value, map, 0).unwrap_or_default();
            map.insert(d.name.clone(), Arc::new(resolved));
        }
        if let Some(map) = vars {
            style.vars = Arc::new(map);
        }

        let mut cx = Ctx {
            em: parent.font_size,
            rem,
            vw: env.width,
            vh: env.height,
        };
        // Then what other values are relative to: the font size, for `em`,
        // and the colour, for `currentcolor`.
        let first = |n: &str| matches!(n, "font-size" | "font" | "color");
        for (_, d) in found.iter().filter(|(_, d)| first(&d.name)) {
            if let Some(value) = resolve(&d.value, &style.vars) {
                apply(&mut style, parent, &d.name, &value, &cx);
            }
        }
        cx.em = style.font_size;
        if id == ROOT {
            rem = style.font_size;
            cx.rem = rem;
        }
        for (_, d) in found
            .iter()
            .filter(|(_, d)| !first(&d.name) && !d.name.starts_with("--"))
        {
            if let Some(value) = resolve(&d.value, &style.vars) {
                apply(&mut style, parent, &d.name, &value, &cx);
            }
        }
        finish(&mut style);
        styles.insert(id, style);
    }
    styles
}

/// `value` with every `var(--x, fallback)` replaced; `None` if one names a
/// variable that is not set and gives no fallback, or the value grows past
/// `MAX_VALUE_TOKENS`.
/// `value` as it is when it names no variable — the common case, and no
/// copy — else with its `var()`s put in.
fn resolve<'a>(
    value: &'a [Token],
    vars: &HashMap<String, Arc<Vec<Token>>>,
) -> Option<Cow<'a, [Token]>> {
    if has_var(value) {
        substitute(value, vars, 0).map(Cow::Owned)
    } else {
        Some(Cow::Borrowed(value))
    }
}

fn has_var(value: &[Token]) -> bool {
    value
        .iter()
        .any(|t| matches!(t, Token::Function(f) if f == "var"))
}

fn substitute(
    value: &[Token],
    vars: &HashMap<String, Arc<Vec<Token>>>,
    depth: usize,
) -> Option<Vec<Token>> {
    if !has_var(value) {
        return Some(value.to_vec());
    }
    if depth > MAX_VAR_DEPTH {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < value.len() {
        if let Token::Function(f) = &value[i] {
            if f == "var" {
                // Its arguments, to the matching bracket.
                let mut nest = 1usize;
                let mut j = i + 1;
                while j < value.len() {
                    match &value[j] {
                        Token::Function(_) | Token::LParen => nest += 1,
                        Token::RParen => {
                            nest -= 1;
                            if nest == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                let args = &value[i + 1..j.min(value.len())];
                let parts = css::split_top(args, &Token::Comma);
                let name = css::trim(&parts[0]);
                let fallback = (parts.len() > 1).then(|| {
                    let at = args
                        .iter()
                        .position(|t| *t == Token::Comma)
                        .unwrap_or(args.len());
                    css::trim(&args[(at + 1).min(args.len())..]).to_vec()
                });
                let found = match name {
                    [Token::Ident(n)] => vars.get(n).map(|v| v.to_vec()),
                    _ => None,
                };
                let chosen = found.or(fallback)?;
                out.extend(substitute(&chosen, vars, depth + 1)?);
                if out.len() > MAX_VALUE_TOKENS {
                    return None;
                }
                i = j + 1;
                continue;
            }
        }
        out.push(value[i].clone());
        if out.len() > MAX_VALUE_TOKENS {
            return None;
        }
        i += 1;
    }
    Some(out)
}

/// What is left once every declaration is in: `currentcolor` put in.
fn finish(s: &mut Style) {
    for c in &mut s.border_color {
        c.get_or_insert(s.color);
    }
    if let Some(sh) = &mut s.shadow {
        sh.color.get_or_insert(s.color);
    }
    for (w, st) in s.border_width.iter_mut().zip(s.border_style) {
        if matches!(st, BorderStyle::None | BorderStyle::Hidden) {
            *w = 0.0;
        }
    }
}

/// The 1–4 values of `margin`, `padding` and their kind, as top, right,
/// bottom, left.
fn sides<T: Copy>(values: &[T]) -> Option<[T; 4]> {
    Some(match *values {
        [a] => [a, a, a, a],
        [a, b] => [a, b, a, b],
        [a, b, c] => [a, b, c, b],
        [a, b, c, d] => [a, b, c, d],
        _ => return None,
    })
}

fn all<T>(parts: &[&[Token]], f: impl Fn(&[Token]) -> Option<T>) -> Option<Vec<T>> {
    parts.iter().map(|p| f(p)).collect()
}

const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];

/// Apply one declaration. Unknown properties and unreadable values change
/// nothing, as in a browser.
fn apply(s: &mut Style, parent: &Style, name: &str, value: &[Token], cx: &Ctx) {
    let w = words(value);
    let kw = values::keyword(value);
    // The global keywords, for every property.
    if let Some(k) = kw.as_deref() {
        let from = match k {
            "inherit" => Some(parent.clone()),
            "initial" => Some(Style::initial()),
            "unset" | "revert" | "revert-layer" => Some(if is_inherited(name) {
                parent.clone()
            } else {
                Style::initial()
            }),
            _ => None,
        };
        if let Some(from) = from {
            copy_property(s, &from, name);
            return;
        }
    }
    let len = |t: &[Token]| values::length(t, cx);
    let lp = |t: &[Token]| values::length_percent(t, cx);
    let px = |t: &[Token]| values::px(t, cx);
    let current = s.color;
    let color = |t: &[Token]| values::color(t, current);

    match name {
        "display" => {
            let d = match kw.as_deref() {
                Some("flow-root")
                | Some("table")
                | Some("table-row-group")
                | Some("table-header-group")
                | Some("table-footer-group")
                | Some("table-caption") => Some(Display::Block),
                Some("table-row") => Some(Display::Flex),
                Some("table-cell") => Some(Display::Block),
                Some("inline-table") => Some(Display::InlineBlock),
                Some(k) => Display::parse(k),
                None => match w.as_slice() {
                    // `block flex` and the like: the inner display decides.
                    [outer, inner] => {
                        let inner = values::keyword(inner).and_then(|k| Display::parse(&k));
                        let inline = values::keyword(outer).as_deref() == Some("inline");
                        match (inline, inner) {
                            (false, Some(d)) => Some(d),
                            (true, Some(Display::Flex)) => Some(Display::InlineFlex),
                            (true, Some(Display::Grid)) => Some(Display::InlineGrid),
                            (true, Some(Display::Block)) => Some(Display::InlineBlock),
                            _ => None,
                        }
                    }
                    _ => None,
                },
            };
            if let Some(d) = d {
                s.display = d;
            }
        }
        "position" => set(&mut s.position, kw.as_deref().and_then(Position::parse)),
        "top" | "right" | "bottom" | "left" => {
            let i = SIDES.iter().position(|x| *x == name).unwrap_or(0);
            set(&mut s.inset[i], len(value));
        }
        "inset" => set_all(&mut s.inset, all(&w, len).and_then(|v| sides(&v))),
        "width" => set(&mut s.width, len(value)),
        "height" => set(&mut s.height, len(value)),
        "min-width" => set(&mut s.min_width, len(value)),
        "min-height" => set(&mut s.min_height, len(value)),
        "max-width" => set(&mut s.max_width, none_or(value, len)),
        "max-height" => set(&mut s.max_height, none_or(value, len)),
        "margin" => set_all(&mut s.margin, all(&w, len).and_then(|v| sides(&v))),
        "padding" => set_all(&mut s.padding, all(&w, lp).and_then(|v| sides(&v))),
        "margin-block" | "margin-inline" | "padding-block" | "padding-inline" => {
            let v = if name.starts_with("margin") {
                all(&w, len)
            } else {
                all(&w, lp)
            };
            let Some(v) = v else { return };
            let (a, b) = match v.as_slice() {
                [a] => (*a, *a),
                [a, b] => (*a, *b),
                _ => return,
            };
            let target = if name.starts_with("margin") {
                &mut s.margin
            } else {
                &mut s.padding
            };
            if name.ends_with("block") {
                target[0] = a;
                target[2] = b;
            } else {
                target[3] = a;
                target[1] = b;
            }
        }
        "border" => {
            if let Some((width, style, col)) = border_parts(&w, cx, current) {
                s.border_width = [width; 4];
                s.border_style = [style; 4];
                s.border_color = [col; 4];
            }
        }
        "border-width" => set_all(
            &mut s.border_width,
            all(&w, |t| border_width(t, cx)).and_then(|v| sides(&v)),
        ),
        "border-style" => set_all(
            &mut s.border_style,
            all(&w, |t| {
                values::keyword(t).and_then(|k| BorderStyle::parse(&k))
            })
            .and_then(|v| sides(&v)),
        ),
        "border-color" => {
            if let Some(v) = all(&w, color).and_then(|v| sides(&v)) {
                s.border_color = v.map(Some);
            }
        }
        "border-radius" => {
            // Elliptical corners (`a / b`) keep their first radius.
            let first: Vec<&[Token]> = w
                .iter()
                .take_while(|t| **t != [Token::Delim('/')])
                .copied()
                .collect();
            set_all(&mut s.radius, all(&first, lp).and_then(|v| sides(&v)));
        }
        "border-top-left-radius" => set(&mut s.radius[0], w.first().and_then(|t| lp(t))),
        "border-top-right-radius" => set(&mut s.radius[1], w.first().and_then(|t| lp(t))),
        "border-bottom-right-radius" => set(&mut s.radius[2], w.first().and_then(|t| lp(t))),
        "border-bottom-left-radius" => set(&mut s.radius[3], w.first().and_then(|t| lp(t))),
        "box-sizing" => set(&mut s.box_sizing, kw.as_deref().and_then(BoxSizing::parse)),
        "overflow" => {
            let v = all(&w, |t| values::keyword(t).and_then(|k| Overflow::parse(&k)));
            match v.as_deref() {
                Some([a]) => s.overflow = [*a, *a],
                Some([a, b]) => s.overflow = [*a, *b],
                _ => {}
            }
        }
        "overflow-x" => set(&mut s.overflow[0], kw.as_deref().and_then(Overflow::parse)),
        "overflow-y" => set(&mut s.overflow[1], kw.as_deref().and_then(Overflow::parse)),
        "flex-direction" => set(
            &mut s.flex_direction,
            kw.as_deref().and_then(FlexDirection::parse),
        ),
        "flex-wrap" => set(&mut s.flex_wrap, kw.as_deref().and_then(FlexWrap::parse)),
        "flex-flow" => {
            for t in &w {
                if let Some(k) = values::keyword(t) {
                    if let Some(d) = FlexDirection::parse(&k) {
                        s.flex_direction = d;
                    } else if let Some(f) = FlexWrap::parse(&k) {
                        s.flex_wrap = f;
                    }
                }
            }
        }
        "justify-content" => set(&mut s.justify_content, align(&w)),
        "align-items" => set(&mut s.align_items, align(&w)),
        "align-self" => set(&mut s.align_self, align(&w)),
        "align-content" => set(&mut s.align_content, align(&w)),
        "justify-items" => set(&mut s.justify_items, align(&w)),
        "justify-self" => set(&mut s.justify_self, align(&w)),
        "place-items" | "place-content" | "place-self" => {
            let (a, b) = match w.as_slice() {
                [a] => (align(&[a]), align(&[a])),
                [a, b] => (align(&[a]), align(&[b])),
                _ => return,
            };
            let (Some(a), Some(b)) = (a, b) else { return };
            match name {
                "place-items" => (s.align_items, s.justify_items) = (a, b),
                "place-content" => (s.align_content, s.justify_content) = (a, b),
                _ => (s.align_self, s.justify_self) = (a, b),
            }
        }
        "flex" => {
            if let Some((g, sh, b)) = flex(&w, cx) {
                s.flex_grow = g;
                s.flex_shrink = sh;
                s.flex_basis = b;
            }
        }
        "flex-grow" => set(
            &mut s.flex_grow,
            values::number(value).filter(|n| *n >= 0.0),
        ),
        "flex-shrink" => set(
            &mut s.flex_shrink,
            values::number(value).filter(|n| *n >= 0.0),
        ),
        "flex-basis" => set(
            &mut s.flex_basis,
            len(value).or_else(|| (kw.as_deref() == Some("content")).then_some(Length::Auto)),
        ),
        "order" => set(&mut s.order, integer(value)),
        "gap" | "grid-gap" => {
            let v = all(&w, |t| normal_or(t, lp));
            match v.as_deref() {
                Some([a]) => s.gap = [*a, *a],
                Some([a, b]) => s.gap = [*a, *b],
                _ => {}
            }
        }
        "row-gap" | "grid-row-gap" => set(&mut s.gap[0], normal_or(value, lp)),
        "column-gap" | "grid-column-gap" => set(&mut s.gap[1], normal_or(value, lp)),
        "grid-template-columns" => set(&mut s.grid_template_columns, track_list(&w, cx)),
        "grid-template-rows" => set(&mut s.grid_template_rows, track_list(&w, cx)),
        "grid-auto-columns" => set(&mut s.grid_auto_columns, all(&w, |t| track(t, cx))),
        "grid-auto-rows" => set(&mut s.grid_auto_rows, all(&w, |t| track(t, cx))),
        "grid-auto-flow" => {
            let ks: Vec<String> = w.iter().filter_map(|t| values::keyword(t)).collect();
            if ks.len() == w.len() && !ks.is_empty() {
                s.grid_auto_flow_column = ks.iter().any(|k| k == "column");
                s.grid_auto_flow_dense = ks.iter().any(|k| k == "dense");
            }
        }
        "grid-column" => set(&mut s.grid_column, placement(value)),
        "grid-row" => set(&mut s.grid_row, placement(value)),
        "grid-column-start" => set(&mut s.grid_column.start, line(value)),
        "grid-column-end" => set(&mut s.grid_column.end, line(value)),
        "grid-row-start" => set(&mut s.grid_row.start, line(value)),
        "grid-row-end" => set(&mut s.grid_row.end, line(value)),
        "grid-area" => {
            // `row-start / column-start / row-end / column-end`, by number.
            let parts: Vec<Option<Line>> = css::split_top(value, &Token::Delim('/'))
                .iter()
                .map(|p| line(css::trim(p)))
                .collect();
            if parts.iter().all(Option::is_some) {
                let p: Vec<Line> = parts.into_iter().flatten().collect();
                let get = |i: usize| p.get(i).copied().unwrap_or(Line::Auto);
                s.grid_row = Placement {
                    start: get(0),
                    end: get(2),
                };
                s.grid_column = Placement {
                    start: get(1),
                    end: get(3),
                };
            }
        }
        "aspect-ratio" => {
            let parts = css::split_top(value, &Token::Delim('/'));
            let r = match parts.as_slice() {
                [a] if values::keyword(css::trim(a)).as_deref() == Some("auto") => None,
                [a] => values::number(css::trim(a)),
                [a, b] => values::number(css::trim(a))
                    .zip(values::number(css::trim(b)))
                    .filter(|(_, b)| *b > 0.0)
                    .map(|(a, b)| a / b),
                _ => return,
            };
            s.aspect_ratio = r.filter(|r| *r > 0.0 && r.is_finite());
        }
        "float" => set(&mut s.float, kw.as_deref().and_then(Float::parse)),
        "clear" => set(&mut s.clear, kw.as_deref().and_then(Clear::parse)),
        "z-index" => {
            if kw.as_deref() == Some("auto") {
                s.z_index = None;
            } else if let Some(z) = integer(value) {
                s.z_index = Some(z);
            }
        }
        "background-color" => set(&mut s.background, color(value)),
        "background" | "background-image" => {
            if kw.as_deref() == Some("none") {
                s.gradient = None;
                if name == "background" {
                    s.background = Color::TRANSPARENT;
                }
                return;
            }
            // A colour and a gradient, wherever they sit among the other
            // background values, which this host does not draw.
            let mut col = None;
            let mut grad = None;
            for t in &w {
                if let Some(c) = color(t) {
                    col = Some(c);
                } else if let Some(g) = gradient(t, current) {
                    grad = Some(g);
                }
            }
            if name == "background" {
                if col.is_none() && grad.is_none() {
                    return;
                }
                s.background = col.unwrap_or(Color::TRANSPARENT);
            }
            s.gradient = grad;
        }
        "opacity" => {
            let o = match value {
                [Token::Percentage(p)] => Some(p / 100.0),
                _ => values::number(value),
            };
            set(&mut s.opacity, o.map(|o| o.clamp(0.0, 1.0)));
        }
        "box-shadow" => {
            if kw.as_deref() == Some("none") {
                s.shadow = None;
            } else {
                // The first shadow of a list; the rest are not drawn.
                let first = css::split_top(value, &Token::Comma)
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                if let Some(sh) = shadow(&words(css::trim(&first)), cx, current) {
                    s.shadow = Some(sh);
                }
            }
        }
        "text-overflow" => set(
            &mut s.text_overflow,
            kw.as_deref().and_then(TextOverflow::parse),
        ),
        "object-fit" => set(&mut s.object_fit, kw.as_deref().and_then(ObjectFit::parse)),
        "color" => set(&mut s.color, values::color(value, parent.color)),
        "font-size" => set(&mut s.font_size, font_size(value, parent.font_size, cx)),
        "font-weight" => set(&mut s.font_weight, font_weight(value, parent.font_weight)),
        "font-style" => {
            if let Some(k) = w.first().and_then(|t| values::keyword(t)) {
                match k.as_str() {
                    "italic" | "oblique" => s.italic = true,
                    "normal" => s.italic = false,
                    _ => {}
                }
            }
        }
        "font-family" => s.monospace = monospace(value),
        "font" => font(s, parent, &w, value, cx),
        "line-height" => set(&mut s.line_height, line_height(value, cx)),
        "text-align" => set(&mut s.text_align, kw.as_deref().and_then(TextAlign::parse)),
        "white-space" | "white-space-collapse" => {
            let ws = match kw.as_deref() {
                Some("break-spaces") => Some(WhiteSpace::PreWrap),
                Some("preserve") => Some(WhiteSpace::PreWrap),
                Some("collapse") => Some(WhiteSpace::Normal),
                Some(k) => WhiteSpace::parse(k),
                None => None,
            };
            set(&mut s.white_space, ws);
        }
        "text-wrap" | "text-wrap-mode" => match kw.as_deref() {
            Some("nowrap") => s.white_space = WhiteSpace::NoWrap,
            Some("wrap") if s.white_space == WhiteSpace::NoWrap => {
                s.white_space = WhiteSpace::Normal
            }
            _ => {}
        },
        "text-transform" => set(
            &mut s.text_transform,
            kw.as_deref().and_then(TextTransform::parse),
        ),
        "letter-spacing" => set(
            &mut s.letter_spacing,
            normal_or(value, |t| px(t).map(Length::Px)).map(|l| l.resolve(0.0).unwrap_or(0.0)),
        ),
        "text-decoration" | "text-decoration-line" => {
            let mut flags = 0u8;
            let mut understood = false;
            for t in &w {
                match values::keyword(t).as_deref() {
                    Some("none") => understood = true,
                    Some("underline") => {
                        flags |= UNDERLINE;
                        understood = true;
                    }
                    Some("line-through") => {
                        flags |= LINE_THROUGH;
                        understood = true;
                    }
                    Some("overline") => {
                        flags |= OVERLINE;
                        understood = true;
                    }
                    _ => {}
                }
            }
            if understood {
                s.text_decoration = flags;
            }
        }
        "visibility" => set(&mut s.visibility, kw.as_deref().and_then(Visibility::parse)),
        "cursor" => {
            // The last word of a fallback list is the keyword.
            let last = css::split_top(value, &Token::Comma)
                .last()
                .cloned()
                .unwrap_or_default();
            set(
                &mut s.cursor,
                values::keyword(css::trim(&last)).and_then(|k| Cursor::parse(&k)),
            );
        }
        "list-style" | "list-style-type" => {
            for t in &w {
                if let Some(l) = values::keyword(t).and_then(|k| ListStyle::parse(&k)) {
                    s.list_style = l;
                }
            }
        }
        "pointer-events" => set(
            &mut s.pointer_events,
            kw.as_deref().and_then(PointerEvents::parse),
        ),
        _ => {
            if let Some(rest) = name.strip_prefix("border-") {
                border_side(s, rest, &w, value, cx, current);
            } else if let Some(side) = name.strip_prefix("margin-") {
                if let Some(i) = side_index(side) {
                    set(&mut s.margin[i], len(value));
                }
            } else if let Some(side) = name.strip_prefix("padding-") {
                if let Some(i) = side_index(side) {
                    set(&mut s.padding[i], lp(value));
                }
            }
        }
    }
}

fn set<T>(field: &mut T, value: Option<T>) {
    if let Some(v) = value {
        *field = v;
    }
}

fn set_all<T>(field: &mut [T; 4], value: Option<[T; 4]>) {
    if let Some(v) = value {
        *field = v;
    }
}

/// `top`, `inline-start`, … as an index into top, right, bottom, left.
fn side_index(side: &str) -> Option<usize> {
    match side {
        "top" | "block-start" => Some(0),
        "right" | "inline-end" => Some(1),
        "bottom" | "block-end" => Some(2),
        "left" | "inline-start" => Some(3),
        _ => None,
    }
}

/// `border-top`, `border-left-color`, …
fn border_side(
    s: &mut Style,
    rest: &str,
    w: &[&[Token]],
    value: &[Token],
    cx: &Ctx,
    current: Color,
) {
    let (side, part) = match rest.split_once('-') {
        Some((side, part)) if side_index(side).is_some() => (side, Some(part)),
        _ => (rest, None),
    };
    let Some(i) = side_index(side) else {
        return;
    };
    match part {
        None => {
            if let Some((width, style, col)) = border_parts(w, cx, current) {
                s.border_width[i] = width;
                s.border_style[i] = style;
                s.border_color[i] = col;
            }
        }
        Some("width") => set(&mut s.border_width[i], border_width(value, cx)),
        Some("style") => set(
            &mut s.border_style[i],
            values::keyword(value).and_then(|k| BorderStyle::parse(&k)),
        ),
        Some("color") => {
            if let Some(c) = values::color(value, current) {
                s.border_color[i] = Some(c);
            }
        }
        _ => {}
    }
}

fn border_width(t: &[Token], cx: &Ctx) -> Option<f32> {
    match values::keyword(t).as_deref() {
        Some("thin") => Some(1.0),
        Some("medium") => Some(3.0),
        Some("thick") => Some(5.0),
        _ => values::px(t, cx).filter(|w| *w >= 0.0),
    }
}

/// `1px solid red` in any order; what is left out is `medium`, `none`,
/// `currentcolor`.
fn border_parts(
    w: &[&[Token]],
    cx: &Ctx,
    current: Color,
) -> Option<(f32, BorderStyle, Option<Color>)> {
    let mut width = None;
    let mut style = None;
    let mut col = None;
    for t in w {
        if let Some(st) = values::keyword(t).and_then(|k| BorderStyle::parse(&k)) {
            style = Some(st);
        } else if let Some(wd) = border_width(t, cx) {
            width = Some(wd);
        } else {
            col = Some(values::color(t, current)?);
        }
    }
    let style = style.unwrap_or(BorderStyle::None);
    Some((width.unwrap_or(3.0), style, col))
}

fn align(w: &[&[Token]]) -> Option<Align> {
    // `safe center` and `unsafe end` keep their alignment.
    let last = w.last()?;
    Align::parse(&values::keyword(last)?)
}

fn none_or(t: &[Token], f: impl Fn(&[Token]) -> Option<Length>) -> Option<Length> {
    if values::keyword(t).as_deref() == Some("none") {
        Some(Length::Auto)
    } else {
        f(t)
    }
}

fn normal_or(t: &[Token], f: impl Fn(&[Token]) -> Option<Length>) -> Option<Length> {
    if values::keyword(t).as_deref() == Some("normal") {
        Some(Length::ZERO)
    } else {
        f(t)
    }
}

fn integer(t: &[Token]) -> Option<i32> {
    values::number(t)
        .filter(|n| n.fract() == 0.0 && n.abs() < 1e9)
        .map(|n| n as i32)
}

/// `flex: none | auto | <grow> [<shrink>] [<basis>]`.
fn flex(w: &[&[Token]], cx: &Ctx) -> Option<(f32, f32, Length)> {
    match w {
        [one] if values::keyword(one).as_deref() == Some("none") => {
            return Some((0.0, 0.0, Length::Auto))
        }
        [one] if values::keyword(one).as_deref() == Some("auto") => {
            return Some((1.0, 1.0, Length::Auto))
        }
        _ => {}
    }
    let mut numbers = Vec::new();
    let mut basis = None;
    for t in w {
        if let Some(n) = values::number(t) {
            numbers.push(n);
        } else {
            basis = Some(values::length(t, cx)?);
        }
    }
    let grow = *numbers.first()?;
    let shrink = numbers.get(1).copied().unwrap_or(1.0);
    // `flex: 1` is `1 1 0`, not `1 1 auto`.
    Some((
        grow.max(0.0),
        shrink.max(0.0),
        basis.unwrap_or(Length::ZERO),
    ))
}

fn track(t: &[Token], cx: &Ctx) -> Option<Track> {
    if let Some(k) = values::keyword(t) {
        return match k.as_str() {
            "auto" => Some(Track::Auto),
            "min-content" => Some(Track::MinContent),
            "max-content" => Some(Track::MaxContent),
            _ => None,
        };
    }
    match t {
        [Token::Dimension(n, u)] if u == "fr" && *n >= 0.0 => Some(Track::Fr(*n)),
        [Token::Function(f), rest @ ..] if f == "minmax" => {
            let inner = &rest[..rest.len().checked_sub(1)?];
            let parts = css::split_top(inner, &Token::Comma);
            let [a, b] = parts.as_slice() else {
                return None;
            };
            Some(Track::MinMax(
                Box::new(track(css::trim(a), cx)?),
                Box::new(track(css::trim(b), cx)?),
            ))
        }
        [Token::Function(f), rest @ ..] if f == "fit-content" => {
            let inner = &rest[..rest.len().checked_sub(1)?];
            Some(Track::FitContent(values::length_percent(
                css::trim(inner),
                cx,
            )?))
        }
        _ => values::length_percent(t, cx).map(Track::Fixed),
    }
}

/// `grid-template-columns`: `none`, or tracks and `repeat()`s. Line names in
/// brackets are skipped.
fn track_list(w: &[&[Token]], cx: &Ctx) -> Option<Vec<TrackEntry>> {
    if let [one] = w {
        if values::keyword(one).as_deref() == Some("none") {
            return Some(Vec::new());
        }
    }
    let mut out = Vec::new();
    for t in w {
        if t.first() == Some(&Token::LBracket) {
            continue;
        }
        match t {
            [Token::Function(f), rest @ ..] if f == "repeat" => {
                let inner = &rest[..rest.len().checked_sub(1)?];
                let at = inner.iter().position(|x| *x == Token::Comma)?;
                let count = css::trim(&inner[..at]);
                let count = match values::keyword(count).as_deref() {
                    Some("auto-fill") => Repeat::AutoFill,
                    Some("auto-fit") => Repeat::AutoFit,
                    _ => Repeat::Count(integer(count).filter(|n| (1..=1000).contains(n))? as u16),
                };
                let tracks = words(css::trim(&inner[at + 1..]))
                    .into_iter()
                    .filter(|x| x.first() != Some(&Token::LBracket))
                    .map(|x| track(x, cx))
                    .collect::<Option<Vec<_>>>()?;
                if tracks.is_empty() {
                    return None;
                }
                out.push(TrackEntry::Repeat(count, tracks));
            }
            _ => out.push(TrackEntry::One(track(t, cx)?)),
        }
    }
    Some(out)
}

/// One grid line: `auto`, `3`, `-1`, `span 2`.
fn line(t: &[Token]) -> Option<Line> {
    let w = words(t);
    match w.as_slice() {
        [one] if values::keyword(one).as_deref() == Some("auto") => Some(Line::Auto),
        [one] => integer(one)
            .filter(|n| *n != 0 && n.abs() <= 10_000)
            .map(|n| Line::Index(n as i16)),
        [a, b] if values::keyword(a).as_deref() == Some("span") => integer(b)
            .filter(|n| (1..=10_000).contains(n))
            .map(|n| Line::Span(n as u16)),
        _ => None,
    }
}

/// `grid-column: 1 / 3`, `span 2`, `2 / span 2`.
fn placement(t: &[Token]) -> Option<Placement> {
    let parts = css::split_top(t, &Token::Delim('/'));
    match parts.as_slice() {
        [a] => {
            let start = line(css::trim(a))?;
            Some(Placement {
                start,
                end: Line::Auto,
            })
        }
        [a, b] => Some(Placement {
            start: line(css::trim(a))?,
            end: line(css::trim(b))?,
        }),
        _ => None,
    }
}

/// `linear-gradient(…)`: an angle or `to <side>`, then colour stops.
fn gradient(t: &[Token], current: Color) -> Option<Gradient> {
    let [Token::Function(f), rest @ ..] = t else {
        return None;
    };
    if f != "linear-gradient" {
        return None;
    }
    let inner = &rest[..rest.len().checked_sub(1)?];
    let parts = css::split_top(inner, &Token::Comma);
    let mut angle = 180.0;
    let mut stops_from = 0;
    if let Some(first) = parts.first() {
        let first = css::trim(first);
        let fw = words(first);
        match first {
            [Token::Dimension(n, u)] => {
                angle = match u.as_str() {
                    "deg" => *n,
                    "turn" => n * 360.0,
                    "rad" => n.to_degrees(),
                    "grad" => n * 0.9,
                    _ => return None,
                };
                stops_from = 1;
            }
            _ if fw
                .first()
                .is_some_and(|x| values::keyword(x).as_deref() == Some("to")) =>
            {
                let dirs: Vec<String> = fw[1..].iter().filter_map(|x| values::keyword(x)).collect();
                let has = |d: &str| dirs.iter().any(|x| x == d);
                let (x, y) = (
                    if has("right") {
                        1.0
                    } else if has("left") {
                        -1.0
                    } else {
                        0.0
                    },
                    if has("bottom") {
                        1.0
                    } else if has("top") {
                        -1.0
                    } else {
                        0.0
                    },
                );
                angle = f32::atan2(x, -y).to_degrees();
                stops_from = 1;
            }
            _ => {}
        }
    }
    let mut stops = Vec::new();
    for p in &parts[stops_from..] {
        let pw = words(css::trim(p));
        let col = values::color(pw.first()?, current)?;
        let at = match pw.get(1) {
            Some([Token::Percentage(p)]) => Some(p / 100.0),
            Some(_) => None,
            None => None,
        };
        stops.push((col, at));
    }
    (stops.len() >= 2).then_some(Gradient { angle, stops })
}

/// `[inset] x y [blur [spread]] [color]`, in any order around the lengths.
fn shadow(w: &[&[Token]], cx: &Ctx, current: Color) -> Option<Shadow> {
    let mut lengths = Vec::new();
    let mut color = None;
    let mut inset = false;
    for t in w {
        if values::keyword(t).as_deref() == Some("inset") {
            inset = true;
        } else if let Some(p) = values::px(t, cx) {
            lengths.push(p);
        } else {
            color = Some(values::color(t, current)?);
        }
    }
    let get = |i: usize| lengths.get(i).copied().unwrap_or(0.0);
    (2..=4).contains(&lengths.len()).then(|| Shadow {
        x: get(0),
        y: get(1),
        blur: get(2).max(0.0),
        spread: get(3),
        color,
        inset,
    })
}

fn font_size(t: &[Token], parent: f32, cx: &Ctx) -> Option<f32> {
    let k = values::keyword(t);
    let size = match k.as_deref() {
        Some("xx-small") => 9.0,
        Some("x-small") => 10.0,
        Some("small") => 13.0,
        Some("medium") => 16.0,
        Some("large") => 18.0,
        Some("x-large") => 24.0,
        Some("xx-large") => 32.0,
        Some("xxx-large") => 48.0,
        Some("smaller") => parent / 1.2,
        Some("larger") => parent * 1.2,
        Some(_) => return None,
        None => match t {
            [Token::Percentage(p)] => parent * p / 100.0,
            _ => values::px(t, &Ctx { em: parent, ..*cx })?,
        },
    };
    // Past this a glyph is bigger than any screen; under it, invisible.
    Some(size.clamp(1.0, 1000.0))
}

fn font_weight(t: &[Token], parent: u16) -> Option<u16> {
    match values::keyword(t).as_deref() {
        Some("normal") => Some(400),
        Some("bold") => Some(700),
        Some("bolder") => Some(if parent < 400 {
            400
        } else if parent < 600 {
            700
        } else {
            900
        }),
        Some("lighter") => Some(if parent < 600 {
            100
        } else if parent < 800 {
            400
        } else {
            700
        }),
        Some(_) => None,
        None => values::number(t)
            .filter(|n| (1.0..=1000.0).contains(n))
            .map(|n| n as u16),
    }
}

/// Whether a family list asks for a fixed-width font: `monospace`, or one
/// of the names people reach for when they mean it.
fn monospace(t: &[Token]) -> bool {
    css::split_top(t, &Token::Comma).iter().any(|f| {
        let name = css::to_css(css::trim(f))
            .trim_matches('"')
            .to_ascii_lowercase();
        matches!(
            name.as_str(),
            "monospace"
                | "ui-monospace"
                | "menlo"
                | "monaco"
                | "consolas"
                | "courier"
                | "courier new"
                | "sf mono"
                | "jetbrains mono"
                | "fira code"
                | "fira mono"
                | "source code pro"
                | "roboto mono"
                | "dejavu sans mono"
                | "liberation mono"
                | "cascadia code"
                | "ubuntu mono"
                | "noto sans mono"
        )
    })
}

fn line_height(t: &[Token], cx: &Ctx) -> Option<LineHeight> {
    match t {
        [Token::Ident(k)] if k.eq_ignore_ascii_case("normal") => Some(LineHeight::Normal),
        [Token::Number(n)] if *n >= 0.0 => Some(LineHeight::Factor(*n)),
        [Token::Percentage(p)] if *p >= 0.0 => Some(LineHeight::Px(cx.em * p / 100.0)),
        _ => values::px(t, cx).filter(|p| *p >= 0.0).map(LineHeight::Px),
    }
}

/// `font: [style] [weight] size[/line-height] family`.
fn font(s: &mut Style, parent: &Style, w: &[&[Token]], value: &[Token], cx: &Ctx) {
    // The size is the first word that reads as one; the family follows it.
    let at = w.iter().position(|t| {
        let size_part = css::split_top(t, &Token::Delim('/'));
        font_size(css::trim(&size_part[0]), parent.font_size, cx).is_some()
            && values::keyword(css::trim(&size_part[0])).as_deref() != Some("normal")
    });
    let Some(at) = at else {
        return;
    };
    let size_part = css::split_top(w[at], &Token::Delim('/'));
    let Some(size) = font_size(css::trim(&size_part[0]), parent.font_size, cx) else {
        return;
    };
    s.font_size = size;
    s.italic = false;
    s.font_weight = 400;
    s.line_height = LineHeight::Normal;
    for t in &w[..at] {
        match values::keyword(t).as_deref() {
            Some("italic") | Some("oblique") => s.italic = true,
            _ => {
                if let Some(wt) = font_weight(t, parent.font_weight) {
                    s.font_weight = wt;
                }
            }
        }
    }
    if let Some(lh) = size_part.get(1) {
        let lcx = Ctx { em: size, ..*cx };
        if let Some(l) = line_height(css::trim(lh), &lcx) {
            s.line_height = l;
        }
    }
    s.monospace = monospace(&value[after(value, w[at])..]);
}

/// Where in `value` the word `part` — a slice of it, from `words` — ends.
fn after(value: &[Token], part: &[Token]) -> usize {
    let start = (part.as_ptr() as usize - value.as_ptr() as usize) / std::mem::size_of::<Token>();
    (start + part.len()).min(value.len())
}

fn is_inherited(name: &str) -> bool {
    matches!(
        name,
        "color"
            | "font"
            | "font-size"
            | "font-weight"
            | "font-style"
            | "font-family"
            | "line-height"
            | "text-align"
            | "white-space"
            | "text-transform"
            | "letter-spacing"
            | "visibility"
            | "cursor"
            | "list-style"
            | "list-style-type"
            | "pointer-events"
    )
}

/// `name` (and, for a shorthand, what it covers) copied from `from`: what
/// `inherit`, `initial` and `unset` do.
fn copy_property(s: &mut Style, from: &Style, name: &str) {
    match name {
        "display" => s.display = from.display,
        "position" => s.position = from.position,
        "top" => s.inset[0] = from.inset[0],
        "right" => s.inset[1] = from.inset[1],
        "bottom" => s.inset[2] = from.inset[2],
        "left" => s.inset[3] = from.inset[3],
        "inset" => s.inset = from.inset,
        "width" => s.width = from.width,
        "height" => s.height = from.height,
        "min-width" => s.min_width = from.min_width,
        "min-height" => s.min_height = from.min_height,
        "max-width" => s.max_width = from.max_width,
        "max-height" => s.max_height = from.max_height,
        "margin" => s.margin = from.margin,
        "padding" => s.padding = from.padding,
        "border" => {
            s.border_width = from.border_width;
            s.border_style = from.border_style;
            s.border_color = from.border_color;
        }
        "border-width" => s.border_width = from.border_width,
        "border-style" => s.border_style = from.border_style,
        "border-color" => s.border_color = from.border_color,
        "border-radius" => s.radius = from.radius,
        "box-sizing" => s.box_sizing = from.box_sizing,
        "overflow" => s.overflow = from.overflow,
        "overflow-x" => s.overflow[0] = from.overflow[0],
        "overflow-y" => s.overflow[1] = from.overflow[1],
        "flex-direction" => s.flex_direction = from.flex_direction,
        "flex-wrap" => s.flex_wrap = from.flex_wrap,
        "justify-content" => s.justify_content = from.justify_content,
        "align-items" => s.align_items = from.align_items,
        "align-self" => s.align_self = from.align_self,
        "align-content" => s.align_content = from.align_content,
        "flex" => {
            s.flex_grow = from.flex_grow;
            s.flex_shrink = from.flex_shrink;
            s.flex_basis = from.flex_basis;
        }
        "flex-grow" => s.flex_grow = from.flex_grow,
        "flex-shrink" => s.flex_shrink = from.flex_shrink,
        "flex-basis" => s.flex_basis = from.flex_basis,
        "gap" => s.gap = from.gap,
        "background" | "background-color" => {
            s.background = from.background;
            s.gradient = from.gradient.clone();
        }
        "opacity" => s.opacity = from.opacity,
        "box-shadow" => s.shadow = from.shadow,
        "color" => s.color = from.color,
        "font-size" => s.font_size = from.font_size,
        "font-weight" => s.font_weight = from.font_weight,
        "font-style" => s.italic = from.italic,
        "font-family" => s.monospace = from.monospace,
        "font" => {
            s.font_size = from.font_size;
            s.font_weight = from.font_weight;
            s.italic = from.italic;
            s.monospace = from.monospace;
            s.line_height = from.line_height;
        }
        "line-height" => s.line_height = from.line_height,
        "text-align" => s.text_align = from.text_align,
        "white-space" => s.white_space = from.white_space,
        "text-transform" => s.text_transform = from.text_transform,
        "letter-spacing" => s.letter_spacing = from.letter_spacing,
        "text-decoration" | "text-decoration-line" => s.text_decoration = from.text_decoration,
        "visibility" => s.visibility = from.visibility,
        "cursor" => s.cursor = from.cursor,
        "list-style" | "list-style-type" => s.list_style = from.list_style,
        "pointer-events" => s.pointer_events = from.pointer_events,
        "z-index" => s.z_index = from.z_index,
        "float" => s.float = from.float,
        "clear" => s.clear = from.clear,
        "order" => s.order = from.order,
        _ => {}
    }
}

/// The properties `style` sets differently from where it started — what
/// its own rules did, for the headless runner to print: for an inherited
/// property, against `parent`; for the rest, against their initial values
/// and the tag's display.
pub fn describe(style: &Style, parent: &Style, tag: &str) -> String {
    let base = Style::child_of(parent, tag);
    let mut out: Vec<String> = Vec::new();
    let mut put = |name: &str, value: String| out.push(format!("{name}: {value}"));
    let quad = |v: &[Length; 4]| short(&v.iter().map(Length::css).collect::<Vec<_>>());
    let pxs = |v: &[f32]| {
        short(
            &v.iter()
                .map(|x| format!("{}px", css::number(*x)))
                .collect::<Vec<_>>(),
        )
    };
    if style.display != base.display {
        put("display", style.display.css().into());
    }
    if style.position != base.position {
        put("position", style.position.css().into());
    }
    if style.inset != base.inset {
        put("inset", quad(&style.inset));
    }
    for (name, a, b) in [
        ("width", style.width, base.width),
        ("height", style.height, base.height),
        ("min-width", style.min_width, base.min_width),
        ("min-height", style.min_height, base.min_height),
        ("max-width", style.max_width, base.max_width),
        ("max-height", style.max_height, base.max_height),
    ] {
        if a != b {
            put(name, a.css());
        }
    }
    if style.margin != base.margin {
        put("margin", quad(&style.margin));
    }
    if style.padding != base.padding {
        put("padding", quad(&style.padding));
    }
    if style.border_width != base.border_width {
        let colors: Vec<String> = style
            .border_color
            .iter()
            .map(|c| c.map_or("currentcolor".into(), |c| c.css()))
            .collect();
        let styles: Vec<String> = style
            .border_style
            .iter()
            .map(|s| s.css().to_string())
            .collect();
        put(
            "border",
            format!(
                "{} {} {}",
                pxs(&style.border_width),
                short(&styles),
                short(&colors)
            ),
        );
    }
    if style.radius != base.radius {
        put("border-radius", quad(&style.radius));
    }
    if style.box_sizing != base.box_sizing {
        put("box-sizing", style.box_sizing.css().into());
    }
    if style.overflow != base.overflow {
        put(
            "overflow",
            short(&[
                style.overflow[0].css().into(),
                style.overflow[1].css().into(),
            ]),
        );
    }
    if style.flex_direction != base.flex_direction {
        put("flex-direction", style.flex_direction.css().into());
    }
    if style.flex_wrap != base.flex_wrap {
        put("flex-wrap", style.flex_wrap.css().into());
    }
    for (name, a, b) in [
        (
            "justify-content",
            style.justify_content,
            base.justify_content,
        ),
        ("align-items", style.align_items, base.align_items),
        ("align-self", style.align_self, base.align_self),
        ("align-content", style.align_content, base.align_content),
        ("justify-items", style.justify_items, base.justify_items),
        ("justify-self", style.justify_self, base.justify_self),
    ] {
        if a != b {
            put(name, a.css().into());
        }
    }
    if (style.flex_grow, style.flex_shrink, style.flex_basis)
        != (base.flex_grow, base.flex_shrink, base.flex_basis)
    {
        put(
            "flex",
            format!(
                "{} {} {}",
                css::number(style.flex_grow),
                css::number(style.flex_shrink),
                style.flex_basis.css()
            ),
        );
    }
    if style.order != base.order {
        put("order", style.order.to_string());
    }
    if style.gap != base.gap {
        put("gap", short(&[style.gap[0].css(), style.gap[1].css()]));
    }
    if style.grid_template_columns != base.grid_template_columns {
        put(
            "grid-template-columns",
            tracks_css(&style.grid_template_columns),
        );
    }
    if style.grid_template_rows != base.grid_template_rows {
        put("grid-template-rows", tracks_css(&style.grid_template_rows));
    }
    if style.grid_column != base.grid_column {
        put("grid-column", placement_css(style.grid_column));
    }
    if style.grid_row != base.grid_row {
        put("grid-row", placement_css(style.grid_row));
    }
    if style.aspect_ratio != base.aspect_ratio {
        put(
            "aspect-ratio",
            style.aspect_ratio.map_or("auto".into(), css::number),
        );
    }
    if style.float != base.float {
        put("float", style.float.css().into());
    }
    if style.z_index != base.z_index {
        put(
            "z-index",
            style.z_index.map_or("auto".into(), |z| z.to_string()),
        );
    }
    if style.background != base.background {
        put("background", style.background.css());
    }
    if let Some(g) = &style.gradient {
        let stops: Vec<String> = g.stops.iter().map(|(c, _)| c.css()).collect();
        put(
            "background-image",
            format!(
                "linear-gradient({}deg, {})",
                css::number(g.angle),
                stops.join(", ")
            ),
        );
    }
    if style.opacity != base.opacity {
        put("opacity", css::number(style.opacity));
    }
    if let Some(sh) = &style.shadow {
        put(
            "box-shadow",
            format!(
                "{}{}px {}px {}px {}px {}",
                if sh.inset { "inset " } else { "" },
                css::number(sh.x),
                css::number(sh.y),
                css::number(sh.blur),
                css::number(sh.spread),
                sh.color.map_or("currentcolor".into(), |c| c.css())
            ),
        );
    }
    if style.color != base.color {
        put("color", style.color.css());
    }
    if style.font_size != base.font_size {
        put("font-size", format!("{}px", css::number(style.font_size)));
    }
    if style.font_weight != base.font_weight {
        put("font-weight", style.font_weight.to_string());
    }
    if style.italic != base.italic {
        put(
            "font-style",
            if style.italic { "italic" } else { "normal" }.into(),
        );
    }
    if style.monospace != base.monospace {
        put(
            "font-family",
            if style.monospace {
                "monospace"
            } else {
                "sans-serif"
            }
            .into(),
        );
    }
    if style.line_height != base.line_height {
        put(
            "line-height",
            match style.line_height {
                LineHeight::Normal => "normal".into(),
                LineHeight::Factor(f) => css::number(f),
                LineHeight::Px(p) => format!("{}px", css::number(p)),
            },
        );
    }
    if style.text_align != base.text_align {
        put("text-align", style.text_align.css().into());
    }
    if style.white_space != base.white_space {
        put("white-space", style.white_space.css().into());
    }
    if style.text_transform != base.text_transform {
        put("text-transform", style.text_transform.css().into());
    }
    if style.letter_spacing != base.letter_spacing {
        put(
            "letter-spacing",
            format!("{}px", css::number(style.letter_spacing)),
        );
    }
    if style.text_decoration != base.text_decoration {
        let mut d = Vec::new();
        if style.text_decoration & UNDERLINE != 0 {
            d.push("underline");
        }
        if style.text_decoration & LINE_THROUGH != 0 {
            d.push("line-through");
        }
        if style.text_decoration & OVERLINE != 0 {
            d.push("overline");
        }
        put(
            "text-decoration",
            if d.is_empty() {
                "none".into()
            } else {
                d.join(" ")
            },
        );
    }
    if style.visibility != base.visibility {
        put("visibility", style.visibility.css().into());
    }
    if style.cursor != base.cursor {
        put("cursor", style.cursor.css().into());
    }
    if style.list_style != base.list_style {
        put("list-style", style.list_style.css().into());
    }
    if style.pointer_events != base.pointer_events {
        put("pointer-events", style.pointer_events.css().into());
    }
    out.join("; ")
}

/// Side values the way a shorthand writes them: `8px`, `8px 16px`, …
fn short(v: &[String]) -> String {
    match v {
        [a, b] if a == b => a.clone(),
        [a, b, c, d] if a == b && b == c && c == d => a.clone(),
        [a, b, c, d] if a == c && b == d => format!("{a} {b}"),
        [a, b, c, d] if b == d => format!("{a} {b} {c}"),
        _ => v.join(" "),
    }
}

fn track_css(t: &Track) -> String {
    match t {
        Track::Fixed(l) => l.css(),
        Track::Fr(f) => format!("{}fr", css::number(*f)),
        Track::Auto => "auto".into(),
        Track::MinContent => "min-content".into(),
        Track::MaxContent => "max-content".into(),
        Track::MinMax(a, b) => format!("minmax({}, {})", track_css(a), track_css(b)),
        Track::FitContent(l) => format!("fit-content({})", l.css()),
    }
}

fn tracks_css(list: &[TrackEntry]) -> String {
    if list.is_empty() {
        return "none".into();
    }
    list.iter()
        .map(|e| match e {
            TrackEntry::One(t) => track_css(t),
            TrackEntry::Repeat(r, ts) => {
                let count = match r {
                    Repeat::Count(n) => n.to_string(),
                    Repeat::AutoFill => "auto-fill".into(),
                    Repeat::AutoFit => "auto-fit".into(),
                };
                let inner: Vec<String> = ts.iter().map(track_css).collect();
                format!("repeat({count}, {})", inner.join(" "))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn placement_css(p: Placement) -> String {
    let l = |l: Line| match l {
        Line::Auto => "auto".to_string(),
        Line::Index(i) => i.to_string(),
        Line::Span(n) => format!("span {n}"),
    };
    format!("{} / {}", l(p.start), l(p.end))
}
