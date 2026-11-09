# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
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
