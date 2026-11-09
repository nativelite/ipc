# nativelite-ipc

**A local, same-user, one-line-request / one-line-reply control channel.** A
Windows named pipe or a unix-domain socket, chosen at compile time, served
**non-blocking** from a run loop and spoken by short-lived clients. **Zero
runtime dependencies**: the Rust standard library only.

## Why

[atrium](https://github.com/nativelite/atrium) runs agents in panes and lets a
pane drive the others through `atrium ctl`: one JSON line in, one JSON line
out, over a channel only the launching user can reach. That channel took a
security audit and four replayed exploits to get right (a reply split by a
non-blocking partial write, one silent connection a second denying service, a
67 MB reply, EOF without a newline). fwterm needs the same channel. This crate
is that implementation, moved out so it is written once.

## Install

```toml
[dependencies]
nlipc = { package = "nativelite-ipc", version = "0.1" }
```

## Use

Server, inside a loop that must never block:

```rust
let addr = nlipc::address("myapp", std::process::id());
let mut listener = nlipc::Listener::bind(&addr)?;
loop {
    // … the rest of the tick …
    while let Ok(Some(line)) = listener.poll() {      // Ok(None): nothing ready
        let reply = handle(&line);                    // one JSON line
        let _ = listener.respond(&reply);             // never blocks; drains later
    }
    if let Some(alarm) = listener.take_security_event() {
        eprintln!("{alarm}");                         // the endpoint was replaced
    }
}
```

Client, anywhere on the same machine as the same user:

```rust
let reply = nlipc::request(&addr, r#"{"cmd":"list"}"#)?; // bounded, deadlined
```

## What it guarantees

- **One request at a time is handed out**; `respond` before the next `poll`
  yields another. Several half-open clients are accepted and parked, so no
  stalled connection wedges the channel; a client that stops reading trips a
  stall deadline and is dropped.
- **`respond` never blocks.** What the kernel takes now is written now; the
  rest is parked and flushed by later polls. A reply over `MAX_REPLY`
  (8 MiB) is refused with `{"ok":false,"err":…}`, never truncated.
- **A reply is whole or an error.** The client reads to the newline under a
  deadline; EOF before it is an error, never a short success.
- **Same-user only.** unix: `0600` socket, peer uid checked on every accept.
  Windows: an SDDL for the current user and SYSTEM, first-instance flag so a
  pre-created name makes `bind` fail loudly. A same-user attacker is an
  explicit non-goal (such a process already holds everything the channel could
  protect); the unix endpoint tripwire is a courtesy, not a defence.

## Development

```bash
python dev.py check   # zero-dependency guard + cargo fmt --check + cargo test
```

The byte-level logic (`wire`, `chan`, `winmap`) compiles and is tested on every
platform, so Windows sequencing is covered on Linux and macOS too. The `sys_*`
modules hold only syscalls and lifetime.

MIT. Part of [nativelite](https://github.com/nativelite): lightweight packages
built on native tools.
