# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-11-08

### Added
- **`Listener::bind_waking(addr, wake)`: the run loop is woken on connect.**
  An accept thread blocks on the endpoint (a blocking `accept` on unix; on
  Windows a blocking pipe instance of its own, created with the same SDDL and
  first-instance flag `bind` uses), hands each connection to the listener,
  calls `wake` and blocks again, so a host that sleeps on a doorbell serves a
  request the moment a client connects and nothing is retried per tick to
  notice one. `poll` keeps its contract (the host keeps polling as its
  fallback); the peer-uid gate, the slot pool and its deadlines still run on
  the polling thread. In this mode a Windows pipe instance is closed when its
  client is done rather than re-armed, and the pool is bounded at
  `MAX_SLOTS + MAX_OUTBOX`. Dropping the listener stops the thread without
  waiting on a client (atrium issue #15).
- **Answer later: `Listener::park` / `respond_parked` / `take_expired`.** A
  request the server cannot answer yet (an `inbox --wait` with nothing queued)
  is taken off the one-request-at-a-time latch under a `Ticket`, so `poll`
  yields the next request, and is answered on a later tick through the same
  never-blocking reply path. A hold lasts at most `HOLD_DEADLINE` (60 s): the
  ticket is reported once through `take_expired` and the server has until its
  next `poll` to answer it itself, after which the client is refused with
  `{"ok":false,"err":"held request timed out; ask again"}` and closed. At most
  `MAX_HELD` (64) requests are held; past that `park` refuses the newest with
  `{"ok":false,"err":"too many held requests; ask again"}` and returns `None`,
  never evicting an earlier waiter. A held client that hangs up frees its slot
  the next time the server looks. `request_held(addr, req, hold)` is the
  client for such a verb: `request` with both budgets extended by `hold`.
- First cut, moved out of atrium 0.43.0's `src/ipc` so atrium and fwterm serve
  one control channel: `Listener::bind` / `poll` (non-blocking, one request
  handed out at a time, several half-open clients parked) / `respond` (never
  blocks; the remainder drains on later polls) / `take_security_event`,
  `request` (the client: connect, one line in, one line out, bounded and
  deadlined), `address(app, pid)` (the per-process endpoint name), `idle_ms`.
  Same-user endpoint: a `0600` unix socket with a peer-uid check, a Windows
  pipe with an SDDL for the current user and SYSTEM and
  `FILE_FLAG_FIRST_PIPE_INSTANCE`; the unix endpoint tripwire. The framing,
  output queue and per-connection state machine (`wire`, `chan`, `winmap`) are
  platform-free and unit-tested everywhere; the four audited exploits replay
  against the real atrium binary in atrium's own tree.

### Changed
- The refusals for an oversized reply and for an oversized request are
  `{"ok":false,"err":…}`, the same failure shape as every other refusal; both
  used to say `"error"`. Client-side error strings no longer name atrium.
