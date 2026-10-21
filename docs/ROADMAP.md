# nimadir — road to beta

The MVP is done (M1–M9 in `README.md`). This plan takes the project from MVP to
a public beta. It is written against the code at `807765c` (2026-10-20) and,
like `ARCHITECTURE.md`, names greppable symbols rather than line numbers.

**Assumptions** — change one and the plan moves: two-week sprints, one developer
full time, eight sprints (about 16 weeks). Sizes are rough: **S** ≤ 1 day,
**M** 2–4 days, **L** a week or more. Item IDs (`S2-4`) are meant to become
issue titles.

---

## What "beta" means

Beta ships when all of these hold:

1. **Installable.** Release builds for Windows, macOS, Linux and Android run
   outside a checkout: the store ships with the binary, the registry comes from a
   hosted URL.
2. **Trusted downloads.** Every remote component is checked against a hash from a
   registry served over HTTPS before it is compiled.
3. **Real apps are possible.** A mini-app can keep data between runs, reach the
   network it declared, and lay out more than a vertical list — and the user
   decides which of those it gets.
4. **Outside developers.** Someone who has never cloned this repo can write, test
   headlessly and publish a mini-app in Rust (templates for C# and Python too)
   from the docs alone.
5. **A stable contract.** The WIT is frozen at `nimadir:shell@1.0.0-beta` and
   additive-only after; an app built for a newer host fails with a sentence, not a
   linker error.
6. **CI on every PR**, on all three desktop OSes plus an Android build, and the
   suite covers each new capability headlessly.
7. **Bounded.** No guest can exhaust host memory or CPU — including through
   host-side buffers and idle redraw, which today it can (`S1-2`, `S3-7`).

## Decisions needed before Sprint 1

| # | Decision | Recommendation | Blocks |
|---|---|---|---|
| D1 | Is Android in beta? | **Yes.** Invariant 8 (`ARCHITECTURE.md` §5) makes it an equal target; adding it after beta means retrofitting every feature built without it | S3 onward |
| D2 | Code signing | macOS: yes — needs a paid Apple Developer account; unsigned apps are blocked by Gatekeeper. Windows: an unsigned beta works, behind a SmartScreen warning | S8-1 |
| D3 | Registry hosting | GitHub Pages, filled by a CI job that builds, hashes and publishes the sample apps | S2-3 |
| D4 | Tracker | One GitHub milestone per sprint, one issue per item ID | — |

## Definition of done, for every item

- Covered by `run-tests.sh` through the headless harness — adding harness flags
  where the feature needs them (`--grant`, `--data-dir`, …).
- Works on desktop **and**, from Sprint 3, on Android.
- `docs/ARCHITECTURE.md` updated: the symbols in §3, a §4 entry if anything
  surprised.
- Respects the §5 invariants. Bending one takes an ADR in `docs/adr/` first.
- CI green.

---

## Sprint 1 — Foundations, and the riskiest unknown

**Goal:** CI guards every change from here on, the host-memory hole is closed,
and the Android question has an answer before anything is built on top of it.

| ID | Item | Size |
|---|---|---|
| S1-1 | **CI** on GitHub Actions: Linux (headless + `gui`), Windows, macOS; `cargo fmt --check`, `clippy -D warnings`, `./run-tests.sh` | M |
| S1-2 | **Bound host-side buffers.** `HostState::ui` and `logs` grow without limit — `ui_label` and `log` just `push` — so a guest looping for its one-second budget can allocate gigabytes *on the host*, where the 512 MiB guest cap does not reach. Cap commands and bytes per frame; going over is a trap named by `explain`, like the other limits. Fixture: a third button on `runaway`. | S |
| S1-3 | **Spike: Android.** eframe + Wasmtime run `counter` and `store` on an emulator; the soft keyboard reaches `ui-search-field`. Output: go/no-go, the packaging tool to use, and a list of blockers. | L |
| S1-4 | **ADR-1: app identity and manifest.** What an app's ID is (not its URL — URLs move), its version, requested permissions and minimum host version, and where that lives (a component custom section vs. the registry entry). Storage, permissions, integrity and updates all key off this. | M |
| S1-5 | **Release hygiene:** a `LICENSE` file (the README says MIT; there is no file), `CHANGELOG.md`, `SECURITY.md` — it runs untrusted code, so say how to report a hole | S |

**Ahead of plan (2026-10-20):** S3-5 (Back, minus Forward) and S3-7 (frame
scheduling, `request-frame`) are in, green in the suite; their check on the
device is still to run.

**Status (2026-10-20):** S1-2, S1-3 and S1-5 done. S1-1 written, and green
locally (fmt, clippy, the suite) — it has not run on GitHub yet. S1-4 is
accepted by default (`docs/adr/0001-app-identity-and-manifest.md`).
The Android spike's verdict is **GO**, with five blockers moved into Sprint 3
below (`docs/spikes/android.md`).

## Sprint 2 — Runs outside the repo

**Goal:** a release build started from any directory opens a working store, and
nothing remote is compiled unchecked.

| ID | Item | Size |
|---|---|---|
| S2-1 | **Ship the store with the binary** (embedded or in a resources dir). `Shell::go_home` finds it today only because `mini-apps/store/…` happens to exist relative to the working directory. | M |
| S2-2 | **Registry from a hosted URL + the user's config dir, not the CWD.** `main` calls `registry::load(Path::new("."))`, and `registry::builtin` holds relative paths: both only work inside a checkout. A dev checkout keeps today's `apps.json` behaviour. | M |
| S2-3 | **Hosted registry v1** (D3): `apps.json` + components, built, hashed and published by CI | M |
| S2-4 | **Integrity.** `AppEntry` gains a `sha256`; the loader verifies it before `Component::from_binary`; remote components only over HTTPS (`127.0.0.1` exempt, for the suite) | M |
| S2-5 | **Component cache** in the data dir: an app opened once opens offline after, still verified against its hash | M |
| S2-6 | **Newer app, older host:** a component importing something this host lacks fails with "needs nimadir ≥ X", not a link error | S |
| S2-7 | `--version`; a log file in the data dir; a panic hook that writes there | S |

New assertions: the binary started from an empty temp dir opens the store; a
component whose bytes do not match its hash is refused; `http://` to a
non-local host is refused.

**Status (2026-10-20): done** — all seven items, on branch `sprint-2`, with the
suite at 63 assertions. One step is the owner's: switch GitHub Pages on with
"GitHub Actions" as its source, so `registry.yml` can publish. Until then the
hosted URL answers 404 and a shell outside a checkout opens on its built-in
store alone.

## Sprint 3 — Android as an equal target

**Goal** (if D1 is yes): everything that works on desktop at the end of S2 works
on Android. From here on, an item is done only when it works on both.

| ID | Item | Size |
|---|---|---|
| S3-1 | **Build with `cargo-ndk` + a small Gradle project, on GameActivity.** The spike's `cargo-apk` cannot target the current API level or build an AAB, and NativeActivity rules out both IME text events and `accesskit`. APK and AAB built in CI. | L |
| S3-2 | Data, cache and config paths from the platform. Includes the compile cache: on Android `wasmtime::Cache::from_file(None)` finds no directory, so every open compiles from scratch. | S |
| S3-3 | Touch: nothing that needs hover, drag to scroll. (Done: insets and the side gutter — `android::SafeArea`; 48 dp controls — `install_theme`.) | M |
| S3-4 | IME text for `ui-text-edit` / `ui-search-field`. The keyboard already appears and key events arrive; winit 0.30 drops IME text events, so predictions and non-ASCII (Cyrillic, `oʻ`) depend on GameActivity text input. | M |
| S3-5 | **Navigation history** in `Shell` (back / forward). The system back button and a chrome Back button both drive it; back from the start page exits. | M |
| S3-6 | Lifecycle: no frames while paused, redraw on resume. Process death loses nothing, since leaving an app already destroys it (invariant 3). | M |
| S3-7 | **Frame scheduling.** `gui.rs` calls `request_repaint()` every frame unconditionally, so a still app keeps a core busy — a battery drain on a phone. Add `request-frame(after-ms)` to the WIT; the host redraws only on input, a pending load, or that request. | M |
| S3-8 | Webview on Android (wry supports it), or the `webview` feature declared desktop-only for beta — decided by S1-3 | M |
| S3-9 | Device smoke test over `adb`: open the store, open `counter`, tap | M |

**Status (2026-10-20), on branch `sprint-3`.** Done and green on the desktop:
S3-1 (Gradle + `cargo-ndk`, API 36, AAB — but still NativeActivity, see S3-4),
S3-2, S3-3, S3-5 (Back; no Forward yet), S3-7, S3-8 (webview desktop-only for
beta), S3-9 (`tools/android-smoke.sh`, written). **Waiting on a device:** the
smoke test itself, S3-6 (the redraw policy draws nothing unasked, and a guest's
`request-frame` now waits while the window is hidden; unmeasured on a phone), and every Android claim since the spike. **Open:**
S3-4 — winit 0.30 drops IME text events, so GameActivity would break typing;
it needs a winit upgrade or patch before predictions and non-ASCII can work.

## Sprint 4 — Identity, permissions, storage

**Goal:** an app can keep data between runs, and the user controls what each app
may do.

| ID | Item | Size |
|---|---|---|
| S4-1 | **Manifest** per ADR-1, read at load and shown in the store. Note: a WIT `record` cannot grow without breaking every guest built against the old shape — add an `app-details` function rather than widening `app-entry`. | M |
| S4-2 | **Capability policy.** Every capability belongs to a permission (labels and buttons need none). An ungranted call fails with a named reason; granting happens in the chrome, which a guest cannot reach (invariant 2). | L |
| S4-3 | **`open-app` policy:** registry apps open freely; any other address needs the user's confirmation in the chrome | S |
| S4-4 | **Storage capability:** per-app key-value on disk, keyed by app ID, with a quota, cleared on uninstall. ADR-2 first: how this squares with invariant 3 — state that outlives the `Store` lives on disk, owned by the host, never in host memory. | L |
| S4-5 | **Settings / My apps** in the chrome: installed apps, their permissions and storage use, clear data, uninstall | M |
| S4-6 | Harness flags `--grant` and `--data-dir`, so permission and storage paths are testable headlessly | S |

**Progress (2026-10-20), on branch `sprint-4`.** S4-1: manifests and
signatures are read, verified and enforced (`crates/manifest`,
`nimadir`, `Shell::identity`); showing them in the store is still to do.
S4-4: `storage-*` (ADR-2), kept for signed apps that ask, 1 MiB each.
S4-3: `open-app` to an unlisted address asks the user (chrome; `--confirm`).
S4-5: settings page (`nimadir:settings`, linked from the store) — what apps
keep and the caches, with clearing; drawn but not yet seen on a screen.
S4-6: `--kept`, `--clear`, `--clear-cache`, `--confirm`; `NIMADIR_HOME` is the
data directory.

**Sprint 6 progress (2026-10-20), on `sprint-6`:** S6-1 (ADR-3) and S6-2 —
`fetch`, declared and granted, with S4-2's grant machinery; S6-3 `locale`
(time zone still to do). S6-4: `rates`, checked against the real cbu.uz by
hand, declined in the suite. S6-5: the `fuzz` fixture, 12 000 awkward calls
without a panic.

**Tagged `v1.0.0-beta.1` (2026-10-21).** CI green on every job; every
installer built by `release.yml` into a draft release; the hosted registry
live on Pages. Private vulnerability reporting and D2 are still the owner's
(`docs/BETA-CHECKLIST.md`).

**On a device (2026-10-21), 119 assertions.** The smoke test passes on an
Android 16 phone (cold start ~0.3 s, 0 % CPU idle), and the store, `counter`
(storage kept across a restart), `gallery`, `rates` (a grant asked, then real
cbu.uz data), typing, settings and the update check were driven by hand. The
run found twelve bugs, all fixed (`CHANGELOG.md`), the worst three from one
cause: winit's one event loop per process (`ARCHITECTURE.md` §4.12). S3-6 is
measured: nothing is drawn unasked.

**Sprint 8 closed (2026-10-20), 117 assertions.** S8-7: the contract frozen at
`nimadir:shell@1.0.0`, versions at 1.0.0-beta.1, and `docs/BETA-CHECKLIST.md`
— four criteria met, three waiting on the owner (push for CI; Pages; a
device), so **not tagged**.

**Sprint 8 progress:** S8-1 installers: a `.deb` built and checked here (the
shell starts from it with the store inside), AppImage failing in linuxdeploy on
this machine, Windows and macOS untried until CI runs them; release workflow
written, signing from secrets (D2 still open for Apple). S8-6: the review,
`docs/SECURITY-REVIEW.md` — four findings fixed, residual risks listed. S8-5 Reload; S8-4 the window in Uzbek, Russian and
English (Android's locale read over JNI); S8-3 store cards labelled for screen
readers — Android accessibility waits on GameActivity, with S3-4. Images have
no alternative text yet: that needs a WIT function, after beta.

**Sprint 7 closed (2026-10-20), 109 assertions.** S7-1: each project vendors
the WIT (`nimadir new`) instead of fetching a package — nothing to publish,
nothing to resolve. S7-2: templates for Rust, C# and Python; an SDK crate of
helpers is left for after beta. S7-3: `wasm32-wasip2` adopted for new Rust
apps (one tool fewer, ~2.6× the size); the samples keep `componentize`.
S7-4: `nimadir new/build/run/check`. S7-5: C# and Python templates, built in
CI; the Go sample was cut. S7-6: `docs/APPS.md`.

**Sprint 6 closed (2026-10-20), 103 assertions.** Left: a time-zone
capability (S6-3's other half).

**Sprint 5 closed (2026-10-20), 90 assertions.** S5-4: images, by handle,
size-checked from the header. Earlier in the sprint: S5-1 groups (row, column,
section, scroll; nesting capped at 32), S5-2 checkbox, choice, separator,
progress, S5-3 submit — all green headlessly through the new `gallery`
sample. S5-5: U+02BB was missing from every bundled font; Noto Sans now
falls back for it. S5-6: light and dark palettes, following the system. S5-7: 500 apps cost
the boundary ~244 µs a frame in release on the desktop (`--timing`); the
phone and the renderer are unmeasured. The window draws them but nobody has looked yet.

**Sprint 4 closed (2026-10-20), 80 assertions.** Carried forward: S4-2's
general grant machinery moves into S6-2, where network access is the first
capability that needs a grant (storage and `open-app` are already
policed); S4-1's display of version and publisher in the store needs a new
WIT function and comes with Sprint 5's vocabulary.

## Sprint 5 — UI vocabulary for real apps

**Goal:** a guest can build a form, a list and a detail page — still saying what
a thing *is*, never where it goes (`ARCHITECTURE.md` §4.6). Each widget costs the
five-file bill from §4.6 plus a headless assertion, and a stateful one keys its
egui id off the guest index (§4.4).

| ID | Item | Size |
|---|---|---|
| S5-1 | **Structure:** `ui-begin(kind)` / `ui-end` for row, column, section and scroll — nesting, no coordinates; the backend decides sizes | L |
| S5-2 | Checkbox / toggle, a touch-friendly choice list, separator, progress | M |
| S5-3 | **Submit** on text fields: Enter on desktop, the IME action key on Android. Closes "there is no Enter key in the UI vocabulary" (§4.7), which the store's `Open address` tile works around. | M |
| S5-4 | **Images:** bytes handed over once and referenced by a handle (a component-model `resource`), size-capped, decoded host-side | L |
| S5-5 | **Fonts:** check that Uzbek Latin (`oʻ`, `gʻ` — U+02BB) and Cyrillic render; bundle a font that covers them if egui's defaults do not | S |
| S5-6 | Dark theme following the OS (`install_theme` pins light today) | S |
| S5-7 | **Performance budget:** a store with 500 entries at 60 fps on a mid-range phone, with the per-frame boundary cost measured. That number decides whether the retained UI tree (after beta) is needed. | M |

## Sprint 6 — Network, and a real app

| ID | Item | Size |
|---|---|---|
| S6-1 | **ADR-3: guest networking** — a `fetch` in `host-api`, or `wasi:http` | S |
| S6-2 | **Fetch capability**, non-blocking — start returns a handle that is polled across frames, because `update` has a one-second deadline. Origins allow-listed in the manifest; size and time caps shared with `resolve::http`. | L |
| S6-3 | `locale()` and time zone capabilities | S |
| S6-4 | **Dogfood app:** one real mini-app using fetch + storage + layout — the first app not written to demonstrate a feature | M |
| S6-5 | **Fuzz the boundary:** random `host-api` calls (huge strings, out-of-range levels and indices) never panic the host | M |

## Sprint 7 — SDK and developer experience

**Goal:** exit criterion 4. Today no mini-app builds outside this repo: each one
points `wit_bindgen::generate!` at `path: "../../wit"`.

| ID | Item | Size |
|---|---|---|
| S7-1 | Publish the WIT as a package (or vendor it in the SDK) so a guest needs no checkout | M |
| S7-2 | Rust SDK crate + a `cargo generate` template | M |
| S7-3 | Evaluate `wasm32-wasip2`, whose rustc output is already a component, to drop the `componentize` step; otherwise ship `componentize` as an installable tool | M |
| S7-4 | **`nimadir` dev CLI:** `new`, `build`, `run` (the headless harness as a developer's tool), `check` (manifest and imports vs. host version), `pack` (hash for the registry) | L |
| S7-5 | C# and Python templates; a Go/TinyGo sample (promised in the README) | M |
| S7-6 | Docs: your first app, the WIT reference, permissions, publishing to the registry | M |

## Sprint 8 — Polish, package, ship

| ID | Item | Size |
|---|---|---|
| S8-1 | **Installers:** Windows (.msi), macOS (.dmg, signed and notarised per D2), Linux (AppImage / .deb), Android (AAB to Play internal testing, plus a plain APK) | L |
| S8-2 | Update check: the shell says when a newer release exists | S |
| S8-3 | Accessibility through egui's AccessKit support; every widget carries its label | M |
| S8-4 | Chrome in Uzbek, Russian and English | M |
| S8-5 | Error and loading pages; reload | S |
| S8-6 | **Security review** of the whole boundary — every capability, the policy, integrity — before the tag | M |
| S8-7 | Beta checklist against the exit criteria above; `CHANGELOG.md`; tag `v1.0.0-beta.1` | S |

---

## Cut line

If the plan slips, these go first, in this order: S8-2, S5-6, S7-5 (the Go
sample), S8-4, S3-8. Nothing in Sprints 1, 2 or 4 is cuttable — those are the
exit criteria.

## Road to 1.1 — the native DOM (ADR-4)

**Goal:** an app can say what HTML and CSS say — a tree of elements, styled by
stylesheets, laid out by block, flex and grid — and the host still draws it
natively, on desktop and phone alike, with no web engine anywhere
(`docs/adr/0004-native-dom.md`). The contract is `interface dom` in
`nimadir:shell@1.1.0`; apps built against `1.0` keep working unchanged.

**1.1 exit criteria:**

1. **Expressive.** The store, rewritten on the tree, and `showcase` look
   like a designer drew them: cards, a grid, a form, a brand colour, dark mode.
2. **Equal targets.** Every item works on Android as on desktop; touch
   scrolls what a wheel scrolls.
3. **Bounded.** The document is capped as a whole; a fuzz fixture drives every
   `dom` function with awkward input without a panic.
4. **Tested headlessly.** The tree, computed styles and boxes print; scripts
   click and type by node.
5. **Additive.** Every `1.0` app and test passes unchanged.

**Progress (2026-10-21):** D1 done — the tree, its limits, events, the
headless flags, `todo`. D2 done — the CSS parser, the cascade, `query-all`,
`--styles`, the `probe` fixture. D3 done — taffy layout with `calc()`, the
display list, hit-testing to the run, the egui painter with Noto Sans bold;
seen in the window under Xvfb (typing, ticking, deleting, `:hover`, a
gradient card in a grid, a scrolling list). D4 on the desktop — `select`,
range, `label`, form `submit`, `disabled`/`readonly`/`maxlength`, focus and
pointer events; the phone is still to see it (touch scrolling, the keyboard
over a field). D5 — `set-markup`, the Rust template on the tree, the guide in
`docs/APPS.md`, the store rebuilt on the tree (cards in a grid, search by
`hidden`, Enter). D6-1 and D6-2 — `fuzz` over `dom` (it found a NaN panic in
painting), the security review's addendum, 10 000 nodes at ~4 ms styling and
~16 ms layout in release, restyle without relayout for what only repaints,
linear markup. 161 assertions. Then `showcase`, the tree in full, listed in the
registry (170 assertions). Left for 1.1: the phone, and D6-3 on the owner's
say.

### D1 — The tree

| ID | Item | Size |
|---|---|---|
| D1-1 | `interface dom` at `1.1.0`: the fixture, the C# namespace, the templates' WIT; `newer_than_us` per interface | M |
| D1-2 | `Document`: elements, text, attributes, classes, inline style, listeners, the event queue; the document's limits | L |
| D1-3 | Headless: the tree prints; `--click`, `--type`, `--enter` by node | M |
| D1-4 | A `dom` sample app and its suite cases | S |

### D2 — CSS

| ID | Item | Size |
|---|---|---|
| D2-1 | Parser: declarations, selectors, `@media`, custom properties; bounded | L |
| D2-2 | Cascade: user-agent sheet (`--nimadir-*` theme variables), author sheets, inline, specificity, `!important`, inheritance, `var()`, `em`/`rem` | L |
| D2-3 | Headless prints computed style | S |

### D3 — Layout and paint

| ID | Item | Size |
|---|---|---|
| D3-1 | taffy: block, flex, grid, absolute, float; inline text as paragraphs; text measured through a trait | L |
| D3-2 | The display list; headless prints boxes | M |
| D3-3 | egui painter: backgrounds, borders, radii, shadows, opacity, text runs with a real bold, images, clipping | L |

### D4 — Controls and interaction

| ID | Item | Size |
|---|---|---|
| D4-1 | `button`, `a`, `input` (text, password, search, checkbox, radio, range), `textarea`, `img`, `progress`; `:hover`, `:active`, `:focus`, `:disabled`, `:checked` | L |
| D4-2 | `overflow: auto` scrolls by touch and wheel; a focused field is scrolled clear of the keyboard | M |
| D4-3 | Events: `click`, `input`, `change`, `submit`, `focus`, `blur`, `pointerdown`, `pointerup` | M |

### D5 — For app developers

| ID | Item | Size |
|---|---|---|
| D5-1 | `set-markup`, `query-all` | M |
| D5-2 | A Rust builder in the template; C# and Python regenerate; `docs/APPS.md` gains the tree | M |
| D5-3 | The store rewritten on the tree. The gallery stays the reference for `ui-*`, which stays in the contract | M |

### D6 — Bounded and fast

| ID | Item | Size |
|---|---|---|
| D6-1 | The fuzz fixture over `dom`; the limits tested; the security review's addendum | M |
| D6-2 | 10 000 nodes styled and laid out inside a frame; restyle only what changed | M |
| D6-3 | `ARCHITECTURE.md` and the invariants; `1.1.0-beta.1` on the owner's say | S |

## After beta — not in this plan

- **A retained "native DOM"** — planned: *Road to 1.1* above.
- A Makepad backend behind `UiCmd`.
- Zero-copy bulk data (shared buffers for pixels and geometry).
- Tabs; a self-serve publishing portal.

## Risks

| Risk | Effect | Mitigation |
|---|---|---|
| IME text on Android | Predictions, Cyrillic and Uzbek letters lost on phones | The S1-3 spike showed the keyboard and key events work; IME text needs GameActivity (S3-1, S3-4). Fallback: a native input overlay |
| Large guests on phones — `counter-py` compiles for ~30 s cold on desktop | A first open that looks like a hang | The compile cache already exists; precompile in the background at install time |
| One developer, sixteen weeks | Slips | The cut line above; Sprints 1, 2 and 4 hold |
| WIT records cannot grow | A "small" field addition breaks every published app | Add functions, never widen records (S4-1); `check` in S7-4 catches it |
