//! The rule the custodian sends a redeemed frog by (docs/v3/wrapper.md, "Steps 2 and 3").
//!
//! A redemption's Bitcoin delivery and a refund on chain 4663 can both happen if the delivery is sent near the deadline: the
//! redeemer then has the frog AND the token, and the supply is backed by a frog the vault no longer holds. No contract can see
//! Bitcoin, so no contract can close this. This module is the canister's half: given the ticket as read from 4663 immediately
//! before sending, it answers **Send**, **Wait** or **Refuse**, and it is the last thing consulted before a broadcast.
//!
//! Pure: no network, no replica, no clock of its own. Every time is passed in, so every margin is tested here.
//!
//! Not wired into the buyer's loop yet: the loop and the Bitcoin send are other work. When they are, this is called with a
//! read made through `rpc_view` (every provider agreeing), and a `Send` is the only answer that may reach the broadcast.

/// `Wait.blocksFor`'s divisor, measured on 4663's parent chain (src/lib/Wait.sol). Rounded down, so a block count errs long.
pub const PARENT_BLOCK_SECONDS: u64 = 12;
/// The margin, on either clock: stop sending a day before the deadline.
pub const MARGIN_SECS: u64 = 24 * 60 * 60;
/// `Wait.blocksFor(MARGIN_SECS)`: 7,200 parent blocks.
pub const MARGIN_BLOCKS: u64 = MARGIN_SECS / PARENT_BLOCK_SECONDS;
/// "Immediately before sending", as a number: a read older than this is read again.
pub const MAX_READ_AGE_SECS: u64 = 60;

/// `Wrapper.Stage`, as read from chain 4663. `None` is a ticket that does not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum Stage {
    None,
    Pinned,
    Resolved,
    Delivered,
    Refunded,
}

/// The ticket, as read from 4663, and when.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct TicketRead {
    pub stage: Stage,
    /// The frog the ticket resolved to (`Ticket.acquisitionId`), as the vault numbers it.
    pub acquisition_id: u128,
    /// `Ticket.destination`, the taproot address fixed at the burn.
    pub destination: String,
    /// `Ticket.deliverBy`: the chain's timestamp after which, with the block clock, anyone may refund.
    pub deliver_by: u64,
    /// `Ticket.deliverByBlock`: the parent-chain block after which, with the timestamp, anyone may refund.
    pub deliver_by_block: u64,
    /// The chain's own latest block timestamp at the read (the sequencer's clock).
    pub chain_time: u64,
    /// The parent chain's block number at the read (`block.number` on Nitro; the sequencer-independent clock).
    pub parent_block: u64,
    /// When the canister made the read, on its own clock, in seconds.
    pub read_at: u64,
}

/// What the canister is about to broadcast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub acquisition_id: u128,
    pub destination: String,
}

/// Why a send is refused. Every one of these is final for this ticket: nothing about it gets better by waiting.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum Refusal {
    /// There is no such ticket.
    NoTicket,
    /// Already delivered: sending again sends a second frog nobody is owed.
    Delivered,
    /// Refunded: the redeemer has their token back, and the frog belongs to the set again.
    Refunded,
    /// The ticket names a different frog or address from the transaction about to go out.
    Mismatch { field: &'static str },
    /// A resolved ticket with no deadline: a read that cannot be the contract's.
    Malformed,
    /// Within a day of the deadline on one of the clocks. The ticket is left to refund.
    InsideMargin { secs_left: u64, blocks_left: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Send,
    /// Nothing is wrong yet; read again. `why` says what to wait for.
    Wait { why: &'static str },
    Refuse(Refusal),
}

/// The decision, given the ticket read just now, what is about to be sent, and the canister's clock (`now`, seconds).
pub fn decide(t: &TicketRead, intent: &Intent, now: u64) -> Decision {
    // 1. The stage. Only a resolved ticket is owed a frog.
    match t.stage {
        Stage::None => return Decision::Refuse(Refusal::NoTicket),
        Stage::Delivered => return Decision::Refuse(Refusal::Delivered),
        Stage::Refunded => return Decision::Refuse(Refusal::Refunded),
        Stage::Pinned => return Decision::Wait { why: "not drawn yet: the ticket's round is still unproven" },
        Stage::Resolved => {}
    }
    // 2. The ticket names what is about to go out.
    if t.acquisition_id != intent.acquisition_id {
        return Decision::Refuse(Refusal::Mismatch { field: "acquisition_id" });
    }
    if t.destination != intent.destination {
        return Decision::Refuse(Refusal::Mismatch { field: "destination" });
    }
    if t.deliver_by == 0 || t.deliver_by_block == 0 {
        return Decision::Refuse(Refusal::Malformed);
    }
    // 3. The read is fresh: no older than a minute, and not from the canister's future.
    if t.read_at > now {
        return Decision::Wait { why: "the read is dated after now: read again" };
    }
    let age = now - t.read_at;
    if age > MAX_READ_AGE_SECS {
        return Decision::Wait { why: "the read is more than a minute old: read again immediately before sending" };
    }
    // 4. The margin, on EITHER clock, after taking off the time since the read. Refund needs both clocks past the deadline;
    //    stopping when either is within a day of it is the stricter rule, and the nearer deadline is the one that binds.
    let secs_left = t.deliver_by.saturating_sub(t.chain_time).saturating_sub(age);
    let blocks_left = t.deliver_by_block.saturating_sub(t.parent_block).saturating_sub(age.div_ceil(PARENT_BLOCK_SECONDS));
    if secs_left <= MARGIN_SECS || blocks_left <= MARGIN_BLOCKS {
        return Decision::Refuse(Refusal::InsideMargin { secs_left, blocks_left });
    }
    Decision::Send
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 24 * 60 * 60;
    const T0: u64 = 1_800_000_000;
    const B0: u64 = 26_000_000;
    const ADDR: &str = "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0";

    /// A ticket resolved at T0, so its deadline is 7 days on (the wrapper's DELIVERY_DEADLINE), read at `at` seconds later.
    fn read(at: u64) -> TicketRead {
        TicketRead {
            stage: Stage::Resolved,
            acquisition_id: 7,
            destination: ADDR.into(),
            deliver_by: T0 + 7 * DAY,
            deliver_by_block: B0 + 7 * DAY / PARENT_BLOCK_SECONDS,
            chain_time: T0 + at,
            parent_block: B0 + at / PARENT_BLOCK_SECONDS,
            read_at: T0 + at,
        }
    }
    fn intent() -> Intent { Intent { acquisition_id: 7, destination: ADDR.into() } }

    #[test]
    fn a_fresh_read_of_a_resolved_ticket_well_before_its_deadline_sends() {
        assert_eq!(decide(&read(0), &intent(), T0), Decision::Send);
        assert_eq!(decide(&read(5 * DAY), &intent(), T0 + 5 * DAY + 30), Decision::Send, "six days in, read 30 s ago");
    }

    #[test]
    fn only_a_resolved_ticket_can_send() {
        let with = |s: Stage| { let mut t = read(0); t.stage = s; decide(&t, &intent(), T0) };
        assert!(matches!(with(Stage::Pinned), Decision::Wait { .. }), "not drawn yet: wait, don't refuse");
        assert_eq!(with(Stage::Delivered), Decision::Refuse(Refusal::Delivered));
        assert_eq!(with(Stage::Refunded), Decision::Refuse(Refusal::Refunded));
        assert_eq!(with(Stage::None), Decision::Refuse(Refusal::NoTicket));
    }

    #[test]
    fn the_ticket_must_name_the_frog_and_the_address_about_to_go_out() {
        let mut other = intent();
        other.acquisition_id = 8;
        assert_eq!(decide(&read(0), &other, T0), Decision::Refuse(Refusal::Mismatch { field: "acquisition_id" }));
        let mut elsewhere = intent();
        elsewhere.destination = "bc1pqqqq".into();
        assert_eq!(decide(&read(0), &elsewhere, T0), Decision::Refuse(Refusal::Mismatch { field: "destination" }));
        let mut blank = read(0);
        blank.deliver_by = 0;
        assert_eq!(decide(&blank, &intent(), T0), Decision::Refuse(Refusal::Malformed));
    }

    #[test]
    fn a_read_older_than_a_minute_or_from_the_future_is_read_again() {
        assert_eq!(decide(&read(0), &intent(), T0 + MAX_READ_AGE_SECS), Decision::Send, "exactly a minute: still fresh");
        assert!(matches!(decide(&read(0), &intent(), T0 + MAX_READ_AGE_SECS + 1), Decision::Wait { .. }));
        assert!(matches!(decide(&read(10), &intent(), T0), Decision::Wait { .. }), "a read dated after now");
    }

    #[test]
    fn the_last_day_on_the_timestamp_clock_refuses() {
        // six days and one second in: 23:59:59 left on both clocks
        let t = read(6 * DAY + 1);
        assert!(matches!(decide(&t, &intent(), t.read_at), Decision::Refuse(Refusal::InsideMargin { .. })));
        // exactly a day left is inside: the margin is "at most a day", so the boundary refuses
        let t = read(6 * DAY);
        assert!(matches!(decide(&t, &intent(), t.read_at), Decision::Refuse(Refusal::InsideMargin { .. })));
        // a second more than a day left, on both clocks, sends
        let mut t = read(6 * DAY - PARENT_BLOCK_SECONDS);
        t.parent_block = B0 + (6 * DAY - PARENT_BLOCK_SECONDS) / PARENT_BLOCK_SECONDS - 1;
        assert_eq!(decide(&t, &intent(), t.read_at), Decision::Send);
    }

    #[test]
    fn either_clock_alone_stops_the_send_whichever_is_nearer() {
        // the sequencer stamps an hour ahead: its clock reads the deadline nearer, the parent chain's does not
        let mut ahead = read(5 * DAY);
        ahead.chain_time = T0 + 6 * DAY + 1;
        assert!(matches!(decide(&ahead, &intent(), ahead.read_at), Decision::Refuse(Refusal::InsideMargin { .. })),
            "the timestamp alone within a day: refuse");
        // the parent chain has moved faster than 12 s a block: its clock is the nearer one
        let mut fast = read(5 * DAY);
        fast.parent_block = fast.deliver_by_block - MARGIN_BLOCKS;
        assert!(matches!(decide(&fast, &intent(), fast.read_at), Decision::Refuse(Refusal::InsideMargin { .. })),
            "the block clock alone within a day: refuse");
    }

    #[test]
    fn the_seconds_since_the_read_come_off_the_margin() {
        // read with a day and 30 s left on both clocks; decided 45 s later, so less than a day remains
        let mut t = read(6 * DAY - 30);
        t.parent_block = t.deliver_by_block - MARGIN_BLOCKS - 3;           // 36 s of blocks to spare at the read
        assert_eq!(decide(&t, &intent(), t.read_at), Decision::Send, "at the read: a day and 30 s");
        assert!(matches!(decide(&t, &intent(), t.read_at + 45), Decision::Refuse(Refusal::InsideMargin { .. })),
            "45 s later the timestamp margin is gone");
    }

    #[test]
    fn past_the_deadline_is_inside_the_margin_and_never_underflows() {
        let mut late = read(8 * DAY);
        late.parent_block = late.deliver_by_block + 10_000;
        assert_eq!(decide(&late, &intent(), late.read_at), Decision::Refuse(Refusal::InsideMargin { secs_left: 0, blocks_left: 0 }));
    }

    #[test]
    fn a_refusal_for_the_margin_is_final_for_the_ticket() {
        // nothing a later read can say about the same ticket turns an InsideMargin into a Send: time only moves forward
        let first = read(6 * DAY + 60);
        assert!(matches!(decide(&first, &intent(), first.read_at), Decision::Refuse(Refusal::InsideMargin { .. })));
        for later in [6 * DAY + 120, 6 * DAY + 3_600, 7 * DAY] {
            let t = read(later);
            assert!(matches!(decide(&t, &intent(), t.read_at), Decision::Refuse(_)), "{later}");
        }
    }

    #[test]
    fn the_margin_in_blocks_is_wait_blocks_for_a_day() {
        assert_eq!(MARGIN_BLOCKS, 7_200);
        assert_eq!(MARGIN_SECS, 86_400);
    }
}
