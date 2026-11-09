// ---------------------------------------------------------------------------
// hold: requests the server chose to answer later. Pure.
//
// `Listener::poll` hands out one request at a time and the caller owes it a
// `respond` before the next poll yields another. `park` takes that request's
// connection out of the latch and puts it here under a `Ticket`, so the channel
// keeps serving while the caller waits for whatever the answer depends on (a
// delivery for an `inbox --wait`). The connection is still one the server is
// holding open on a client's behalf, so each has a deadline and the pool a cap;
// both are decided here, from a `now` the caller passes in, so they test
// without sleeping. The entry type is generic: unix parks a `Channel<Stream>`,
// Windows a whole pipe `Instance`.
// ---------------------------------------------------------------------------
use super::wire::{expired, HOLD_DEADLINE, MAX_HELD};
use std::time::Instant;

/// The handle to a held request: what [`crate::Listener::park`] returns and
/// [`crate::Listener::respond_parked`] takes. Unique for the life of the
/// listener; never reused, so a stale ticket cannot answer someone else's
/// request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ticket(pub(crate) u64);

struct Entry<T> {
    ticket: Ticket,
    conn: T,
    since: Instant,
    /// The deadline passed and the caller has been told once (through
    /// [`Held::take_expired`]). Still here on the next sweep means the caller
    /// did not answer: the entry is handed back to be refused and closed.
    reported: bool,
}

/// The held requests, oldest first.
pub struct Held<T> {
    entries: Vec<Entry<T>>,
    next: u64,
    /// Tickets whose hold is over and that the caller has not yet been told
    /// about: the deadline passed (the caller may still answer, this tick) or
    /// the client left (nothing to answer).
    lapsed: Vec<Ticket>,
}

impl<T> Default for Held<T> {
    fn default() -> Held<T> {
        Held {
            entries: Vec::new(),
            next: 0,
            lapsed: Vec::new(),
        }
    }
}

impl<T> Held<T> {
    pub fn new() -> Held<T> {
        Held::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Would one more hold exceed [`MAX_HELD`]? The caller refuses the newest
    /// request (with [`super::wire::HOLD_FULL_REPLY`]) rather than evicting an
    /// older waiter, which has done nothing wrong.
    pub fn is_full(&self) -> bool {
        self.entries.len() >= MAX_HELD
    }

    /// Hold `conn` from `now`. The caller checks [`is_full`](Held::is_full)
    /// first; this never evicts.
    pub fn park(&mut self, conn: T, now: Instant) -> Ticket {
        self.next += 1;
        let ticket = Ticket(self.next);
        self.entries.push(Entry {
            ticket,
            conn,
            since: now,
            reported: false,
        });
        ticket
    }

    /// Take the connection `ticket` names, to answer it. `None` for a ticket
    /// that is not held: already answered, lapsed, or never issued.
    pub fn take(&mut self, ticket: Ticket) -> Option<T> {
        let i = self.entries.iter().position(|e| e.ticket == ticket)?;
        Some(self.entries.remove(i).conn)
    }

    /// Drop every entry `gone` says has lost its client, and report their
    /// tickets as lapsed. The probe is the platform's non-blocking read: a
    /// client that hung up while held should free its slot now, not at the
    /// deadline.
    pub fn prune(&mut self, mut gone: impl FnMut(&mut T) -> bool) {
        let mut i = 0;
        while i < self.entries.len() {
            if gone(&mut self.entries[i].conn) {
                let e = self.entries.remove(i);
                self.lapsed.push(e.ticket);
            } else {
                i += 1;
            }
        }
    }

    /// Age the holds. An entry past [`HOLD_DEADLINE`] is reported once (its
    /// ticket joins [`take_expired`](Held::take_expired)); one that is still
    /// here on the following sweep, unanswered, is handed back for the caller
    /// to refuse and close. So the server using this gets one tick to give a
    /// timed-out waiter its own answer before the transport's refusal goes out.
    pub fn sweep(&mut self, now: Instant) -> Vec<T> {
        let mut refuse = Vec::new();
        let mut i = 0;
        while i < self.entries.len() {
            let e = &mut self.entries[i];
            if !expired(e.since, now, HOLD_DEADLINE) {
                i += 1;
                continue;
            }
            if e.reported {
                let e = self.entries.remove(i);
                refuse.push(e.conn);
            } else {
                e.reported = true;
                self.lapsed.push(e.ticket);
                i += 1;
            }
        }
        refuse
    }

    /// Tickets whose hold is over since the last call: past the deadline (an
    /// answer this tick still reaches the client; the next sweep refuses it)
    /// or lost their client (nothing to answer; `take` returns `None`).
    pub fn take_expired(&mut self) -> Vec<Ticket> {
        std::mem::take(&mut self.lapsed)
    }

    /// Every held connection, for a `Drop` that must close them by hand.
    pub fn drain(&mut self) -> Vec<T> {
        self.entries.drain(..).map(|e| e.conn).collect()
    }
}

#[cfg(test)]
mod tests;
