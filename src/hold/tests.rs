//! The hold pool, driven with constructed instants: deadline, cap, lapse
//! reporting, and that a ticket answers exactly its own connection.

use super::*;
use std::time::Duration;

#[test]
fn a_ticket_takes_back_exactly_the_connection_it_was_issued_for() {
    let t0 = Instant::now();
    let mut held: Held<&str> = Held::new();
    assert!(held.is_empty());
    let a = held.park("alpha", t0);
    let b = held.park("beta", t0);
    assert_ne!(a, b, "tickets are unique");
    assert_eq!(held.len(), 2);
    assert_eq!(held.take(b), Some("beta"));
    assert_eq!(held.take(b), None, "a ticket answers once");
    assert_eq!(held.take(a), Some("alpha"));
    assert!(held.is_empty());
    assert!(held.take_expired().is_empty(), "nothing lapsed");
}

#[test]
fn tickets_are_never_reused_for_the_life_of_the_pool() {
    let t0 = Instant::now();
    let mut held: Held<u32> = Held::new();
    let mut seen = std::collections::HashSet::new();
    for n in 0..200 {
        let t = held.park(n, t0);
        assert!(seen.insert(t), "ticket {t:?} issued twice");
        held.take(t);
    }
}

/// The cap refuses the newest, never evicts a waiter: `is_full` is what the
/// caller checks before parking.
#[test]
fn the_pool_is_full_at_max_held_and_no_sooner() {
    let t0 = Instant::now();
    let mut held: Held<usize> = Held::new();
    for n in 0..MAX_HELD {
        assert!(!held.is_full(), "full after only {n} holds");
        held.park(n, t0);
    }
    assert!(held.is_full());
    assert_eq!(held.len(), MAX_HELD);
    // Answering one makes room for one.
    let t = held.park(usize::MAX, t0); // the caller would not do this; the pool still bookkeeps
    held.take(t);
    assert!(held.is_full());
}

/// The deadline, in two steps: the first sweep past it REPORTS the ticket and
/// keeps the connection (so the server may give its own answer this tick);
/// the next sweep hands the still-unanswered connection back to be refused.
#[test]
fn a_hold_past_the_deadline_is_reported_once_then_handed_back_for_refusal() {
    let t0 = Instant::now();
    let mut held: Held<&str> = Held::new();
    let old = held.park("old", t0);
    let young = held.park("young", t0 + Duration::from_secs(30));

    // Just inside the deadline: nothing happens.
    let early = t0 + HOLD_DEADLINE;
    assert!(held.sweep(early).is_empty());
    assert!(held.take_expired().is_empty());

    // Past it: `old` is reported, still held; `young` is untouched.
    let late = t0 + HOLD_DEADLINE + Duration::from_millis(1);
    assert!(held.sweep(late).is_empty(), "first sweep refuses nothing");
    assert_eq!(held.take_expired(), vec![old]);
    assert!(held.take_expired().is_empty(), "reported once");
    assert_eq!(held.len(), 2);

    // Still unanswered on the next sweep: handed back for the refusal.
    let refused = held.sweep(late + Duration::from_millis(20));
    assert_eq!(refused, vec!["old"]);
    assert_eq!(held.len(), 1);
    assert!(held.take(old).is_none(), "gone from the pool");

    // The server that answers a reported ticket in time wins the race.
    let late2 = t0 + Duration::from_secs(30) + HOLD_DEADLINE + Duration::from_millis(1);
    assert!(held.sweep(late2).is_empty());
    assert_eq!(held.take_expired(), vec![young]);
    assert_eq!(held.take(young), Some("young"));
    assert!(held.sweep(late2 + Duration::from_secs(1)).is_empty());
    assert!(held.is_empty());
}

/// A client that hangs up while held frees its slot at once and its ticket
/// is reported as lapsed, so the server forgets it.
#[test]
fn a_held_client_that_left_is_pruned_and_reported() {
    let t0 = Instant::now();
    let mut held: Held<(&str, bool)> = Held::new();
    let stay = held.park(("stay", false), t0);
    let left = held.park(("left", true), t0);
    held.prune(|c| c.1);
    assert_eq!(held.len(), 1);
    assert_eq!(held.take_expired(), vec![left]);
    assert!(held.take(left).is_none());
    assert_eq!(held.take(stay), Some(("stay", false)));
}

#[test]
fn the_hold_deadline_outlasts_the_client_budgets_it_must_cover() {
    use super::super::wire::{REPLY_DEADLINE, REPLY_STALL};
    // A held client waits HOLD_DEADLINE plus the ordinary budgets; the hold
    // itself must be the long part, or `request_held` buys nothing.
    assert!(HOLD_DEADLINE > REPLY_DEADLINE);
    assert!(HOLD_DEADLINE > REPLY_STALL);
}
