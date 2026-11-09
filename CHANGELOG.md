# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- **The never-reads test no longer races the scheduler** (atrium issue #9).
  `neither_poll_nor_respond_waits_on_a_client_that_never_reads` asserted that
  every `poll` and `respond` returned inside 50 ms, which a parallel cargo
  build on the same box could fail with nothing blocking. The property is
  now proven by shape: after `respond` the tail of the (4 MiB) reply is
  parked (`outbox_len() == 1`), where a call that waited on the client would
  have returned with nothing parked or never returned; the per-call bound is
  1 s, which only a wait on the peer can exceed, and the 20 ms tick budget
  is measured and printed, not asserted. Test only; no crate behaviour
  changed.

## [0.2.0] - 2026-11-08

### Fixed (review of the accept thread, before release)
- **The hand-over from the accept thread to `poll` is bounded** at
  `HANDOVER_CAP` (`MAX_SLOTS + MAX_OUTBOX`); it was an unbounded channel, so a
  connect flood against a stalled loop held one fd or pipe instance per
  client until EMFILE. A client past the cap is turned away at the door
  (unix: `{"ok":false,"err":"server busy; ask again"}` then close; Windows: a
  close, since the disconnect would discard a reply) and counted;
  `Listener::refused_at_door` reads the count and the bounded-rate refusal
  report names it.
- **Windows: connections still queued when the listener drops are closed**
  (`SendHandle` owns its handle and disconnects+closes on drop; `Drop` drains
  the hand-over), so their clients see EOF instead of hanging on a pipe nobody
  reads. The same drain on unix.
- **`Drop` waits for the accept thread on a real clock** (200 ms unix, 2 s
  Windows, 1 ms naps) instead of a spin count or an unbounded busy loop, and
  detaches with a warning past the deadline. The thread checks the stop flag
  right before ringing, so at most one ring can land after the drop; a waker
  must be harmless then (atrium's ring into an empty slot is).
- **A panic in the waker no longer kills acceptance**: it is caught; and a
  thread that has nevertheless exited makes `poll` return an error (the host
  notices) instead of reporting an idle channel forever.
- Documented: the wake fires on connect, before the request bytes land, so the
  first poll after a wake may see nothing and the fallback tick serves it; on
  Windows the pool plus the held instances is bounded at `POOL_MAX + MAX_HELD`.

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
