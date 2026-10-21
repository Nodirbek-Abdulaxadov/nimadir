# Security review before beta (S8-6)

**Date:** 2026-10-20. **Scope:** everything a mini-app can reach — every
function in `wit/world.wit` — and the shell's handling of what comes back
through it; the registry and downloads; the WASI surface. **Not in scope:**
the webview module's web pages (they run in the platform's browser engine,
with no route to the host), and the Rust and Wasmtime toolchains themselves.

The model: a mini-app is untrusted code. It must reach nothing `host-api`
does not grant; it must not be able to stop, starve or mislead the shell; and
what it says must not be able to pass for what the shell says.

## Findings fixed by this review

| # | Finding | Fix | Test |
|---|---|---|---|
| 1 | A guest's `log` reached the terminal raw: an escape sequence is a terminal command (retitle, move the cursor, overwrite), and a newline in the shell's log file forged an entry | `text::visible` escapes control and bidi characters at `log`, and `diag::line` escapes everything it writes | "a guest's control characters reach the terminal escaped" |
| 2 | WASI stderr was the shell's own (`inherit_stderr`), the same raw path to the terminal for any runtime-bearing guest | Captured in a 64 KiB pipe per app and moved into its log, escaped, after each call | (C#/Python guests, CI polyglot job) |
| 3 | A bidirectional override in an address made the chrome's "wants to open …" / "wants to connect to …" question show a different address from the one the answer applies to | Both questions show the address through `text::visible` | — (window; not seen on a screen) |
| 4 | An app could ask to reach `http://127.0.0.1:…`, `192.168.…` and the like, and one "Allow" opened a developer's database or a router's admin page to it | Origins naming this machine or a private network by address are refused unless `NIMADIR_ALLOW_LOCAL_NETWORK` is set | "without leave, no app reaches this machine or its network" |

## Capability by capability

| Capability | What it grants | Threat | Mitigation | Tested by |
|---|---|---|---|---|
| any call | — | never returning; growing memory; flooding the host with output | ~1 s epoch deadline per frame, ~10 s to start; 512 MiB linear memory; 16 MiB / 65 536 commands of output per frame; each a trap that names the limit | `runaway` buttons 0–2 |
| any call | — | malformed input making the host panic | `fuzz`: 12 000 calls across every function with awkward inputs, no panic | "the host answers every function's worst inputs" |
| `ui-*` | drawing in the page area | drawing over the chrome; spoofing host UI | the page area is a rect below the chrome; no coordinates in the vocabulary; questions are chrome | (by construction) |
| `ui-begin` / `ui-end` | nesting | exhausting the renderer's stack | 32 levels drawn, the rest flattened; groups always balanced | "nested past the limit are drawn flat" |
| `image-load` | decoding images | decompression bombs | size read from the header before allocating; 4096 a side, 8 MP an app; decoder limits as well | "an image that claims to be huge is refused" |
| `open-app` | navigation | taking the user somewhere unasked | registry apps only; anything else asks in the chrome | "asks before opening an address the registry does not list" |
| `storage-*` | persistence | reading another app's data; filling the disk | kept only under a signed identity; 1 MiB an app; identity checked before it becomes a path | storage and `--clear` cases |
| `fetch` | the network | exfiltration; reaching unchecked origins via redirects; local services | declared origins only; user grant; HTTPS (loopback alone excepted, and local networks off by default); no redirects followed; 8 in flight, 8 MiB | the "network for apps" cases |
| `list-apps` | the registry | learning what is installed | it is the registry by design; see residual risks | — |
| `now-millis`, `locale` | time, language | fingerprinting | coarse by nature; see residual risks | — |
| WASI p2 | runtime support for C#/Python | filesystem, network, environment | no preopens, no sockets, no environment; stderr captured (finding 2) | polyglot CI job |

Outside `host-api`:

| Surface | Threat | Mitigation |
|---|---|---|
| Downloads | a server streaming forever; a huge body | 30 s of silence; 64 MiB; refused before the body when `Content-Length` says so |
| Remote components | tampering in transit or at rest | HTTPS only (loopback excepted); optional SHA-256 pin per registry entry; signatures (ADR-1) with a publisher pin |
| Signed components | a signature claiming code it did not sign | the signature covers every byte before it and must be the last section; checked before compilation |
| The registry | listing malicious apps | it is trusted by whoever configures it; the hosted one is the project's, entries pinned; a registry cannot raise an app's permissions |
| The start page | its exemption from the `open-app` question | the exemption is the page titled `store` — what Home loads from the registry — and a guest cannot give another page that title |

## Addendum: the element tree (ADR-4, D6-1, 2026-10-21)

`interface dom` adds a document the host keeps for the life of the app, CSS
and markup the host parses, and layout and painting it runs on the guest's
say. Reviewed the same way:

| Surface | Threat | Mitigation | Tested by |
|---|---|---|---|
| the document | growing the host without end: nodes, text, stylesheets, events | 65 536 nodes, 32 MiB of strings and sheets (each node costing 64 bytes besides), 256 levels, a trap naming the limit; 1024 events queued, oldest dropped | `runaway` buttons 6–8 |
| any `dom` call | malformed input making the host panic | `fuzz` drives every `dom` function — cycles, stale numbers, garbage CSS and markup, unreadable selectors — while the page is styled, laid out and painted each frame | "the host answers every function's worst inputs" (run with `--styles --boxes --paint`) |
| selectors | exponential matching (`a a a … b` over a deep tree) | 32 compounds a selector, 10 000 steps a match, then no match | `fuzz` |
| `var()` | expansion bombs (`--a: var(--b) var(--b)`, …) and cycles | 16 levels, 4096 tokens a value, then the value is dropped | `fuzz` |
| `calc()`, lengths | nesting; values that send layout to infinity or NaN | 16 levels of brackets; lengths bounded to ±1 000 000; layout's output made finite before it is drawn, scrolled or hit-tested | `fuzz` (a NaN radius panicked before this review) |
| `set-markup` | parser recursion; scripts | read with an explicit stack, refused past 256 levels; `script` and `style` dropped unread | "a script in markup is dropped, never run" |
| painting | drawing over the chrome, or outside the page | everything is clipped to the page area; a box positioned above it is not drawn | seen in the window: a box at `top: -70px` stays out of the chrome |
| links | navigating unasked | `<a href>` is followed like `open-app`: listed apps, else the chrome asks | the store's cases |
| text | control characters reaching a terminal | the headless outline quotes and escapes every guest string | the outline cases |

## Residual risks, accepted for beta

- **By name, not by address.** The local-network check reads the origin: a
  public name that resolves to `127.0.0.1` (DNS rebinding) is not caught. The
  user still has to allow that origin.
- **Unpinned remote apps** — opened by a typed address, or listed with no
  `sha256` — are protected by TLS alone.
- **`list-apps`** hands every app the registry, sources included; a registry
  in the user's config directory may name local paths.
- **`now-millis`** at millisecond precision is a timing source, as in a
  browser.
- **Android** was last run on a device in the spike, before this review's
  fixes; nothing here is Android-specific, but nothing here was re-run there.
- **A `select`'s menu** is drawn in egui's popup layer, which may overlap the
  chrome while open; it holds only the app's option labels, under the
  select the user tapped.
- **No independent review.** This was the project reviewing itself. Before a
  stable release, someone who did not write it should try.
