# ADR-4: A native element tree, styled with CSS

**Status:** Accepted (2026-10-21). The plan is `docs/ROADMAP.md`, *Road to
1.1*; the contract is `interface dom` in `wit/world.wit` (`nimadir:shell@1.1.0`).

## Context

Until now an app *described* its page each frame — `ui-label`, `ui-button`,
`ui-begin("row")` — and the host decided how every thing looked: no
coordinates, no sizes, no colours (`ARCHITECTURE.md` §4.6). That kept the
renderer swappable and every app consistent, and it caps what an app can be.
A designer's layout, a brand's colours, a card that is not the host's card, a
grid of uneven tracks, an app that does not look like every other app: none of
it can be said. What people know how to say it in is HTML and CSS.

The owner's direction (2026-10-21): the WIT should reach **DOM + CSS level**.
Three ways to get there:

- **Style the immediate mode**: `ui-style("padding", "8px")` before a widget.
  Small, but a frame-by-frame description has nowhere to keep a node's
  identity, so no stylesheets, no `:hover`, no transitions, no diffs — every
  frame resends everything.
- **Embed a web engine**: a webview per app. Everything for free, and the
  thing this project exists not to be: a browser, with its size, its attack
  surface and its JavaScript.
- **A native element tree**: the app builds and edits a tree of elements held
  by the host; the host styles it with a CSS cascade of its own, lays it out
  with a layout engine of its own, and paints the result natively.

## Decision

**A native element tree** — a DOM that is not a browser's:

- **Retained.** The app creates elements and text, nests them, sets attributes
  and styles, and edits only what changes. Only the edits cross the boundary.
  Nodes are numbers (`type node = u32`), not resources: a node in the tree
  outlives the guest's handle to it, and every guest language handles a
  number.
- **Styled with CSS, parsed by the host.** Stylesheets with selectors (type,
  class, id, descendant, child, the usual pseudo-classes), a cascade with
  specificity, inheritance, custom properties and `var()`, and `@media` on
  colour scheme and width. Inline `style`. A built-in user-agent stylesheet
  gives every element a sensible look that follows the system's light or dark
  theme, through variables the host defines (`--nimadir-*`).
- **Laid out by the host**: block, flexbox, grid, absolute positioning and
  floats, by [taffy](https://github.com/DioxusLabs/taffy), pure Rust. Text is
  measured by the renderer through a trait, so layout stays in the core.
- **Painted natively.** The core turns the laid-out tree into a display list —
  boxes, borders, shadows, text runs, images, controls — which `gui.rs` paints
  with egui and the headless runner prints. A renderer swap still touches the
  renderer alone.
- **Events, polled.** An app listens on a node for a kind (`click`, `input`,
  `submit`, `focus`, …); the host queues what happened and the app drains the
  queue with `events()` in its next `update`. An event goes to the nearest
  listening ancestor, carrying both the node hit and the one listening.
- **Markup, optionally.** `set-markup` parses an HTML-like snippet into
  children, and `query-all` finds nodes by selector — the same matcher the
  cascade uses.
- **Additive.** A new interface, `dom`, imported by the same world; the
  package moves to `1.1.0`. Apps built against `1.0` link unchanged. The tree
  under `root()` is the page; an app that never touches it draws exactly what
  it drew before, and immediate-mode widgets drawn in the same frame follow
  the tree.

Names are strings throughout — tags, attributes, properties, event kinds — as
`ui-begin`'s kind already is: a newer name on an older host is ignored or
drawn as a `div`, never a broken link.

## Consequences

- **Invariant 6 changes wording, not intent.** "No web stack in the mini-app
  path" becomes "no web *engine*": no JavaScript, no browser, no webview.
  Markup and CSS are text the host parses into its own tree and draws itself.
  §4.6's rule — the host decides how things look — now holds for the
  user-agent stylesheet and for `ui-*`, not for an app that styles its own
  elements.
- **Bounds move from frames to the document.** A tree persists, so it is
  capped as a whole: 65 536 nodes, 32 MiB of text, attributes and styles,
  256 levels deep, and the stylesheets' rules. Every call is also charged to
  the frame as before.
- **Consistency is the app's to keep** once it styles itself. The user-agent
  stylesheet and the `--nimadir-*` variables make following the system the
  easy path, not the only one.
- **Not in 1.1**: JavaScript (the app's own language is its script),
  `<canvas>`, transforms, animations and transitions, `calc()` beyond what
  taffy resolves, web fonts. Each can be a later addition to the same
  interface.
- **Accessibility** has to be fed from the tree to the platform; egui's own
  widgets were doing it for `ui-*`. Tracked as a gap.
