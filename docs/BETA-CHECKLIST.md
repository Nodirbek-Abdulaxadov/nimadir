# Beta checklist — 1.0.0-beta.1

The exit criteria from `docs/ROADMAP.md`, against the tree on 2026-10-21.
**Not tagged yet**: two criteria need things only the repository's owner
can do (push, switch Pages on). Everything here was built and run on one
Linux desktop and one Android 16 phone; "unverified" means not run anywhere
yet.

| # | Criterion | Status |
|---|---|---|
| 1 | **Installable** — Windows, macOS, Linux, Android, outside a checkout | **Partly.** `.deb` built and checked: it installs a shell that opens on its built-in store from an empty directory. AppImage fails in linuxdeploy on this machine. Windows (NSIS) and macOS (.dmg) are configured and in the release workflow, **unverified**. Android APK and AAB build (API 36) and **run on an Android 16 phone** (2026-10-21): the smoke test passes, and the store, storage, a network grant against the real cbu.uz, typing and settings were driven by hand. That run found twelve bugs, all fixed. |
| 2 | **Trusted downloads** — remote components checked against a hash from a registry over HTTPS | **Met.** HTTPS only (loopback excepted); SHA-256 pins; publisher pins; signatures. The hosted registry's workflow is written; **Pages is not switched on**, so it serves nothing yet. |
| 3 | **Real apps** — data kept, network declared, real layout, the user decides | **Met.** Storage (ADR-2), `fetch` (ADR-3) with grants, groups and widgets, images; `rates` uses all of it against the real cbu.uz. |
| 4 | **Outside developers** | **Met for Rust and C#** (`nimadir new/build/run/check`, `docs/APPS.md`); the Python template builds only in CI, **unverified** here. |
| 5 | **Stable contract** | **Met.** `nimadir:shell@1.0.0`, additive-only by minor versions; a newer app is refused with the functions it lacks named. |
| 6 | **CI on every PR** | **Written, never run**: nothing has been pushed. Lint, the suite on three OSes, Android, the C#/Python job, Pages and release workflows all wait on the first push. |
| 7 | **Bounded** | **Met.** Time, memory, output, nesting, image size, downloads, network; idle and background drawing; the security review's four fixes. |

## Known gaps carried into beta

- **IME text on Android** (`S3-4`): typing reaches the app as key events, so
  predictions and non-Latin input may be lost. Needs a winit change.
- **Android accessibility**: needs GameActivity, which waits on the above.
- **Images have no alternative text.**
- **No time zone** for apps (`locale` only).
- **The desktop window was looked at on Linux only** (X11, under Xvfb, in
  light and dark): the store, every sample, a network grant, the ask bar, a
  web page without the webview, settings. Windows and macOS have never shown
  it. The live switch of the desktop's dark mode is written, not exercised —
  it would have meant flipping the owner's desktop theme.
- **Predictive back is off** (`enableOnBackInvokedCallback="false"`): it
  needs a Java callback the app does not have. Android says the opt-out will
  go in a later version.
- **An activity the system destroys while the process lives hangs**
  (developer option "Don't keep activities"): winit does not end its loop on
  `Destroy` (`ARCHITECTURE.md` §4.12).
- See `docs/SECURITY-REVIEW.md` for accepted residual risks.

## Before tagging — the owner's steps

1. **Push** the sprint branches and open a PR; let CI run. Fix what it finds
   on Windows and macOS — those have never been built by anyone.
2. **Switch on GitHub Pages** (Settings → Pages → Source: GitHub Actions), so
   `registry.yml` publishes the hosted registry. Optionally add a
   `NIMADIR_SIGNING_KEY` secret (`nimadir keygen`) to sign the samples.
3. **Enable private vulnerability reporting** (Settings → Security), which
   `SECURITY.md` points to.
4. ~~Plug in the phone and run the smoke test~~ — done 2026-10-21 on an
   Android 16 phone; run it again on any other device you can.
5. Decide **D2** (Apple signing). Without it the `.dmg` is unsigned.
6. Then:

```bash
git tag -a v1.0.0-beta.1 -m "nimadir 1.0.0-beta.1"
git push origin v1.0.0-beta.1      # release.yml builds every installer into a draft
```
