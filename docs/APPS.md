# Writing a nimadir app

A nimadir app — a *mini-app* — is a WebAssembly component. The shell loads it,
runs it in a sandbox, and draws the UI it describes, natively: no HTML, no CSS,
no JavaScript. It can be written in Rust, C# or Python, and it reaches nothing
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

For C# you need the .NET 10 SDK; for Python, `pip install componentize-py`.

## How an app draws

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
