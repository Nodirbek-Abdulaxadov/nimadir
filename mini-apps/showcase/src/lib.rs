//! `showcase` — the element tree in full (ADR-4): a page an app developer
//! would write with HTML and CSS, drawn natively by the shell.
//!
//! Four tabs, each showing a side of it: **Layout** (a responsive grid of
//! cards, an absolutely placed ribbon, a strip that scrolls sideways, a
//! sidebar that folds under `@media`), **Forms** (every control, driving a
//! live preview — a range sets its text size, radios set a CSS variable, a
//! checkbox toggles a class, submitting shows a toast), **Type** (headings,
//! inline styles in one paragraph, lists, quotes, code, an ellipsis) and
//! **Theme** (the shell's colours as variables, button styles, a counter).
//!
//! It is built once from markup and a stylesheet; one listener on the root
//! hears everything, and `update` edits only what changed.

use std::cell::RefCell;

wit_bindgen::generate!({
    world: "mini-app",
    path: "../../wit",
});

use nimadir::shell::dom::{self, Event};
use nimadir::shell::host_api::{image_load, log, now_millis, request_frame};

const STYLE: &str = r#"
.app { --accent: #1a73e8; max-width: 960px; margin: 0 auto;
       padding: 0 16px 32px; display: flex; flex-direction: column; gap: 20px; }
.hero { padding: 36px 24px 28px; border-radius: 0 0 24px 24px; color: #fff;
        background: linear-gradient(135deg, var(--accent), #a142f4);
        display: flex; flex-direction: column; gap: 10px;
        box-shadow: 0 8px 24px rgba(0, 0, 0, 0.25); }
.hero h1 { margin: 0; font-size: 34px; letter-spacing: -0.5px; }
.hero p { margin: 0; opacity: 0.9; max-width: 560px; }
.pills { display: flex; flex-wrap: wrap; gap: 8px; }
.pill { padding: 4px 10px; border-radius: 99px; font-size: 12px; font-weight: 600;
        background: rgba(255, 255, 255, 0.2); }

.tabs { display: flex; gap: 6px; padding: 4px; border-radius: 12px; overflow-x: auto;
        background: var(--nimadir-surface); }
.tab { border: none; background: transparent; padding: 8px 16px; border-radius: 9px;
       font-weight: 600; color: var(--nimadir-muted); }
.tab:hover { color: var(--nimadir-text); }
.tab.on { background: var(--nimadir-bg); color: var(--nimadir-text);
          box-shadow: 0 1px 4px rgba(0, 0, 0, 0.2); }

.panel { display: flex; flex-direction: column; gap: 14px; position: relative; }
.panel h2 { margin: 0; }
.panel h3 { margin: 4px 0 0; }
.lead { margin: 0; color: var(--nimadir-muted); }

.cards { display: grid; gap: 14px; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); }
.card { position: relative; padding: 18px; border-radius: 16px; display: flex;
        flex-direction: column; gap: 8px; background: var(--nimadir-surface);
        border: 1px solid var(--nimadir-border); box-shadow: 0 2px 8px rgba(0, 0, 0, 0.08); }
.card:hover { border-color: var(--accent); }
.card h3, .card p { margin: 0; }
.card p { color: var(--nimadir-muted); font-size: 13px; }
.ribbon { position: absolute; top: 12px; right: 12px; padding: 2px 8px; border-radius: 99px;
          background: #ea4335; color: #fff; font-size: 11px; font-weight: 700; }
.icon { width: 40px; height: 40px; border-radius: 12px; display: flex; align-items: center;
        justify-content: center; color: #fff; font-weight: 700; font-size: 18px; }
.meter { display: flex; align-items: center; gap: 8px; font-size: 12px; }
.meter progress { flex: 1; color: var(--accent); background: var(--nimadir-border); }

.strip { display: flex; gap: 8px; overflow-x: auto; padding-bottom: 6px; }
.chip { flex: none; padding: 6px 14px; border-radius: 99px; white-space: nowrap;
        border: 1px solid var(--nimadir-border); }
.chip:hover { background: var(--nimadir-accent-wash); }

.split { display: grid; grid-template-columns: 180px 1fr; gap: 16px; }
.side { display: flex; flex-direction: column; gap: 4px; padding: 10px; border-radius: 12px;
        background: var(--nimadir-surface); }
.side a { padding: 6px 10px; border-radius: 8px; text-decoration: none; color: var(--nimadir-text); }
.side a.on { background: var(--nimadir-accent-wash); color: var(--accent); font-weight: 600; }
.scrollbox { height: 150px; overflow: auto; padding: 12px; border-radius: 12px;
             border: 1px solid var(--nimadir-border); }
.scrollbox p { margin: 0 0 10px; }

.formgrid { display: grid; grid-template-columns: 1fr 1fr; gap: 24px; }
.form { display: flex; flex-direction: column; gap: 8px; position: relative; }
.form label { font-size: 13px; font-weight: 600; color: var(--nimadir-muted); }
.form label.check, .form label.swatch { font-weight: 400; color: var(--nimadir-text); }
.accents { display: flex; flex-wrap: wrap; gap: 14px; }
.primary { background: var(--accent); color: #fff; border: none; }
.form .primary { align-self: flex-start; margin-top: 6px; }
.preview { align-self: start; padding: 22px; border-radius: 18px; border: 2px solid var(--accent);
           background: var(--nimadir-surface); display: flex; flex-direction: column;
           align-items: center; gap: 6px; text-align: center; }
.preview.square { border-radius: 0; }
.avatar { width: 72px; height: 72px; object-fit: contain; }
#p-name { margin: 4px 0 0; color: var(--accent); }
#p-country { margin: 0; font-size: 12px; text-transform: uppercase; letter-spacing: 1.5px;
             color: var(--nimadir-muted); }
#p-bio { margin: 0; }
.toast { position: absolute; bottom: 0; left: 96px; padding: 8px 14px; border-radius: 10px;
         background: #34a853; color: #fff; font-weight: 600;
         box-shadow: 0 6px 16px rgba(0, 0, 0, 0.3); }

.caps { text-transform: uppercase; letter-spacing: 2px; font-size: 12px; color: var(--nimadir-muted); }
.clip { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 320px;
        padding: 8px 12px; border-radius: 8px; border: 1px dashed var(--nimadir-border); }
pre { padding: 12px; border-radius: 10px; background: var(--nimadir-surface); }

.swatches { display: grid; gap: 8px; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); }
.sw { display: flex; align-items: center; gap: 10px; padding: 8px; border-radius: 10px;
      border: 1px solid var(--nimadir-border); font-size: 12px; }
.dot { width: 28px; height: 28px; border-radius: 50%; border: 1px solid var(--nimadir-border); }
.buttons { display: flex; flex-wrap: wrap; gap: 8px; }
.ghost { background: transparent; border-color: var(--accent); color: var(--accent); }
.danger { background: var(--nimadir-danger); color: #fff; border: none; }
.counter { display: flex; align-items: center; gap: 12px; }
.counter span { font-size: 28px; font-weight: 700; min-width: 56px; text-align: center; }

@media (max-width: 640px) {
  .split, .formgrid { grid-template-columns: 1fr; }
  .hero h1 { font-size: 28px; }
}
"#;

const MARKUP: &str = r##"<div class="app">
<header class="hero">
  <h1>Showcase</h1>
  <p>A page written the way a web page is — elements, classes and a stylesheet — and drawn
  by the shell itself, natively, with no browser behind it.</p>
  <div class="pills"><span class="pill">flex</span><span class="pill">grid</span><span class="pill">calc()</span><span class="pill">var()</span><span class="pill">@media</span><span class="pill">forms</span></div>
</header>
<nav class="tabs">
  <button class="tab on" data-tab="layout">Layout</button><button class="tab" data-tab="forms">Forms</button><button class="tab" data-tab="type">Type</button><button class="tab" data-tab="theme">Theme</button>
</nav>

<section class="panel" id="layout">
  <h2>Layout</h2>
  <p class="lead">Grid, flex, absolute positioning and scrolling, laid out by the shell.</p>
  <div class="cards">
    <article class="card"><span class="ribbon">NEW</span><div class="icon" style="background: #1a73e8">G</div><h3>Grid</h3><p>repeat(auto-fill, minmax(200px, 1fr)): as many columns as fit.</p><div class="meter"><progress value="0.8"></progress><span>80%</span></div></article>
    <article class="card"><div class="icon" style="background: #34a853">F</div><h3>Flex</h3><p>Rows and columns that grow, shrink and wrap.</p><div class="meter"><progress value="0.6"></progress><span>60%</span></div></article>
    <article class="card"><div class="icon" style="background: #f9ab00">P</div><h3>Position</h3><p>The ribbon is absolute, placed from its card.</p><div class="meter"><progress value="0.45"></progress><span>45%</span></div></article>
    <article class="card"><div class="icon" style="background: #a142f4">C</div><h3>calc()</h3><p>Widths like calc(100% - 32px), resolved at layout.</p><div class="meter"><progress value="0.9"></progress><span>90%</span></div></article>
  </div>
  <h3>A strip that scrolls sideways</h3>
  <div class="strip"><span class="chip">Tashkent</span><span class="chip">Samarkand</span><span class="chip">Bukhara</span><span class="chip">Khiva</span><span class="chip">Fergana</span><span class="chip">Namangan</span><span class="chip">Andijan</span><span class="chip">Nukus</span><span class="chip">Termez</span><span class="chip">Navoi</span><span class="chip">Jizzakh</span><span class="chip">Qarshi</span></div>
  <h3>A sidebar that folds under a narrow screen</h3>
  <div class="split">
    <aside class="side"><a class="on" data-pick="1">Overview</a><a data-pick="2">Details</a><a data-pick="3">History</a></aside>
    <div class="scrollbox" id="box">
      <p><b>Overview.</b> This box scrolls when its content is taller than it: by the wheel on a desktop, by a finger on a phone.</p>
      <p>Its height is fixed by the stylesheet; what is inside is clipped to it.</p>
      <p>Below 640 pixels the sidebar moves above it, by @media (max-width: 640px).</p>
      <p>Nothing here is a web view: the shell parsed the markup, cascaded the styles, laid the boxes out with taffy and painted them.</p>
      <p>Pick another item on the left: the app moves a class, and only that repaints.</p>
    </div>
  </div>
</section>

<section class="panel" id="forms" hidden>
  <h2>Forms</h2>
  <p class="lead">Every control drives the card on the right as you use it.</p>
  <div class="formgrid">
    <form class="form" id="profile">
      <label for="name">Name</label><input id="name" placeholder="Ada Lovelace" maxlength="40">
      <label for="country">Country</label><select id="country"><option>Uzbekistan</option><option>Kazakhstan</option><option>Turkey</option><option>Japan</option></select>
      <label for="size">Text size: <span id="size-out">15px</span></label><input type="range" id="size" min="12" max="24" step="1" value="15">
      <label>Accent</label>
      <div class="accents"><label class="swatch"><input type="radio" name="accent" value="#1a73e8" checked> Blue</label><label class="swatch"><input type="radio" name="accent" value="#188038"> Green</label><label class="swatch"><input type="radio" name="accent" value="#9334e6"> Purple</label><label class="swatch"><input type="radio" name="accent" value="#e8710a"> Orange</label></div>
      <label class="check"><input type="checkbox" id="round" checked> Rounded corners</label>
      <label for="bio">Bio</label><textarea id="bio" placeholder="A line about you"></textarea>
      <button class="primary">Save</button>
      <div class="toast" id="toast" hidden>Saved</div>
    </form>
    <div class="preview" id="preview">
      <img id="avatar" class="avatar" alt="Avatar">
      <h3 id="p-name">Ada Lovelace</h3>
      <p id="p-country">Uzbekistan</p>
      <p id="p-bio">A line about you</p>
    </div>
  </div>
</section>

<section class="panel" id="type" hidden>
  <h1>Heading one</h1>
  <h2>Heading two</h2>
  <h3>Heading three</h3>
  <p>One paragraph, wrapping as a whole: <b>bold</b>, <i>italic</i>, <u>underlined</u>, <s>struck through</s>, <code>code</code>, <mark>marked</mark>, <small>small</small> and <a href="gallery">a link to the gallery</a> — a tap on it lands on the link.</p>
  <p class="caps">Letter-spaced capitals</p>
  <blockquote>The page is a tree of elements the app builds once and edits where it changes.</blockquote>
  <ul><li>Lists keep their bullets</li><li>and their indent</li><li>from the shell's own sheet</li></ul>
  <ol><li>Numbered,</li><li>counted by the shell,</li><li>not by the app</li></ol>
  <pre>fn update() {
    for e in dom::events() {
        handle(&amp;e);
    }
}</pre>
  <p class="clip">A line far too long for its box, cut with an ellipsis rather than wrapped onto the next.</p>
</section>

<section class="panel" id="theme" hidden>
  <h2>Theme</h2>
  <p class="lead">The shell's colours, as CSS variables: switch the system to dark and they follow.</p>
  <div class="swatches">
    <div class="sw"><span class="dot" style="background: var(--nimadir-bg)"></span><code>--nimadir-bg</code></div>
    <div class="sw"><span class="dot" style="background: var(--nimadir-surface)"></span><code>--nimadir-surface</code></div>
    <div class="sw"><span class="dot" style="background: var(--nimadir-border)"></span><code>--nimadir-border</code></div>
    <div class="sw"><span class="dot" style="background: var(--nimadir-text)"></span><code>--nimadir-text</code></div>
    <div class="sw"><span class="dot" style="background: var(--nimadir-muted)"></span><code>--nimadir-muted</code></div>
    <div class="sw"><span class="dot" style="background: var(--nimadir-accent)"></span><code>--nimadir-accent</code></div>
    <div class="sw"><span class="dot" style="background: var(--nimadir-accent-wash)"></span><code>--nimadir-accent-wash</code></div>
    <div class="sw"><span class="dot" style="background: var(--nimadir-danger)"></span><code>--nimadir-danger</code></div>
  </div>
  <div class="buttons"><button class="primary">Primary</button><button>Default</button><button class="ghost">Ghost</button><button class="danger">Danger</button><button disabled>Disabled</button></div>
  <div class="counter"><button id="minus">−</button><span id="count">0</span><button id="plus">+</button></div>
</section>
</div>"##;

const TABS: [&str; 4] = ["layout", "forms", "type", "theme"];

/// What the page keeps of its own nodes, found once by selector.
struct Page {
    app: u32,
    tabs: Vec<u32>,
    panels: Vec<u32>,
    picks: Vec<u32>,
    preview: u32,
    name: u32,
    country: u32,
    bio: u32,
    size_out: u32,
    toast: u32,
    count: u32,
    taps: i32,
    /// When the toast goes, in the shell's milliseconds.
    toast_until: Option<i64>,
}

thread_local! {
    static PAGE: RefCell<Option<Page>> = const { RefCell::new(None) };
}

fn one(selector: &str) -> u32 {
    dom::query_all(dom::root(), selector)
        .first()
        .copied()
        .unwrap_or(0)
}

struct Showcase;

impl Guest for Showcase {
    fn init() {
        dom::add_stylesheet(STYLE);
        if let Err(why) = dom::set_markup(dom::root(), MARKUP) {
            log(&format!("showcase: the page did not build: {why}"));
            return;
        }
        match image_load(include_bytes!("../assets/avatar.png")) {
            Ok(handle) => dom::set_attribute(one("#avatar"), "src", &handle.to_string()),
            Err(why) => log(&format!("showcase: no avatar: {why}")),
        }
        let root = dom::root();
        for kind in ["click", "input", "change", "submit"] {
            dom::listen(root, kind);
        }
        let page = Page {
            app: one(".app"),
            tabs: dom::query_all(root, ".tab"),
            panels: TABS.iter().map(|t| one(&format!("#{t}"))).collect(),
            picks: dom::query_all(root, ".side a"),
            preview: one("#preview"),
            name: one("#p-name"),
            country: one("#p-country"),
            bio: one("#p-bio"),
            size_out: one("#size-out"),
            toast: one("#toast"),
            count: one("#count"),
            taps: 0,
            toast_until: None,
        };
        PAGE.with(|p| *p.borrow_mut() = Some(page));
    }

    fn update() {
        let events = dom::events();
        PAGE.with(|p| {
            let mut p = p.borrow_mut();
            let Some(page) = p.as_mut() else { return };
            for e in &events {
                page.handle(e);
            }
            page.tick();
        });
    }
}

/// The nearest of `node` and its ancestors with attribute `name`, and its value.
fn find(mut node: u32, name: &str) -> Option<(u32, String)> {
    loop {
        if let Some(v) = dom::get_attribute(node, name) {
            return Some((node, v));
        }
        node = dom::parent(node)?;
    }
}

fn id_of(node: u32) -> String {
    dom::get_attribute(node, "id").unwrap_or_default()
}

impl Page {
    fn handle(&mut self, e: &Event) {
        match e.kind.as_str() {
            "click" => self.click(e.target),
            "input" => self.input(e.target, &e.value),
            "change" => self.change(e.target, &e.value),
            "submit" => self.saved(),
            _ => {}
        }
    }

    fn click(&mut self, target: u32) {
        if let Some((_, tab)) = find(target, "data-tab") {
            for (i, name) in TABS.iter().enumerate() {
                let on = *name == tab;
                dom::toggle_class(self.tabs[i], "on", on);
                if on {
                    dom::remove_attribute(self.panels[i], "hidden");
                } else {
                    dom::set_attribute(self.panels[i], "hidden", "");
                }
            }
            return;
        }
        if let Some((picked, _)) = find(target, "data-pick") {
            for &a in &self.picks {
                dom::toggle_class(a, "on", a == picked);
            }
            return;
        }
        match id_of(target).as_str() {
            "plus" => self.count_by(1),
            "minus" => self.count_by(-1),
            _ => {}
        }
    }

    fn input(&mut self, target: u32, value: &str) {
        match id_of(target).as_str() {
            "name" => dom::set_text(
                self.name,
                if value.trim().is_empty() {
                    "Ada Lovelace"
                } else {
                    value
                },
            ),
            "bio" => dom::set_text(
                self.bio,
                if value.trim().is_empty() {
                    "A line about you"
                } else {
                    value
                },
            ),
            "country" => dom::set_text(self.country, &dom::value(target)),
            "size" => {
                dom::set_style(self.preview, "font-size", &format!("{value}px"));
                dom::set_text(self.size_out, &format!("{value}px"));
            }
            _ => {}
        }
    }

    fn change(&mut self, target: u32, value: &str) {
        if dom::get_attribute(target, "name").as_deref() == Some("accent") && value == "true" {
            // A custom property set inline on the page: everything that
            // says var(--accent) follows it.
            if let Some(color) = dom::get_attribute(target, "value") {
                dom::set_style(self.app, "--accent", &color);
            }
        } else if id_of(target) == "round" {
            dom::toggle_class(self.preview, "square", value != "true");
        }
    }

    fn saved(&mut self) {
        dom::remove_attribute(self.toast, "hidden");
        self.toast_until = Some(now_millis() + 1800);
        request_frame(2000);
    }

    /// The toast goes once its time is up.
    fn tick(&mut self) {
        if let Some(until) = self.toast_until {
            if now_millis() >= until {
                dom::set_attribute(self.toast, "hidden", "");
                self.toast_until = None;
            } else {
                request_frame((until - now_millis()).max(16) as u32);
            }
        }
    }

    fn count_by(&mut self, by: i32) {
        self.taps += by;
        dom::set_text(self.count, &self.taps.to_string());
    }
}

export!(Showcase);
