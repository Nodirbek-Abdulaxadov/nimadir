# Changelog

Notable changes, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/). The road from here to the first
beta is `docs/ROADMAP.md`.

## [Unreleased]

### Security
- An app can open an app the registry lists, but opening any other address
  needs the user's yes, asked in the chrome where no app can answer for them.
- **Signed apps** (ADR-1). A component may carry a manifest and its
  publisher's Ed25519 signature; one changed after signing, or signed by
  someone other than its registry entry names, is refused before it is
  compiled. `nimadir` makes keys, adds manifests and signs.
- Remote components and registries are fetched over HTTPS only; plain http
  is allowed to this machine alone. A registry entry may pin its component's
  SHA-256, and bytes that differ are refused before they are compiled.
- A guest can no longer fill the host's memory by sending UI. Widget text and
  log lines are copied into the host, outside the guest's 512 MiB cap, so one
  buffer sent in a loop grew the host without limit for the whole one-second
  frame budget. A frame may now send at most 65 536 widgets or 16 MiB of text;
  past that the guest is stopped and the reason names the limit.

### Added
- **Check for updates**, on the settings page and as `--check-update`: asked
  only when the user asks, never at start.
- Store cards tell screen readers what they are (desktop; AccessKit through
  egui). Android's needs GameActivity, which waits on IME support.
- The window speaks Uzbek, Russian or English, by the system's locale — on
  Android read from the platform, where no environment variable says it.
- A Reload button in the chrome (`--reload` headlessly): the app afresh,
  without a trip home.
- **`nimadir`, the developer tool** (it absorbs `nimadir-pack`): `new` makes a
  Rust, C# or Python app with its own copy of the WIT, so it builds outside
  this repository; `build` (Rust via `wasm32-wasip2`, no componentize step),
  `run`, `check`, plus `keygen` / `pack` / `inspect`.
- `docs/APPS.md`, the guide to writing an app.
- A fuzzing fixture: every host function, forty times a frame for 300 frames,
  with awkward inputs; the suite runs it.
- **rates**, the first real app: the Central Bank of Uzbekistan's exchange
  rates and a converter, in Uzbek, Russian or English, working offline from
  the last rates it kept.
- `build-mini-apps.sh` packs an app's `manifest.json` into it, and signs it
  when `NIMADIR_SIGNING_KEY` names a key; the registry workflow can sign from
  a secret, and the published registry then pins each publisher.
- **Network for apps** (ADR-3): `fetch` / `fetch-poll`, to origins an app's
  manifest declares, once the user allows — asked in the chrome, remembered
  for a signed app. HTTPS only, no redirects followed, 8 in flight, 8 MiB.
- `locale()`: the user's language and region, BCP 47.
- **More to build with**: groups (`ui-begin` / `ui-end`: row, column,
  section, scroll), `ui-checkbox`, `ui-choice`, `ui-separator`,
  `ui-progress`, and `ui-submitted` for acting when typing is done. The new
  `gallery` sample uses every one.
- **Images**: `image-load` hands PNG or JPEG over once, `ui-image` draws it
  by number. 4096 pixels a side, 8 megapixels an app, checked from the
  header before anything is decoded.
- **Settings page** (`nimadir:settings`, linked from the store): what each
  app keeps, the caches, clearing either. `--kept`, `--clear` and
  `--clear-cache` do the same from the command line.
- **Storage** (ADR-2): `storage-get` / `-set` / `-remove` / `-keys`. Kept
  between visits for an app that is signed and asks for `storage`, under its
  identity; memory-only for any other. 1 MiB per app. `counter` keeps its
  count this way when it is signed.
- The shell opens on a start page from any directory: the store is built
  into the binary, and the registry is looked for in `NIMADIR_REGISTRY` and
  the user's config directory, not only the working directory.
- Pinned remote components are cached by hash and open again offline.
- **A hosted registry.** `tools/publish_registry.py` builds one with every
  entry pinned; a Pages workflow publishes it. A shell with no registry of its
  own opens on the copy saved last time and refreshes it in the background.
- `--version`; a log of navigations, failures and panics in the data
  directory; `NIMADIR_HOME` to put config, data and caches under one root.
- `LICENSE` (MIT, as the README always said), this changelog, and
  `SECURITY.md`.
- `docs/ROADMAP.md`: the sprints from MVP to beta, and what beta is held to;
  `docs/adr/0001-app-identity-and-manifest.md`, proposed.
- CI on GitHub Actions: fmt and clippy, the suite on Linux, Windows and macOS,
  and a job that builds and runs the C# and Python guests.
- `android/`: the shell on Android — the desktop window code behind an
  `android_main`, packaged by Gradle (`android/build.sh`: debug APK and
  release AAB, API 36) and built in CI. It loads the registry as the desktop
  does instead of bundling the samples. `tools/android-smoke.sh` checks a
  device.
- **Back.** The shell keeps a history; Back reopens the page before, from a
  Back button in the chrome, Android's system Back, or a mouse's back button.
  Back from the start page leaves the app on Android. Headless: `--back`.
- **`request-frame`** in the WIT (additive): a guest asks for its next frame
  in N ms. `hello` uses it to show how long it has been open.

### Changed
- The window follows the system's light or dark theme; light where the
  system does not say.
- The window redraws only when something happens — input, a load, a webview
  page, or a guest's `request-frame` — instead of every frame. On a phone the
  old loop held 58 % of a core idle and 99 % in the background.
- The address bar shows a listed app by its name, not its file path.
- On Android every control is at least 48 dp tall.
- `host` is now a library plus the desktop binary, so the Android crate can
  share everything the desktop runs.
- All Rust sources are `cargo fmt`-formatted, which CI now checks.

### Fixed
- Uzbek Latin's oʻ and gʻ (U+02BB) drew as boxes: none of egui's bundled
  fonts has the character. Noto Sans is bundled as a fallback for it.
- On Android the UI is laid out inside the window insets. It was drawn under
  the status bar, where the Home button could not be tapped; the address bar
  also shrinks now, so its Open button stays on a phone's screen.
- `build-mini-apps.sh` skips the C# app unless a .NET **10** SDK is present;
  an older SDK used to fail the whole build instead.
- `run-tests.sh` no longer needs `truncate`, which macOS does not ship.
- `.gitattributes` keeps shell scripts LF on Windows checkouts, where bash
  otherwise fails on the stray `\r`.

## 0.1.0 — the MVP (untagged)

Milestones M1–M9 in `README.md`, verified in `docs/ARCHITECTURE.md` §1:

- A native shell (Rust, Wasmtime, egui) running sandboxed WASM **component**
  mini-apps, with the whole host↔guest contract in `wit/world.wit`.
- Mini-apps in Rust, C# (componentize-dotnet) and Python (componentize-py) on
  the same host, unchanged.
- Loading from a file path or an HTTP URL, routed by content type; web pages in
  an optional webview module.
- The start page is itself a mini-app (`store`), reading the registry through
  `list-apps` and navigating through `open-app`.
- Guests bounded in time (~1 s per frame, ~10 s to start), memory (512 MiB) and
  downloads (64 MiB, 30 s of silence).
- An end-to-end test suite driven through the real binary, headlessly.
