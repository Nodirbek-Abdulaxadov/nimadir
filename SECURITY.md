# Security policy

nimadir runs code it downloads. The sandbox is the product, so a hole in it is
the most important kind of bug this project can have.

## Reporting a vulnerability

Please **do not open a public issue**. Report it privately through GitHub:
the repository's **Security** tab → **Report a vulnerability**. Include the
component (or the WIT calls it makes) that shows the problem, and what it
managed to do.

You should get a first reply within a week. A fix is credited to you in
`CHANGELOG.md` unless you would rather it was not.

## What counts

Anything that lets a mini-app do more than `wit/world.wit` grants it:

- reaching the filesystem, the network, other apps, or the shell's chrome
  (address bar, Home) outside the capabilities in `host-api`;
- getting past a resource limit — time per frame, linear memory, what one frame
  may send the host, download size (`docs/ARCHITECTURE.md` §4.10);
- crashing or hanging the host process from guest code;
- surviving being left: state that outlives the app's `Store`.

## Supported versions

Before the first beta, only the `main` branch is supported; fixes land there.

## Known gaps

These are planned work, not vulnerabilities — reports about them are still
welcome if you find a way they are worse than described here:

- **No integrity checks yet.** A component fetched over plain HTTP, or from a
  compromised server, is run as received. Hash pinning and HTTPS-only remote
  components are `S2-4` in `docs/ROADMAP.md`.
- **No per-app permissions yet.** Every mini-app gets the whole `host-api`,
  and `open-app` may ask for any address. Policy is `S4-2` and `S4-3`.
