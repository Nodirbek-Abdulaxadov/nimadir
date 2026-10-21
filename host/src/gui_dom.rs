//! The element tree, drawn by egui (ADR-4) — part of the egui backend, as
//! `gui.rs` is.
//!
//! The core decides what to draw (`dom::paint`); this file only turns each
//! item into egui's shapes and widgets, measures text with egui's fonts
//! (`EguiFonts`), and hands what the user does back to the document: the
//! element under the pointer for `:hover`, the one pressed for `:active`, a
//! tap, a scroll, a field's text. Fields are egui's own `TextEdit`s, placed
//! where layout put them and keyed by node, so focus survives a page that
//! changes around them.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use eframe::egui;
use egui::epaint::{CornerRadius, Mesh, RectShape, Shape, Stroke, StrokeKind, Tessellator};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, Pos2};

use crate::dom::css::Env;
use crate::dom::layout::{Align, Font, Fonts, Paragraph, Rect};
use crate::dom::paint::Item;
use crate::dom::style::{self as st, Gradient, LINE_THROUGH, UNDERLINE};
use crate::dom::values::Color;
use crate::dom::NodeId;
use crate::shell::Shell;

/// The families an app's text is drawn in: Noto Sans, regular and bold, so
/// a page reads as one face whatever its weights (`install_fonts`).
pub const NOTO: &str = "noto-sans";
pub const NOTO_BOLD: &str = "noto-sans-bold";

/// What the window remembers about the page between frames.
#[derive(Default)]
pub struct DomView {
    /// Images uploaded to the GPU, by `Picture::id`; dropped once not drawn.
    textures: HashMap<u64, egui::TextureHandle>,
    /// The focus the document had last frame, to notice the app moving it.
    focus_seen: Option<NodeId>,
    /// Whether a press is on the page, for `pointerdown` / `pointerup`.
    down: Option<NodeId>,
}

impl DomView {
    /// A page left behind takes its textures and state with it.
    pub fn clear(&mut self) {
        *self = DomView::default();
    }
}

/// egui's fonts, measuring for layout.
pub struct EguiFonts {
    pub ctx: egui::Context,
}

fn color32(c: Color) -> Color32 {
    let [r, g, b, a] = c.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn text_format(f: &Font, line_height: f32) -> TextFormat {
    let family = if f.mono {
        FontFamily::Monospace
    } else if f.bold {
        FontFamily::Name(NOTO_BOLD.into())
    } else {
        FontFamily::Name(NOTO.into())
    };
    let color = color32(f.color);
    let line = Stroke::new((f.size / 14.0).max(1.0), color);
    TextFormat {
        font_id: FontId::new(f.size, family),
        color,
        italics: f.italic,
        underline: if f.decoration & UNDERLINE != 0 {
            line
        } else {
            Stroke::NONE
        },
        strikethrough: if f.decoration & LINE_THROUGH != 0 {
            line
        } else {
            Stroke::NONE
        },
        extra_letter_spacing: f.letter_spacing,
        line_height: Some(line_height),
        valign: egui::Align::Center,
        ..Default::default()
    }
}

/// `p` as egui lays text out, at most `width` wide.
fn job(p: &Paragraph, width: Option<f32>) -> LayoutJob {
    let mut job = LayoutJob::default();
    for run in &p.runs {
        job.append(&run.text, 0.0, text_format(&run.font, p.line_height));
    }
    job.wrap.max_width = match (p.wrap || p.ellipsis, width) {
        (true, Some(w)) => w.max(1.0),
        _ => f32::INFINITY,
    };
    if p.ellipsis {
        job.wrap.max_rows = 1;
        job.wrap.overflow_character = Some('…');
    }
    job.halign = match p.align {
        Align::Left => egui::Align::LEFT,
        Align::Center => egui::Align::Center,
        Align::Right => egui::Align::RIGHT,
    };
    job
}

impl EguiFonts {
    fn galley(&self, p: &Paragraph, width: Option<f32>) -> Arc<egui::Galley> {
        self.ctx.fonts_mut(|f| f.layout_job(job(p, width)))
    }
}

impl Fonts for EguiFonts {
    fn measure(&self, p: &Paragraph, width: Option<f32>) -> (f32, f32) {
        let g = self.galley(p, width);
        (g.rect.width().ceil(), g.rect.height().ceil())
    }

    fn min_width(&self, p: &Paragraph) -> f32 {
        if !p.wrap {
            return self.measure(p, None).0;
        }
        // A word wider than the line overflows its row rather than breaking,
        // so at almost no width each word has a row: the widest is the answer.
        self.galley(p, Some(1.0)).rect.width().ceil()
    }

    fn run_at(&self, p: &Paragraph, width: f32, x: f32, y: f32) -> Option<usize> {
        let g = self.galley(p, Some(width));
        let origin = match p.align {
            Align::Left => 0.0,
            Align::Center => width / 2.0,
            Align::Right => width,
        };
        let pos = egui::vec2(x - origin, y);
        if !g.rect.expand(2.0).contains(pos.to_pos2()) {
            return None;
        }
        let at = g.cursor_from_pos(pos).index.0;
        let mut seen = 0;
        for (i, r) in p.runs.iter().enumerate() {
            seen += r.text.chars().count();
            if at < seen {
                return Some(i);
            }
        }
        p.runs.len().checked_sub(1)
    }
}

fn corners(r: [f32; 4]) -> CornerRadius {
    let c = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    CornerRadius {
        nw: c(r[0]),
        ne: c(r[1]),
        se: c(r[2]),
        sw: c(r[3]),
    }
}

/// Draw the open app's page at the top of `ui` and take what the user did
/// to it. Whether the app has something to hear about, which needs a frame.
pub fn show(ui: &mut egui::Ui, shell: &mut Shell, view: &mut DomView, reveal: bool) -> bool {
    let ctx = ui.ctx().clone();
    let env = Env {
        width: ui.available_width().max(1.0),
        height: ctx.content_rect().height(),
        dark: ui.visuals().dark_mode,
        // A phone has no pointer that hovers; `:hover` would stick to the
        // last place touched.
        hover: !cfg!(target_os = "android"),
    };
    let fonts = EguiFonts { ctx: ctx.clone() };
    let Some(height) = shell.layout_page(&env, &fonts).map(|l| l.height) else {
        return false;
    };
    let (page, resp) =
        ui.allocate_exact_size(egui::vec2(env.width, height.max(1.0)), egui::Sense::click());
    let origin = page.min;
    let to_screen = |r: &Rect| {
        egui::Rect::from_min_size(
            Pos2::new(origin.x + r.x, origin.y + r.y),
            egui::vec2(r.w, r.h),
        )
    };

    // Images the page draws, read before the document is borrowed.
    let items = shell
        .document()
        .map(|d| d.display_list())
        .unwrap_or_default();
    let mut images = HashMap::new();
    for item in &items {
        if let Item::Image { handle, .. } = item {
            if let Some(p) = shell.image(*handle) {
                images.insert(*handle, p);
            }
        }
    }
    let Some(doc) = shell.document_mut() else {
        return false;
    };

    // Draw.
    let base_clip = ui.clip_rect();
    let mut clips: Vec<egui::Rect> = Vec::new();
    let mut fields = Vec::new();
    let mut selects = Vec::new();
    let mut ranges = Vec::new();
    let mut used = HashSet::new();
    for item in items {
        let clip = clips.last().copied().unwrap_or(base_clip);
        let painter = ui.painter().with_clip_rect(clip);
        match item {
            Item::Clip(r) => clips.push(to_screen(&r).intersect(base_clip)),
            Item::Unclip => {
                clips.pop();
            }
            Item::Shadow {
                rect,
                radius,
                shadow,
            } => {
                let c = |v: f32| v.round().clamp(-128.0, 127.0) as i8;
                let u = |v: f32| v.round().clamp(0.0, 255.0) as u8;
                let sh = egui::epaint::Shadow {
                    offset: [c(shadow.x), c(shadow.y)],
                    blur: u(shadow.blur),
                    spread: u(shadow.spread.max(0.0)),
                    color: color32(shadow.color.unwrap_or(Color::BLACK)),
                };
                painter.add(sh.as_shape(to_screen(&rect), corners(radius)));
            }
            Item::Fill {
                rect,
                radius,
                color,
            } => {
                painter.rect_filled(to_screen(&rect), corners(radius), color32(color));
            }
            Item::Gradient {
                rect,
                radius,
                gradient,
            } => {
                painter.add(gradient_mesh(
                    &ctx,
                    to_screen(&rect),
                    corners(radius),
                    &gradient,
                ));
            }
            Item::Border {
                rect,
                radius,
                widths,
                colors,
                ..
            } => border(&painter, to_screen(&rect), radius, widths, colors),
            Item::Text { rect, paragraph } => {
                let r = to_screen(&rect);
                let g = fonts.galley(&paragraph, Some(r.width()));
                let x = match paragraph.align {
                    Align::Left => r.left(),
                    Align::Center => r.center().x,
                    Align::Right => r.right(),
                };
                painter.galley(Pos2::new(x, r.top()), g, Color32::PLACEHOLDER);
            }
            Item::Image {
                rect,
                handle,
                opacity,
                fit,
            } => {
                let Some(p) = images.get(&handle) else {
                    continue;
                };
                let texture = view.textures.entry(p.id).or_insert_with(|| {
                    ctx.load_texture(
                        format!("dom-image-{}", p.id),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [p.width as usize, p.height as usize],
                            &p.rgba,
                        ),
                        egui::TextureOptions::LINEAR,
                    )
                });
                used.insert(p.id);
                let (dest, uv) = fitted(to_screen(&rect), p.width as f32, p.height as f32, fit);
                painter
                    .with_clip_rect(clip.intersect(to_screen(&rect)))
                    .image(
                        texture.id(),
                        dest,
                        uv,
                        Color32::WHITE.gamma_multiply(opacity),
                    );
            }
            Item::Field { .. } => fields.push((item, clip)),
            Item::Select {
                rect,
                node,
                options,
                selected,
                font,
                disabled,
            } => {
                let r = to_screen(&rect);
                let color = color32(font.color);
                let label = Paragraph {
                    runs: vec![crate::dom::layout::Run {
                        text: options.get(selected).cloned().unwrap_or_default(),
                        font,
                        node,
                    }],
                    align: Align::Left,
                    wrap: false,
                    line_height: font.line,
                    ellipsis: true,
                };
                let g = fonts.galley(&label, Some(r.width()));
                painter
                    .with_clip_rect(clip.intersect(r.expand2(egui::vec2(0.0, 4.0))))
                    .galley(
                        Pos2::new(r.left(), r.center().y - g.rect.height() / 2.0),
                        g,
                        color,
                    );
                // The arrow, in the padding the sheet leaves on the right.
                let c = Pos2::new(r.right() + 16.0, r.center().y);
                let s = font.size * 0.28;
                painter.add(Shape::convex_polygon(
                    vec![
                        Pos2::new(c.x - s, c.y - s * 0.5),
                        Pos2::new(c.x + s, c.y - s * 0.5),
                        Pos2::new(c.x, c.y + s * 0.6),
                    ],
                    color,
                    Stroke::NONE,
                ));
                selects.push((
                    node,
                    r.expand2(egui::vec2(10.0, 8.0))
                        .with_max_x(r.right() + 32.0),
                    options,
                    selected,
                    disabled,
                    clip,
                ));
            }
            Item::Check {
                rect,
                checked,
                radio,
                mark,
            } => {
                if checked {
                    let r = to_screen(&rect);
                    let color = color32(mark);
                    if radio {
                        painter.circle_filled(r.center(), r.width().min(r.height()) * 0.22, color);
                    } else {
                        let s = r.width().min(r.height());
                        let p = |x: f32, y: f32| Pos2::new(r.left() + x * s, r.top() + y * s);
                        painter.line(
                            vec![p(0.22, 0.52), p(0.42, 0.72), p(0.78, 0.3)],
                            Stroke::new((s * 0.13).max(1.5), color),
                        );
                    }
                }
            }
            Item::Range {
                rect,
                node,
                fraction,
                min,
                max,
                step,
                accent,
                track,
                disabled,
            } => {
                let r = to_screen(&rect);
                if !disabled {
                    ranges.push((node, r, min, max, step, clip));
                }
                // The thumb stays inside the box: the track is what is left
                // of it once half a thumb is taken off each end.
                let thumb = thumb_radius(r);
                let y = r.center().y;
                let (left, right) = (r.left() + thumb, r.right() - thumb);
                let x = left + (right - left) * fraction;
                painter.line_segment(
                    [Pos2::new(left, y), Pos2::new(right, y)],
                    Stroke::new(4.0, color32(track)),
                );
                painter.line_segment(
                    [Pos2::new(left, y), Pos2::new(x, y)],
                    Stroke::new(4.0, color32(accent)),
                );
                painter.circle_filled(Pos2::new(x, y), thumb, color32(accent));
            }
            Item::Progress {
                rect,
                radius,
                fraction,
                color,
            } => {
                let mut r = to_screen(&rect);
                r.set_width(r.width() * fraction);
                painter.rect_filled(r, corners(radius), color32(color));
            }
            Item::Marker {
                rect,
                kind,
                ordinal,
                font,
                line,
            } => {
                let r = to_screen(&rect);
                let y = r.top() + line / 2.0;
                let color = color32(font.color);
                let s = font.size * 0.18;
                let x = r.left() - font.size * 0.8;
                match kind {
                    st::ListStyle::Disc => {
                        painter.circle_filled(Pos2::new(x, y), s, color);
                    }
                    st::ListStyle::Circle => {
                        painter.circle_stroke(Pos2::new(x, y), s, Stroke::new(1.0, color));
                    }
                    st::ListStyle::Square => {
                        painter.rect_filled(
                            egui::Rect::from_center_size(
                                Pos2::new(x, y),
                                egui::vec2(s * 2.0, s * 2.0),
                            ),
                            0.0,
                            color,
                        );
                    }
                    st::ListStyle::Decimal => {
                        painter.text(
                            Pos2::new(r.left() - font.size * 0.4, y),
                            egui::Align2::RIGHT_CENTER,
                            format!("{ordinal}."),
                            FontId::new(font.size, FontFamily::Name(NOTO.into())),
                            color,
                        );
                    }
                    st::ListStyle::None => {}
                }
            }
        }
    }
    view.textures.retain(|id, _| used.contains(id));

    // What the user does to the page. Fields first take what is theirs.
    let mut want_focus = None;
    if doc.focused != view.focus_seen {
        want_focus = doc.focused;
        view.focus_seen = doc.focused;
    }
    for (item, clip) in fields {
        let Item::Field {
            rect,
            node,
            value,
            placeholder,
            font,
            multiline,
            password,
            disabled,
            readonly,
            max_chars,
        } = item
        else {
            continue;
        };
        let r = to_screen(&rect);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
        // A field's text stays in its box: a long line scrolls inside it,
        // and a textarea's lines scroll under it.
        child.set_clip_rect(clip.intersect(base_clip).intersect(r));
        let mut text = value;
        let format = text_format(&font, font.line);
        let edit = if multiline {
            egui::TextEdit::multiline(&mut text)
        } else {
            egui::TextEdit::singleline(&mut text)
        };
        let edit = edit
            .id(egui::Id::new(("dom-field", node)))
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(format.font_id.clone())
            .text_color(format.color)
            .hint_text(placeholder)
            .password(password)
            .interactive(!disabled && !readonly)
            .char_limit(max_chars.unwrap_or(usize::MAX))
            .desired_width(r.width());
        let field = if multiline {
            egui::ScrollArea::vertical()
                .id_salt(("dom-field-scroll", node))
                .max_height(r.height())
                .auto_shrink([false, false])
                .show(&mut child, |ui| ui.add(edit))
                .inner
        } else {
            child.put(r, edit.vertical_align(egui::Align::Center))
        };
        if want_focus == Some(node) {
            field.request_focus();
        }
        if field.changed() {
            let _ = doc.type_text(node, &text);
        }
        if field.gained_focus() {
            doc.user_focus(node);
            view.focus_seen = Some(node);
        }
        if field.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !multiline {
                doc.submit(node);
            }
            doc.user_blur(node);
            view.focus_seen = doc.focused;
        }
        if reveal && field.has_focus() {
            field.scroll_to_me(Some(egui::Align::Center));
        }
    }

    // A select's options, offered when it is tapped.
    for (node, r, options, selected, disabled, clip) in selects {
        if disabled {
            continue;
        }
        let area = r.intersect(clip);
        let tap = ui.interact(
            area,
            egui::Id::new(("dom-select", node)),
            egui::Sense::click(),
        );
        if tap.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let mut chosen = None;
        egui::Popup::menu(&tap).width(area.width()).show(|ui| {
            for (i, label) in options.iter().enumerate() {
                if ui.selectable_label(i == selected, label).clicked() {
                    chosen = Some(i);
                }
            }
        });
        if let Some(i) = chosen {
            let _ = doc.choose(node, i);
        }
    }

    // A range follows the finger along it, and says `change` when let go.
    for (node, r, min, max, step, clip) in ranges {
        let grip = ui.interact(
            r.expand2(egui::vec2(8.0, 10.0)).intersect(clip),
            egui::Id::new(("dom-range", node)),
            egui::Sense::click_and_drag(),
        );
        let done = grip.drag_stopped() || grip.clicked();
        if grip.dragged() || done {
            if let Some(p) = grip.interact_pointer_pos() {
                let thumb = thumb_radius(r);
                let f =
                    ((p.x - r.left() - thumb) / (r.width() - 2.0 * thumb).max(1.0)).clamp(0.0, 1.0);
                let mut v = min + f * (max - min);
                if step > 0.0 {
                    v = min + ((v - min) / step).round() * step;
                }
                // `max` then `min`: an app may give a `min` above its `max`.
                let _ = doc.slide(node, &crate::dom::css::number(v.max(min).min(max)), done);
            }
        }
    }

    // Scroll containers take a drag inside them — on a phone, the way to
    // scroll — before the page around them does.
    for (node, area) in doc.scroll_areas() {
        let a = ui.interact(
            to_screen(&area),
            egui::Id::new(("dom-scroll", node)),
            egui::Sense::drag(),
        );
        if a.dragged() {
            let d = a.drag_delta();
            let at = a.interact_pointer_pos().unwrap_or(a.rect.center());
            if doc.scroll_at(at.x - origin.x, at.y - origin.y, -d.x, -d.y) {
                ctx.request_repaint();
            }
        }
    }

    let local = |p: Pos2| (p.x - origin.x, p.y - origin.y);
    // :hover, and the cursor that goes with what is under it.
    // Over the page, whatever of ours is on top there — a field, a scroll
    // area — but not under a window or popup above it.
    let pointer = ctx
        .pointer_hover_pos()
        .filter(|p| page.contains(*p) && ui.rect_contains_pointer(page));
    let hovered = if env.hover {
        pointer.and_then(|p| {
            let (x, y) = local(p);
            doc.hit(&fonts, x, y)
        })
    } else {
        None
    };
    if doc.hovered != hovered {
        doc.hovered = hovered;
        ctx.request_repaint();
    }
    if let Some(cursor) = hovered.and_then(|h| cursor_of(doc, h)) {
        ctx.set_cursor_icon(cursor);
    }
    // A wheel scrolls the innermost container that can still move, and
    // only what it does not use reaches the page.
    if let Some(p) = pointer {
        let delta = ui.input(|i| i.smooth_scroll_delta);
        let (x, y) = local(p);
        if delta != egui::Vec2::ZERO && doc.scroll_at(x, y, -delta.x, -delta.y) {
            ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
            ctx.request_repaint();
        }
    }
    // :active, pointerdown and pointerup.
    let pressed = if resp.is_pointer_button_down_on() {
        ui.input(|i| i.pointer.press_origin()).and_then(|p| {
            let (x, y) = local(p);
            doc.hit(&fonts, x, y)
        })
    } else {
        None
    };
    if pressed != view.down {
        if let Some(old) = view.down {
            doc.dispatch("pointerup", old, "", 0.0, 0.0);
        }
        if let Some(new) = pressed {
            doc.dispatch("pointerdown", new, "", 0.0, 0.0);
        }
        view.down = pressed;
    }
    if doc.pressed != pressed {
        doc.pressed = pressed;
        ctx.request_repaint();
    }
    // A tap, where it landed relative to what it landed on.
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let (x, y) = local(p);
            if let Some(node) = doc.hit(&fonts, x, y) {
                let at = doc
                    .last_layout()
                    .and_then(|l| l.rect_of(node))
                    .unwrap_or_default();
                // A tap that is not on the field with the keyboard takes it
                // away — before the click, which may give it somewhere else.
                if let Some(f) = doc.focused.filter(|f| *f != node) {
                    doc.user_blur(f);
                    ctx.memory_mut(|m| m.surrender_focus(egui::Id::new(("dom-field", f))));
                    view.focus_seen = None;
                }
                doc.click(node, x - at.x, y - at.y);
            }
        }
    }
    doc.has_events() || doc.wants_navigation()
}

/// A range's thumb, for a box of `r`'s height.
fn thumb_radius(r: egui::Rect) -> f32 {
    (r.height() * 0.45).max(6.0).min(r.width() / 2.0)
}

/// The cursor an element's style asks for.
fn cursor_of(doc: &crate::dom::Document, id: NodeId) -> Option<egui::CursorIcon> {
    let s = doc.computed(id)?;
    Some(match s.cursor {
        st::Cursor::Pointer => egui::CursorIcon::PointingHand,
        st::Cursor::Text => egui::CursorIcon::Text,
        st::Cursor::NotAllowed => egui::CursorIcon::NotAllowed,
        st::Cursor::Grab => egui::CursorIcon::Grab,
        st::Cursor::Grabbing => egui::CursorIcon::Grabbing,
        st::Cursor::Move => egui::CursorIcon::Move,
        st::Cursor::Wait => egui::CursorIcon::Wait,
        st::Cursor::Progress => egui::CursorIcon::Progress,
        st::Cursor::Help => egui::CursorIcon::Help,
        st::Cursor::Crosshair => egui::CursorIcon::Crosshair,
        st::Cursor::None => egui::CursorIcon::None,
        st::Cursor::Default => egui::CursorIcon::Default,
        st::Cursor::Auto => return None,
    })
}

/// A border: one stroke when every side is the same, else each side as a
/// band along its edge, corners square.
fn border(
    painter: &egui::Painter,
    r: egui::Rect,
    radius: [f32; 4],
    widths: [f32; 4],
    colors: [Color; 4],
) {
    let uniform = widths.iter().all(|w| *w == widths[0]) && colors.iter().all(|c| *c == colors[0]);
    if uniform {
        if widths[0] > 0.0 {
            painter.rect_stroke(
                r,
                corners(radius),
                Stroke::new(widths[0], color32(colors[0])),
                StrokeKind::Inside,
            );
        }
        return;
    }
    let [t, rt, b, l] = widths;
    let bands = [
        egui::Rect::from_min_max(r.left_top(), Pos2::new(r.right(), r.top() + t)),
        egui::Rect::from_min_max(Pos2::new(r.right() - rt, r.top()), r.right_bottom()),
        egui::Rect::from_min_max(Pos2::new(r.left(), r.bottom() - b), r.right_bottom()),
        egui::Rect::from_min_max(r.left_top(), Pos2::new(r.left() + l, r.bottom())),
    ];
    for (band, (w, c)) in bands.iter().zip(widths.iter().zip(colors)) {
        if *w > 0.0 && !c.is_transparent() {
            painter.rect_filled(*band, 0.0, color32(c));
        }
    }
}

/// A linear gradient over a rounded rectangle: the rectangle tessellated
/// as egui would draw it, each vertex then coloured by where it falls along
/// the gradient — so the corners stay round and the edges smooth.
fn gradient_mesh(ctx: &egui::Context, r: egui::Rect, cr: CornerRadius, g: &Gradient) -> Shape {
    let mut tess = Tessellator::new(
        ctx.pixels_per_point(),
        ctx.tessellation_options(|o| *o),
        ctx.fonts(|f| f.font_image_size()),
        Vec::new(),
    );
    let mut mesh = Mesh::default();
    tess.tessellate_shape(
        Shape::Rect(RectShape::filled(r, cr, Color32::WHITE)),
        &mut mesh,
    );
    // CSS's angle: 0 points up, 90 right; the gradient line runs through the
    // centre, long enough that its ends meet the far corners.
    let a = g.angle.to_radians();
    let dir = egui::vec2(a.sin(), -a.cos());
    let half = (r.width() * dir.x.abs() + r.height() * dir.y.abs()) / 2.0;
    let stops = positions(&g.stops);
    for v in &mut mesh.vertices {
        let t = if half > 0.0 {
            ((v.pos - r.center()).dot(dir) / half + 1.0) / 2.0
        } else {
            0.0
        };
        let c = at(&stops, t.clamp(0.0, 1.0));
        let coverage = v.color.a() as f32 / 255.0;
        let [cr_, cg, cb, ca] = c.0;
        v.color =
            Color32::from_rgba_unmultiplied(cr_, cg, cb, (ca as f32 * coverage).round() as u8);
    }
    Shape::mesh(mesh)
}

/// Each stop's position, filled in where the sheet left it out: the first at
/// 0, the last at 1, the rest spaced evenly between known ones.
fn positions(stops: &[(Color, Option<f32>)]) -> Vec<(Color, f32)> {
    let n = stops.len();
    let mut at: Vec<Option<f32>> = stops.iter().map(|(_, p)| *p).collect();
    if at[0].is_none() {
        at[0] = Some(0.0);
    }
    if at[n - 1].is_none() {
        at[n - 1] = Some(1.0);
    }
    let mut i = 0;
    while i < n {
        if at[i].is_some() {
            i += 1;
            continue;
        }
        let start = i - 1;
        let end = (i..n).find(|&j| at[j].is_some()).unwrap_or(n - 1);
        let (a, b) = (at[start].unwrap_or(0.0), at[end].unwrap_or(1.0));
        for (k, slot) in at.iter_mut().enumerate().take(end).skip(i) {
            *slot = Some(a + (b - a) * (k - start) as f32 / (end - start) as f32);
        }
        i = end;
    }
    stops
        .iter()
        .zip(at)
        .map(|((c, _), p)| (*c, p.unwrap_or(0.0)))
        .collect()
}

/// The gradient's colour at `t`.
fn at(stops: &[(Color, f32)], t: f32) -> Color {
    if t <= stops[0].1 {
        return stops[0].0;
    }
    for w in stops.windows(2) {
        let ((c0, p0), (c1, p1)) = (w[0], w[1]);
        if t <= p1 {
            let f = if p1 > p0 { (t - p0) / (p1 - p0) } else { 1.0 };
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f).round() as u8;
            return Color([
                mix(c0.0[0], c1.0[0]),
                mix(c0.0[1], c1.0[1]),
                mix(c0.0[2], c1.0[2]),
                mix(c0.0[3], c1.0[3]),
            ]);
        }
    }
    stops[stops.len() - 1].0
}

/// Where an image goes in its box, and which part of it, for `object-fit`.
fn fitted(r: egui::Rect, w: f32, h: f32, fit: st::ObjectFit) -> (egui::Rect, egui::Rect) {
    let full = egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    if w <= 0.0 || h <= 0.0 {
        return (r, full);
    }
    let scale_x = r.width() / w;
    let scale_y = r.height() / h;
    match fit {
        st::ObjectFit::Fill => (r, full),
        st::ObjectFit::Contain | st::ObjectFit::ScaleDown => {
            let mut s = scale_x.min(scale_y);
            if fit == st::ObjectFit::ScaleDown {
                s = s.min(1.0);
            }
            (
                egui::Rect::from_center_size(r.center(), egui::vec2(w * s, h * s)),
                full,
            )
        }
        st::ObjectFit::Cover => {
            let s = scale_x.max(scale_y);
            let (vw, vh) = (r.width() / (w * s), r.height() / (h * s));
            let uv = egui::Rect::from_center_size(Pos2::new(0.5, 0.5), egui::vec2(vw, vh));
            (r, uv)
        }
        st::ObjectFit::None => {
            let shown = egui::Rect::from_center_size(r.center(), egui::vec2(w, h));
            let visible = shown.intersect(r);
            let uv = egui::Rect::from_min_max(
                Pos2::new(
                    (visible.left() - shown.left()) / w,
                    (visible.top() - shown.top()) / h,
                ),
                Pos2::new(
                    (visible.right() - shown.left()) / w,
                    (visible.bottom() - shown.top()) / h,
                ),
            );
            (visible, uv)
        }
    }
}
