# ADR-3: Guest networking

**Status:** Accepted (2026-10-20). Implements `S6-1` and `S6-2` in
`docs/ROADMAP.md`, and the grant machinery `S4-2` deferred to it.

## Context

A mini-app can reach nothing outside its sandbox but what `host-api` grants,
and until now that included no network. Real apps need one — rates, news,
anything with a server — and the network is the capability most worth
policing: it is where data leaves the device.

Two ways to give it:

- **`wasi:http`**, which wasmtime implements. Standard, and every WASI-aware
  toolchain speaks it. But it is a large surface — resources, streams,
  trailers, outgoing bodies written in pieces — and policy would have to be
  bolted onto each piece of it.
- **A `fetch` of our own** in `host-api`: one request in, one response out,
  over a handle the app polls.

## Decision

**Our own `fetch`**, because the policy is the point:

- **Declared.** An app may only reach origins its manifest lists under
  `network` (`{"network": ["https://api.example.com"]}`). Anything else fails
  at once, saying so; an app without a manifest declares nothing and reaches
  nothing.
- **Granted.** The first request to an origin asks the user, in the chrome —
  where no app can draw or click. A signed app's *yes* is remembered under its
  identity (`<data>/apps/<id>/grants.json`); an unsigned app is asked each run;
  a *no* lasts until the app is left.
- **Bounded.** HTTPS only (plain http to this machine alone, as everywhere);
  8 requests in flight per app; 8 MiB of body each way; the shell's 30-second
  silence timeout. Requests run on threads, never inside a frame.
- **Non-blocking.** `fetch` returns a number at once and `fetch-poll` answers
  it in a later frame; while any request is open the shell asks itself for
  frames, so the answer is polled without the user touching anything.

## Consequences

- One shape for every guest language: a request record in, a number out.
- No streaming, no WebSockets. They can come as further functions; nothing
  here closes the door on `wasi:http` later either.
- Headless, the runner waits for open requests between frames so tests stay
  deterministic, and `--confirm` answers the grant question yes.
