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

- **Pinning is per entry.** Remote components are fetched over HTTPS only
  (loopback excepted), and a registry entry with a `sha256` is refused unless
  the bytes match. A component opened by address with no registry entry, or
  an entry with no pin, is run as received — TLS is all that protects it.
  A component may be signed (ADR-1), and an entry may require its publisher;
  the hosted registry does not require signatures yet.
- **Permissions are per capability, not per call.** The network is
  declared per origin and allowed by the user (ADR-3); storage persists only
  for a signed app that asks; `open-app` to an unlisted address needs the
  user's yes. The rest of `host-api` — UI, the clock, the registry list,
  locale — every app has.
