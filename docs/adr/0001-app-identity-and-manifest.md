# ADR-1: App identity and manifest

**Status:** Proposed — needs the owner's sign-off. Blocks `S2-4`, `S4-1`,
`S4-2` and `S4-4` in `docs/ROADMAP.md`.
**Date:** 2026-10-20

## Context

Today an app is whatever bytes an address returned. `registry::AppEntry` is
`name`, `src` and `description`: `name` is a nickname local to one registry, and
`src` is a path or URL. Nothing says "this is the same app as last time".

That has been fine, because nothing needed it: leaving an app destroys it
(invariant 3), and every app gets the whole `host-api`. Three planned features
each need it:

- **Storage** (`S4-4`): data is kept per app, so the key it is kept under must
  not be claimable by a different app — or one app reads another's data.
- **Permissions** (`S4-2`): "notes may use the network" has to stay attached to
  notes, not to whatever that URL serves next week.
- **Updates and integrity** (`S2-4`, `S2-5`, `S8-2`): a new version of the same
  app keeps its data and grants; a tampered copy gets neither.

So an identity has to be:

1. **stable** across versions and across moving to another host;
2. **unforgeable** by any other component;
3. **available for an app opened by a raw address**, not only from the
   registry — the address bar and `open-app` both take anything.

## Options

### A. The URL's origin — the web's model

Data and grants keyed by `scheme://host:port`.

- \+ Nothing to build; TLS makes it unforgeable.
- − Every app on one host shares one identity, and the hosted registry (D3)
  puts every app on one GitHub Pages origin. Moving hosts loses everything.
  A local file has no origin at all.

### B. Assigned by the registry

ID = (registry URL, entry name), with the registry pinning the bytes by hash.

- \+ Small: `S2-4` adds the hash anyway.
- − The identity belongs to the registry, not the app. The same app in two
  registries is two apps; an app opened by address has none; and a registry
  operator can point a name at different code, which then inherits the old
  app's data.

### C. A publisher key

Each publisher holds an Ed25519 key. ID = `<publisher>/<name>`, where
`<publisher>` is derived from the public key, and every release is signed.

- \+ Stable across versions and hosts; unforgeable without the key; works for
  raw addresses; and an update is simply "same key, same name, higher
  version" — what the store needs to offer one.
- − Developers manage a key (`S7-4`'s `nimadir pack` generates and signs). A
  lost key is a lost app: there is no recovering an identity, the same
  trade Android's app signing makes.

## Decision (proposed)

**C.** The registry is how apps are *found* and how their bytes are *pinned*;
it is not where their identity comes from.

**Identity.** `<publisher>/<name>`. `<publisher>` is the first 16 bytes of the
SHA-256 of the Ed25519 public key, base32, lowercase. Users never see it raw:
the store shows the app's display name and the publisher's name from the
registry.

**Manifest.** JSON in a custom section named `nimadir-manifest`, inside the
component. It travels with the bytes, so an app opened by address has one too,
and it needs no WIT change. A custom section can be appended to a component
after it is built, so Rust, C# and Python apps all get theirs from one tool —
`componentize --manifest` now, `nimadir pack` later.

```json
{
  "name": "notes",
  "display_name": "Notes",
  "version": "1.2.0",
  "description": "Plain-text notes, kept on this device.",
  "permissions": ["storage", { "network": ["https://api.example.com"] }]
}
```

There is deliberately **no minimum-host-version field**. A component's imports
already say exactly what it needs, and `S2-6` checks them against what the host
provides; a second, hand-written statement of the same thing could only
disagree with the first.

**Signature.** A second custom section, `nimadir-signature`: the public key, and
an Ed25519 signature over the component with that section removed. Checked at
load, after the registry's hash (`S2-4`), together with "the manifest's name and
publisher match the registry entry".

**Unsigned components** — a local build, an address typed by hand — still run,
as *unverified*: storage lasts only until the app is left, and a permission is
asked for every time rather than remembered. Nothing is lost for development,
and nothing persistent can be claimed without a key.

## Consequences

- Invariant 3 holds in substance: per-app data lives on disk, owned by the
  host and keyed by this ID, never in host memory. ADR-2 (`S4-4`) settles the
  details.
- One new dependency, `ed25519-dalek` — pure Rust, no system libraries, so the
  default headless build is unaffected (invariant 4). Parsing the sections uses
  `wasmparser`, already in the tree through Wasmtime.
- `registry::AppEntry` gains `id`, `version` and `sha256`. The WIT `app-entry`
  record does **not** — widening a record breaks every guest built against it —
  so the store reads the new fields through a new function (`S4-1`).
- `S7-4` must make key handling hard to get wrong, and the publishing guide
  must say plainly what losing a key costs.
- Two publishers may both ship a `notes`. The store shows the publisher next to
  the name wherever that is ambiguous.

## Questions for the owner

1. Accept **C**, or take **B** for less work now and migrate later? Migrating
   identities after users have data under them is the expensive direction.
2. Unsigned components: run without persistence, as proposed, or refuse them
   outside an explicit developer mode?
3. Should the hosted registry (`S2-3`) accept only signed apps from day one?
