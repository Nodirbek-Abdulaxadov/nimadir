# Writing a nimadir app

A nimadir app — a *mini-app* — is a WebAssembly component. The shell loads it,
runs it in a sandbox, and draws the UI it describes, natively: no browser and
no JavaScript behind it. It can be written in Rust, C# or Python, and it reaches nothing
outside its sandbox except what one file grants it: `wit/world.wit`.

This guide is for writing apps. How the shell itself is built is in
`docs/ARCHITECTURE.md`.

## The first one

You need the `nimadir` tool and the shell, `nimadir-shell`
(`cargo build --release -p nimadir-cli -p host` in this repository puts both in
`target/release/`) and, for Rust,
`rustup target add wasm32-wasip2`.

```bash
nimadir new hello-world          # --lang csharp or --lang python for the others
cd hello-world
nimadir run                      # builds it and opens it in the shell
```

`nimadir new` writes a project that needs nothing from this repository:

| File | What it is |
|---|---|
| `src/lib.rs` (or `Program.cs`, `app.py`) | The app |
| `manifest.json` | What the app says about itself — name, version, permissions |
| `wit/world.wit` | Its own copy of the contract: everything it can ask the shell for |

`nimadir build` makes `hello-world.component.wasm`, with the manifest packed
inside. That one file is the app: open it from the shell's address bar, list it
in a registry, or hand it to someone.

For C# you need the .NET 10 SDK, on Linux or Windows: NativeAOT-LLVM, which
compiles it, has no macOS build. For Python, `pip install componentize-py`.

There are two ways for an app to draw. The Rust template builds **a page of
elements** styled with CSS (below, `1.1`); the C# and Python templates
*describe* their UI each frame with `ui-*` calls. An app may use either, or
both.

## How an app draws, frame by frame

Every frame the shell calls the app's `update`, and the app *describes* its UI
by calling functions — `ui-label`, `ui-button`, `ui-text-edit`, and so on. It
never says where anything goes; the shell decides that, differently on a phone
than in a window.

```rust
fn update() {
    ui_heading("Notes", 1);
    ui_begin("section");
    NAME.set(ui_text_edit(&NAME.get()));    // the value goes out, the edited one comes back
    if ui_submitted() {                     // Enter, or the keyboard's action key
        save();
    }
    ui_end();
    if ui_button("Clear") {                 // true in the frame after the tap
        clear();
    }
}
```

The app owns its state; the shell owns the widgets. A tap or an edit reaches the
app in the frame *after* it happened — that one-frame lag is how immediate mode
works, and it is not something to work around.

The shell draws only when something happens: a tap, a key, a load. An app that
shows time passing asks for its next frame: `request-frame(1000)`.

### What there is to draw

| Function | What it is |
|---|---|
| `ui-label`, `ui-heading` (level 1–3) | Text |
| `ui-button`, `ui-tile`, `ui-checkbox` | Things to tap |
| `ui-text-edit`, `ui-search-field`, `ui-choice` | Things that hold a value |
| `ui-begin(kind)` … `ui-end` | Groups: `"row"`, `"column"`, `"section"`, `"scroll"` |
| `ui-separator`, `ui-progress` | A line; progress from 0.0 to 1.0 |
| `image-load` then `ui-image` | A PNG or JPEG, handed over once, drawn by number |

`wit/world.wit` documents each one; that file is the reference.

## A page of elements (`1.1`)

Since `nimadir:shell@1.1.0` an app can build its page the way a web page is
built — a tree of elements, styled with CSS — and the shell lays it out and
draws it natively, with no browser behind it (ADR-4). `nimadir new` writes a
Rust app this way. The page is built once and edited where it changes; what
the user does arrives as events.

```rust
use nimadir::shell::dom;

const PAGE: &str = r#"<main class="app">
  <h1>Notes</h1>
  <input id="note" placeholder="Write something">
  <ul id="list"></ul>
</main>"#;

const STYLE: &str = ".app { max-width: 480px; margin: 0 auto; padding: 24px 16px;
                            display: flex; flex-direction: column; gap: 12px; }";

fn init() {
    dom::add_stylesheet(STYLE);
    let _ = dom::set_markup(dom::root(), PAGE);
    dom::listen(dom::root(), "submit");          // Enter in the field
}

fn update() {
    for e in dom::events() {
        if e.kind == "submit" {
            let list = dom::query_all(dom::root(), "#list")[0];
            let li = dom::create_element("li");
            dom::set_text(li, &e.value);             // the field's text
            dom::append_child(list, li);
            dom::set_attribute(e.target, "value", ""); // clear the field
        }
    }
}
```

### Building and changing the page

| | |
|---|---|
| `set-markup(node, html)` | Replace a node's children with markup: elements, attributes, text, entities. Fails, changing nothing, on markup it cannot read. No scripts. |
| `create-element`, `create-text`, `append-child`, `insert-before`, `remove`, `release` | Build by hand. `remove` keeps a node to put back; `release` forgets it and what is under it. |
| `query-all(scope, selector)` | Find nodes by CSS selector, in document order. |
| `set-text`, `set-attribute`, `toggle-class`, `set-style` | Change what changed. `set-style(n, "color", "")` removes one. |
| `value(node)`, `focus(node)`, `bounds(node)` | A control's value as the user left it; give it the keyboard; where it was drawn. |

Nodes are numbers; one that is gone is ignored. Text that comes from
outside — a registry, a server — goes in with `set-text`, or escaped
(`&amp;`, `&lt;`) when it goes into markup.

### Events

`listen(node, kind)` asks for `click`, `input`, `change`, `submit`, `focus`,
`blur`, `pointerdown` or `pointerup` on a node **and what is under it**: one
listener on a list hears every item. Each event names `target`, the node it
happened on, and `current`, the listener. `value` carries a field's text,
`"true"`/`"false"` for a checkbox, a select's option value, a range's number.
The shell calls `update` when there are events; read them with `events()`.

A tap on a link (`<a href="counter">`) is followed like `open-app`; on a
checkbox or radio it flips it; on a `label` it acts for its control; a
`button` in a `form` sends `submit` to the form. Nothing inside a `disabled`
element answers.

### What CSS there is

- **Selectors**: type, `.class`, `#id`, `[attr]` with `= ~= |= ^= $= *=`,
  descendant, `>`, `+`, `~`, and `:hover :active :focus :focus-within
  :disabled :enabled :checked :root :empty :first-child :last-child
  :only-child :nth-child() :nth-last-child() :not() :placeholder-shown`. No
  pseudo-elements (`::before`): a rule with one is skipped.
- **The cascade** as on the web: specificity, `!important`, inline `style`,
  inheritance, `inherit`/`initial`/`unset`, custom properties and `var()`.
- **Values**: `px em rem vw vh vmin vmax pt` and friends, `%`, `calc()`,
  `min() max() clamp()`, hex, `rgb()`, `hsl()`, named colours.
- **Layout**: `display: block | flex | grid | inline | inline-block | none |
  contents`, flex and grid in full (`repeat()`, `minmax()`, `fr`, spans,
  `auto-fill`), `position: relative | absolute`, `float`, `gap`,
  `box-sizing`, `aspect-ratio`, `overflow: hidden | auto | scroll` (which
  scrolls, by wheel and by touch), `z-index`.
- **Look**: backgrounds and `linear-gradient()`, borders and `border-radius`,
  `box-shadow` (one), `opacity`, `visibility`, `font-size`, `font-weight`
  (600 and up is bold), `font-style`, `font-family: monospace`,
  `line-height`, `text-align`, `text-decoration`, `text-transform`,
  `letter-spacing`, `white-space`, `text-overflow: ellipsis`, `cursor`,
  `list-style`, `object-fit`.
- **`@media`** on `min-`/`max-width` and `-height`, `orientation`,
  `prefers-color-scheme` and `hover`.
- What a sheet cannot say is skipped and logged (`css: skipped …`).

Text inside a paragraph flows as one block: an inline element (`span`, `b`,
`a`, `mark`) gives its text a look and a background, not a box of its own. No transforms, transitions,
animations, web fonts or `<canvas>` yet.

**The shell's colours** are variables every element can use, and they follow
the system's light or dark theme — an app that uses them does too:
`--nimadir-bg`, `--nimadir-surface`, `--nimadir-border`, `--nimadir-text`,
`--nimadir-muted`, `--nimadir-accent`, `--nimadir-accent-text`,
`--nimadir-accent-wash`, `--nimadir-danger`, `--nimadir-field`. Every element
also starts with the shell's own look — a button looks like a button — until
a sheet says otherwise.

### Controls

`input` (text, password, search, number, checkbox, radio, range), `textarea`,
`select` with `option`s, `button`, `label`, `progress`, `img` (its `src` is a
number from `image-load`, its `alt` kept), `a`. `disabled`, `readonly`,
`maxlength`, `placeholder`, `checked`, `value`, `min`/`max`/`step` mean what
they mean in HTML.

### Limits

A page holds at most 65 536 nodes, 32 MiB of text, attributes and styles,
and 256 levels; an app that goes past one is stopped, saying which. Events
not read pile up to 1024, then the oldest go.

### Testing a page without a screen

The headless shell prints the page as a tree and acts on it by **node
number**, which a node keeps from frame to frame:

```bash
nimadir-shell app.wasm --frames 3 --type "1:7=hello" --enter "1:7" --click "2:12"
nimadir-shell app.wasm --styles --boxes --viewport 360x640 --dark
nimadir-shell app.wasm --paint        # what the window would draw
```

`--styles` adds each element's computed style, `--boxes` its layout (with
stand-in fonts, so the same on every machine), `--paint` the display list.

An app may still draw `ui-*` widgets too: they follow the tree.

## What an app may do, and how it asks

Everything in the WIT is available to every app, with three exceptions that
depend on the manifest and on the user:

```json
{
  "name": "notes",
  "version": "1.0.0",
  "permissions": ["storage", { "network": ["https://api.example.com"] }]
}
```

- **Storage** (`storage-get`, `-set`, `-remove`, `-keys`): 1 MiB of strings.
  Kept between visits only for an app that is **signed** and lists
  `"storage"`; for any other app it ends when the app is left.
- **Network** (`fetch`, then `fetch-poll` in later frames): only to the origins
  listed under `"network"`, only over https, and only once the user has allowed
  it — the shell asks the first time. A signed app's *yes* is remembered.
  Redirects are not followed; a 3xx comes back to the app.
- **Opening another address** (`open-app`): an app the registry lists opens at
  once; anything else asks the user first.

Limits every app runs under, whatever it declares: about a second per frame,
512 MiB of memory, 16 MiB of output per frame, images up to 4096 pixels a side.
An app that goes over is stopped, and the shell says which limit it hit.

## Signing

A signature gives an app an identity — `<publisher>/<name>`, the publisher
derived from a key — which is what storage and remembered permissions are
kept under, and what a registry can require.

```bash
nimadir keygen ~/my-publisher.key        # once; keep it secret, back it up
nimadir build --key ~/my-publisher.key
nimadir inspect notes.component.wasm     # what it claims, and who signed it
```

A component changed after it was signed does not run. A lost key cannot be
recovered: updates under that identity become impossible.

## Testing without a screen

The shell runs headless, printing every frame's UI as text, with taps and
typing injected:

```bash
nimadir-shell notes.component.wasm --frames 3                       # just look
nimadir-shell notes.component.wasm --input "1:0=milk" --submit "2:0" --frames 4
nimadir-shell notes.component.wasm --script "1:0" --confirm --frames 4
```

`--script "frame:index"` taps; buttons, tiles and checkboxes share one index
space, counted in the order the app draws them. `--input "frame:index=text"`
sets a value; text fields, search fields and choices share the other.
`--confirm` says yes when the app asks for the network or to open an address.
`nimadir check app.component.wasm` says whether this shell will run it at all.

## Publishing

A registry is a JSON file listing apps, each by name, address and SHA-256:

```json
{ "apps": [ {
  "name": "notes",
  "source": "https://example.org/notes.component.wasm",
  "description": "Plain-text notes, kept on this device.",
  "sha256": "…",
  "publisher": "…"
} ] }
```

The shell refuses a component whose bytes do not match `sha256`, or that was
signed by anyone but `publisher`. `tools/publish_registry.py` builds one from
this repository's apps; any static web host can serve it. A shell is pointed at
a registry with `--registry <url>`, `NIMADIR_REGISTRY`, or an `apps.json` in its
configuration directory.

## When the shell is older than the app

The WIT only ever grows. An app built against a newer one that uses something
this shell lacks is refused with "this app needs a newer nimadir: it uses …" —
before it runs, never halfway through.
