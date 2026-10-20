# ADR-2: Per-app storage

**Status:** Accepted (2026-10-20). Implements `S4-4` in `docs/ROADMAP.md`.

## Context

Invariant 3 says leaving an app destroys it — its `Store`, its linear memory,
everything it held — and that no state may outlive the `Store`. That made a
counter forget its count, which was the point while apps were demos. A real
app (notes, settings, a login) has to keep something between visits.

The invariant exists for two reasons that still hold: a guest that is left
must not keep running or keep host memory, and nothing one app keeps may be
reachable by another.

## Decision

Four functions in `host-api` — `storage-get`, `storage-set` (a `result`, so
a full store says why instead of trapping), `storage-remove`, `storage-keys`
— over a map of strings the **host** keeps for the app.

- **Kept between visits only under an identity**: the app is signed (ADR-1)
  *and* its manifest asks for `storage`. The data lives at
  `<data>/apps/<publisher>/<name>/storage.json`. Signing is what makes the key
  unforgeable: an unsigned app cannot claim another app's name, because it has
  no name to claim.
- **Any other app's storage is memory-only** and ends with it, exactly like
  the rest of its state.
- **1 MiB per app**, keys and values together. Over it, `storage-set` fails
  with the reason; the app carries on.
- **Written at most every 250 ms** and when the app is left. A pending write
  asks the shell for a frame, so a change made just before the app goes idle
  is not held until the next tap.

## Invariant 3, restated

*Leaving an app destroys it; nothing of it stays in host memory.* What an app
put in storage under its identity is kept **on disk, by the host**, and handed
back only to the same identity. The guest never sees a path, and the file is
written only between the guest's calls.

## Consequences

- An app that wants persistence has to be signed, which is also what the
  hosted registry will require (ADR-1).
- Clearing an app's data, and showing what it uses, belong to the settings
  page (`S4-5`).
- Strings only. Binary data is for later, with the handle-based bulk data
  the roadmap leaves after beta.
