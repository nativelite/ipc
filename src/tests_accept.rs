//! The accept thread under pressure and at the end of its life: the bounded
//! hand-over, what a never-polled or dropped listener does to the clients it
//! was holding, a waker that panics, and the ring that may land late. Both
//! platforms.

use super::testutil::test_addr;
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Wait for `done` at most `limit`, a short nap at a time; the message on
/// failure names what did not happen.
fn within(limit: Duration, what: &str, mut done: impl FnMut() -> bool) {
    let t = Instant::now();
    while !done() {
        assert!(
            t.elapsed() < limit,
            "{what} did not happen within {limit:?}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// Join `handles`, insisting the whole set finishes within `limit`.
fn join_all<T>(handles: Vec<JoinHandle<T>>, limit: Duration, what: &str) -> Vec<T> {
    let t = Instant::now();
    for h in &handles {
        while !h.is_finished() {
            assert!(
                t.elapsed() < limit,
                "{what}: a client did not finish within {limit:?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
    handles
        .into_iter()
        .map(|h| h.join().expect("client thread"))
        .collect()
}

/// **Finding 1.** A flood of connects against a server that never polls
/// must not pile up one fd (or pipe instance) per client behind the loop:
/// the thread hands over at most `HANDOVER_CAP` and turns the rest away at
/// the door, counted. Nothing hangs: every client comes back on its own.
#[test]
fn a_connect_flood_against_a_stalled_loop_is_bounded_at_the_door() {
    const FLOOD: usize = 200;
    let addr = test_addr(9401);
    let rings = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&rings);
    let server = Listener::bind_waking(
        &addr,
        Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .expect("bind_waking");

    // Each client connects, sends one line and waits for a reply it will not
    // get from the loop: a zero hold, so its own stall budget (REPLY_STALL)
    // ends the wait. The server is never polled.
    let clients: Vec<JoinHandle<io::Result<String>>> = (0..FLOOD)
        .map(|_| {
            let a = addr.clone();
            thread::spawn(move || request_held(&a, r#"{"cmd":"list"}"#, Duration::from_millis(0)))
        })
        .collect();

    // Count only once every client has resolved on its own: a queued one
    // waits out its stall budget (REPLY_STALL, 5 s) and a refused one is cut
    // off at once. Until then a client may still sit in the kernel's listen
    // backlog, neither queued nor refused, and the counters are not final.
    let outcomes = join_all(clients, Duration::from_secs(10), "flood clients");
    // Each successful hand-over rings exactly once, so the rings are the
    // connections the never-polled loop is holding: at most HANDOVER_CAP, and
    // every other client was refused at the door — an exact split, not a floor.
    let queued = rings.load(Ordering::SeqCst);
    let refused = server.refused_at_door();
    assert!(
        queued <= HANDOVER_CAP,
        "{queued} connections queued behind the loop; the hand-over holds {HANDOVER_CAP}"
    );
    assert_eq!(
        refused + queued,
        FLOOD,
        "every client was either queued or refused at the door"
    );

    // What the clients saw. No client was answered `ok`. A queued one timed
    // out on its own budget. A refused one either read the busy refusal
    // (unix, when its request had landed before the refusal) or saw the
    // connection cut: on Linux a client whose request had NOT yet been
    // written when the server closed gets EPIPE on that write and never
    // reads the refusal sitting in its receive buffer — a race of the
    // client's making, not a leak of the server's; on Windows the pipe is
    // disconnected without a reply (one would be discarded) and the client
    // sees the pipe fail, or times out writing into it (ERROR_NO_DATA).
    let mut kinds: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut told = 0;
    let mut answered_ok = 0;
    for o in &outcomes {
        match o {
            Ok(r) if r.contains("server busy") => told += 1,
            Ok(r) => {
                answered_ok += 1;
                eprintln!("unexpected reply: {r}");
            }
            Err(e) => *kinds.entry(format!("{:?}", e.kind())).or_default() += 1,
        }
    }
    assert_eq!(answered_ok, 0, "a never-polled server answered a client");
    // On stderr for `--nocapture`: how the flood resolved on this machine.
    eprintln!("flood: queued={queued} refused={refused} told={told} errors={kinds:?}");
    let timed_out = kinds.get("TimedOut").copied().unwrap_or(0);
    let cut = outcomes.len() - told - timed_out;
    assert!(
        timed_out >= queued,
        "the {queued} queued clients must have waited out their budget; outcomes: told={told} {kinds:?}"
    );
    if cfg!(unix) {
        // Only the queued clients wait: a refused one is told or cut at once.
        assert_eq!(
            timed_out, queued,
            "a refused client must not wait out a budget; outcomes: told={told} {kinds:?}"
        );
        assert!(
            told > 0,
            "the refusal never reached a client; outcomes: {kinds:?}"
        );
    }
    assert_eq!(
        told + cut + (timed_out - queued),
        refused,
        "refused clients: told={told} cut={cut} timed_out={timed_out} queued={queued} {kinds:?}"
    );
    drop(server);
}

/// **Finding 2.** Connections the thread handed over but the loop never took
/// are closed when the listener drops: their clients see EOF at once, not a
/// pipe nobody will ever read.
#[test]
fn dropping_a_listener_closes_the_connections_still_in_the_hand_over() {
    let addr = test_addr(9402);
    let rings = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&rings);
    let server = Listener::bind_waking(
        &addr,
        Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .expect("bind_waking");
    let clients: Vec<JoinHandle<io::Result<String>>> = (0..3)
        .map(|_| {
            let a = addr.clone();
            thread::spawn(move || request_held(&a, r#"{"cmd":"list"}"#, HOLD_DEADLINE))
        })
        .collect();
    within(Duration::from_secs(5), "three connects handed over", || {
        rings.load(Ordering::SeqCst) == 3
    });
    // Never polled: the three sit in the hand-over. Drop.
    let t = Instant::now();
    drop(server);
    let outcomes = join_all(clients, Duration::from_secs(1), "clients after drop");
    assert!(
        t.elapsed() < Duration::from_secs(1),
        "drop plus EOF took {:?}",
        t.elapsed()
    );
    for o in &outcomes {
        let e = o
            .as_ref()
            .expect_err("a dropped server cannot have answered");
        assert!(
            matches!(
                e.kind(),
                io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::BrokenPipe
                    | io::ErrorKind::ConnectionReset
            ),
            "the client must see the connection end, got {e:?}"
        );
    }
}

/// **Finding 3.** Dropping the listener stops the thread on a real clock, and
/// the drop's own wake-up connect does not ring the waker as if a client had
/// arrived — at most one ring can land late, and none here where nothing
/// connected before the stop.
#[test]
fn a_drop_rings_at_most_once_late_and_never_hangs() {
    let addr = test_addr(9403);
    let rings = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&rings);
    let server = Listener::bind_waking(
        &addr,
        Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .expect("bind_waking");
    let t = Instant::now();
    drop(server);
    assert!(
        t.elapsed() < Duration::from_secs(3),
        "drop took {:?}",
        t.elapsed()
    );
    thread::sleep(Duration::from_millis(300));
    assert!(
        rings.load(Ordering::SeqCst) <= 1,
        "the drop's wake-up connect rang {} times",
        rings.load(Ordering::SeqCst)
    );
}

/// **Finding 5.** A waker that panics costs its ring, not the endpoint: the
/// connection is still handed over and served, and the next client's ring
/// goes through.
#[test]
fn a_panicking_waker_does_not_take_acceptance_down() {
    let addr = test_addr(9404);
    let rings = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&rings);
    let mut server = Listener::bind_waking(
        &addr,
        Box::new(move || {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("the first ring panics");
            }
        }),
    )
    .expect("bind_waking");
    for round in 0..2 {
        let a = addr.clone();
        let client = thread::spawn(move || request(&a, r#"{"cmd":"list"}"#));
        let mut got = None;
        for _ in 0..400 {
            match server.poll() {
                Ok(Some(req)) => {
                    got = Some(req);
                    break;
                }
                Ok(None) => thread::sleep(Duration::from_millis(5)),
                Err(e) => panic!("round {round}: poll reported the thread gone: {e}"),
            }
        }
        assert!(got.is_some(), "round {round}: the request never arrived");
        server.respond(r#"{"ok":true}"#).expect("respond");
        for _ in 0..200 {
            let _ = server.poll();
            thread::sleep(Duration::from_millis(2));
        }
        let reply = client.join().expect("client").expect("reply");
        assert!(reply.contains("true"), "round {round}: {reply}");
    }
    assert_eq!(
        rings.load(Ordering::SeqCst),
        2,
        "both connects rang (the first panicked)"
    );
}
