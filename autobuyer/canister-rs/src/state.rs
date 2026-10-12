//! The purchase state machine: what the buyer is doing, and what it does when it wakes up in the middle of it.
//!
//! The rule from `docs/v2/state-machine.md`: nothing that spends is attempted until the intent to spend is durably recorded,
//! and every spend is idempotent against something the outside world already holds. This module is pure — it holds no keys,
//! makes no calls, and decides only what the next act is — so every interrupted transition can be tested by building the
//! state a crash would have left and asking what happens next.

use crate::psbt::Quote;
use crate::runestone::RuneId;

/// What the two chains say, gathered before deciding. The canister's own memory is never the source of truth: on any doubt
/// it re-reads 4663 and Bitcoin and believes them over itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Observed {
    /// The vault's own state for this purchase, as `eth_getLogs` reports it.
    pub vault: VaultState,
    /// ETH at our EVM address, in wei.
    pub evm_balance_wei: u128,
    /// A bridge transaction we signed, if the chain has it.
    pub bridge_tx_mined: Option<bool>,
    /// Confirmed sats at our taproot address.
    pub btc_sats: u64,
    /// Bitcoin has this txid, and it is confirmed.
    pub btc_tx_confirmed: Option<bool>,
    /// Bitcoin shows the asset at the recipient's script.
    pub delivered_units: u128,
    /// Seconds since the epoch, as the canister's time reports it.
    pub now: u64,
    /// The vault's `readyAt` for this purchase, when it has one: what a lost `announce` reply is rebuilt from.
    pub vault_ready_at: u64,
    /// The vault's `deadline` for this purchase, when it has been pulled.
    pub vault_deadline: u64,
}

/// The vault's `State` enum, as read from chain 4663.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum VaultState {
    #[default]
    None,
    Announced,
    Pulled,
    Settled,
    Returned,
    WrittenOff,
}

/// Where the buyer believes it is. Every variant carries exactly what a restart needs and nothing more.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum Phase {
    Watching,
    Announced { ready_at: u64 },
    Pulled { deadline: u64 },
    Bridging { sent_tx: [u8; 32], expect_sats: u64, give_up_at: u64 },
    Funded,
    /// The quote, recorded before any PSBT is asked for, so the venue cannot influence what it is checked against.
    /// `signing`: the transaction the first signature was for. Every later signature must be for the same one, so one
    /// quote can never pay for two transactions.
    Quoted { quote: Quote, signing: Option<[u8; 32]> },
    /// The bytes are saved **before** anything is broadcast, so a restart re-broadcasts the same transaction.
    Signed { txid: [u8; 32], raw_tx: Vec<u8>, quote: Quote },
    Broadcast { txid: [u8; 32], quote: Quote },
    Delivered { txid: [u8; 32], rune: RuneId, units: u128, sats: u64 },
    Settled,
    Returning,
    Returned,
    Stuck { why: Stuck },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum Stuck {
    /// Bridged ETH that never arrived as BTC.
    Bridge,
    /// A signed transaction that never confirmed before the deadline.
    Delivery,
    /// The venue would not produce a PSBT we could accept.
    Venue,
    /// One of our own EVM calls was mined and reverted (`docs/v2/buyer-loop.md`): a buyer that keeps re-signing a call the
    /// vault rejects spends gas for ever, so it stops and the owner looks.
    Reverted,
    /// A venue or bridge that was configured when the purchase was announced is no longer there. Nothing is improvised.
    Unconfigured,
    /// The vault's record of this purchase is not what we announced (another amount, another quantity).
    Mismatch,
}

/// What a purchase is for: the rune, how many of its smallest units, and the script the asset must land on. Fixed when the
/// operator opens the purchase; every quote is checked against it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Order {
    /// `block:tx`, as the listing names it.
    pub rune: String,
    pub units: u128,
    pub recipient_script: Vec<u8>,
    /// Where the cap came from, for the vault's `announce`.
    pub cap_ref: String,
}

/// Which of the vault calls (or the bridge's) a signed EVM transaction is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum EvmStep {
    Announce,
    Pull,
    Bridge { expect_sats: u64 },
    Settle,
    Return,
}

/// One signed EVM transaction of ours, saved **before** it was sent. Sent again, the same bytes, until the chain says what
/// happened to it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct PendingEvm {
    pub step: EvmStep,
    pub raw: Vec<u8>,
    pub hash: [u8; 32],
    pub nonce: u64,
}

/// One purchase, keyed by the id the **vault** assigned. The buyer never invents an id.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Purchase {
    pub id: u64,
    pub phase: Phase,
    /// What the vault reserved, in wei.
    pub amount_wei: u128,
    /// What to buy. `None` only on rehearsal purchases, which the loop does not quote.
    pub order: Option<Order>,
    /// Our signed EVM transaction, if one is out. At most one across all purchases (`docs/v2/buyer-loop.md`).
    pub pending_evm: Option<PendingEvm>,
}

/// What to do next. The caller performs it; this module never does.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum Act {
    /// Nothing to do now.
    Wait,
    /// Ask the vault to reserve `amount_wei` and start the public hour.
    Announce { amount_wei: u128 },
    /// Take the reserved ETH to our own address.
    Pull { id: u64 },
    /// Get BTC for the ETH we hold.
    Bridge { id: u64, amount_wei: u128 },
    /// Ask the venue what the asset costs, and record it before asking for a PSBT.
    GetQuote { id: u64 },
    /// Ask for a PSBT and judge it against `quote`.
    RequestAndVerifyPsbt { id: u64, quote: Quote },
    /// Send the saved bytes. Safe to repeat: the same inputs can be spent once.
    Broadcast { id: u64, txid: [u8; 32], raw_tx: Vec<u8> },
    /// Read Bitcoin to find out what really happened.
    ReadBackBitcoin { id: u64, txid: [u8; 32] },
    /// Post the receipt on 4663.
    Settle { id: u64, txid: [u8; 32], rune: RuneId, units: u128, sats: u64 },
    /// Send the vault its ETH back.
    ReturnFunds { id: u64, amount_wei: u128 },
    /// Stop, loudly. The owner writes off, in public.
    Halt { id: u64, why: Stuck },
}

impl Purchase {
    /// Reconcile belief with the chains, then say what to do. This is the whole restart story: the caller loads whatever was
    /// in stable memory, gathers [`Observed`], and calls this. It never needs to know how it got here.
    pub fn next(&mut self, o: &Observed) -> Act {
        // 1. The chain's own record overrides ours wherever it is further along. A reply we never saw is not a reason to
        //    repeat a call that already happened.
        match (o.vault, &self.phase) {
            (VaultState::Settled, p) if !matches!(p, Phase::Settled) => {
                self.phase = Phase::Settled;
                return Act::Wait;
            }
            (VaultState::Returned | VaultState::WrittenOff, p) if !matches!(p, Phase::Returned) => {
                self.phase = Phase::Returned;
                return Act::Wait;
            }
            // We announced and never saw the reply; the vault's own record is adopted (state-machine.md, "announce sent,
            // reply lost"), never a second announcement.
            (VaultState::Announced, Phase::Watching) => {
                self.phase = Phase::Announced { ready_at: o.vault_ready_at };
            }
            // We asked to pull and never saw the reply; the vault says it happened. Its own deadline is used when it gave
            // one, less a margin so that we act before the vault calls it overdue.
            (VaultState::Pulled, Phase::Announced { .. } | Phase::Watching) => {
                let deadline = if o.vault_deadline > 0 { o.vault_deadline.saturating_sub(DEADLINE_MARGIN_SECS) }
                    else { o.now + SETTLE_WINDOW_SECS };
                self.phase = Phase::Pulled { deadline };
            }
            _ => {}
        }

        match self.phase.clone() {
            Phase::Watching => Act::Announce { amount_wei: self.amount_wei },

            Phase::Announced { ready_at } => {
                if o.vault != VaultState::Announced {
                    // The vault does not agree that anything is reserved: start again rather than pull into the dark.
                    self.phase = Phase::Watching;
                    return Act::Wait;
                }
                if o.now < ready_at {
                    Act::Wait
                } else {
                    Act::Pull { id: self.id }
                }
            }

            Phase::Pulled { deadline } => {
                if o.now > deadline {
                    self.phase = Phase::Returning;
                    return Act::ReturnFunds { id: self.id, amount_wei: self.amount_wei };
                }
                if o.btc_sats > 0 {
                    // Bitcoin already has what we need: whatever happened to the bridge call, we are funded.
                    self.phase = Phase::Funded;
                    return Act::GetQuote { id: self.id };
                }
                Act::Bridge { id: self.id, amount_wei: self.amount_wei }
            }

            Phase::Bridging { expect_sats, give_up_at, .. } => {
                // Watch Bitcoin, never the bridge's own word.
                if o.btc_sats >= expect_sats {
                    self.phase = Phase::Funded;
                    Act::GetQuote { id: self.id }
                } else if o.now > give_up_at {
                    self.phase = Phase::Stuck { why: Stuck::Bridge };
                    Act::Halt { id: self.id, why: Stuck::Bridge }
                } else {
                    Act::Wait
                }
            }

            Phase::Funded => Act::GetQuote { id: self.id },

            Phase::Quoted { quote, .. } => Act::RequestAndVerifyPsbt { id: self.id, quote },

            // The signature exists and the bytes are saved. Send them, however many times it takes.
            Phase::Signed { txid, raw_tx, .. } => Act::Broadcast { id: self.id, txid, raw_tx },

            Phase::Broadcast { txid, quote } => {
                if o.delivered_units >= quote.units && o.btc_tx_confirmed == Some(true) {
                    let sats = quote.max_spend_sats;
                    self.phase = Phase::Delivered { txid, rune: quote.rune, units: o.delivered_units, sats };
                    Act::Settle { id: self.id, txid, rune: quote.rune, units: o.delivered_units, sats }
                } else if o.btc_tx_confirmed == Some(false) {
                    // Bitcoin rejected it outright (its inputs went to another transaction): nothing of ours was spent, so try
                    // again from the quote. `None` -- not seen yet -- is not this. (It read `o.now > SETTLE_WINDOW_SECS +
                    // o.now.saturating_sub(o.now)` before, which is `o.now > 259200`: true for any real clock.)
                    self.phase = Phase::Funded;
                    Act::GetQuote { id: self.id }
                } else {
                    Act::ReadBackBitcoin { id: self.id, txid }
                }
            }

            Phase::Delivered { txid, rune, units, sats } => Act::Settle { id: self.id, txid, rune, units, sats },

            Phase::Returning => Act::ReturnFunds { id: self.id, amount_wei: self.amount_wei },

            Phase::Settled | Phase::Returned => Act::Wait,

            Phase::Stuck { why } => Act::Halt { id: self.id, why },
        }
    }
}

/// The vault's own settle window, mirrored here so a restart can rebuild a deadline it never saw.
pub const SETTLE_WINDOW_SECS: u64 = 3 * 24 * 60 * 60;
/// How long before the vault's own deadline the buyer gives up and returns: the canister's clock and the sequencer's are not
/// the same clock, and returning a minute late is an overdue purchase that blocks the vault.
pub const DEADLINE_MARGIN_SECS: u64 = 60 * 60;

impl Purchase {
    /// Record the quote this purchase will be checked against. Only from `Funded`, and only once: a quote that could be
    /// replaced after the venue has seen it is a quote the venue can steer. (`docs/v2/state-machine.md`; audit B3's
    /// remaining half.)
    pub fn record_quote(&mut self, quote: Quote) -> Result<(), String> {
        match &self.phase {
            Phase::Funded => {
                self.phase = Phase::Quoted { quote, signing: None };
                Ok(())
            }
            Phase::Quoted { .. } => Err(format!("refused: purchase {} already has a recorded quote, and it is not replaced", self.id)),
            other => Err(format!("refused: purchase {} is {other:?}; a quote is recorded only once it is funded", self.id)),
        }
    }

    /// The recorded quote, for signing an input of transaction `txid` — refused if this purchase has already signed for a
    /// different transaction, or has no recorded quote.
    pub fn quote_to_sign(&self, txid: &[u8; 32]) -> Result<Quote, String> {
        match &self.phase {
            Phase::Quoted { quote, signing: None } => Ok(quote.clone()),
            Phase::Quoted { quote, signing: Some(t) } if t == txid => Ok(quote.clone()),
            Phase::Quoted { signing: Some(t), .. } => Err(format!(
                "refused: purchase {} is already signing transaction {}; one quote pays for one transaction", self.id, hex::encode(t))),
            other => Err(format!("refused: purchase {} is {other:?}; it signs only against a recorded quote", self.id)),
        }
    }

    /// Bind the purchase to `txid`. Called after the checks pass and **before** the signature is awaited, so two calls in
    /// flight at once cannot both pass `quote_to_sign` for different transactions.
    pub fn bind(&mut self, txid: [u8; 32]) {
        if let Phase::Quoted { signing, .. } = &mut self.phase {
            signing.get_or_insert(txid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote() -> Quote {
        Quote {
            rune: RuneId { block: 840_000, tx: 3 },
            units: 1_000,
            max_spend_sats: 50_000,
            max_fee_sats: 5_000,
            recipient_script: vec![0x51, 0x20, 2],
            our_script: vec![0x51, 0x20, 1],
        }
    }

    fn p(phase: Phase) -> Purchase {
        Purchase { id: 7, phase, amount_wei: 1_000_000_000_000_000_000, order: None, pending_evm: None }
    }

    fn seen(vault: VaultState, now: u64) -> Observed {
        Observed { vault, now, ..Default::default() }
    }

    #[test]
    fn a_fresh_purchase_announces() {
        let mut x = p(Phase::Watching);
        assert_eq!(x.next(&seen(VaultState::None, 100)), Act::Announce { amount_wei: x.amount_wei });
    }

    #[test]
    fn interrupted_after_announce_the_reply_was_lost_so_the_chain_is_believed() {
        // We recorded nothing, but the vault's log shows the reservation. The id came from the vault, so we adopt it.
        let mut x = p(Phase::Announced { ready_at: 1_000 });
        assert_eq!(x.next(&seen(VaultState::Announced, 500)), Act::Wait, "the public hour is not over");
        assert_eq!(x.next(&seen(VaultState::Announced, 1_000)), Act::Pull { id: 7 });
    }

    #[test]
    fn interrupted_between_pull_and_its_reply_the_vault_says_it_happened() {
        let mut x = p(Phase::Announced { ready_at: 1_000 });
        let act = x.next(&seen(VaultState::Pulled, 2_000));
        assert!(matches!(x.phase, Phase::Pulled { .. }), "adopted from the chain, not from memory");
        assert_eq!(act, Act::Bridge { id: 7, amount_wei: x.amount_wei }, "and it moves on rather than pulling twice");
    }

    #[test]
    fn an_announcement_the_vault_does_not_know_about_is_abandoned_not_pulled() {
        let mut x = p(Phase::Announced { ready_at: 1_000 });
        assert_eq!(x.next(&seen(VaultState::None, 5_000)), Act::Wait);
        assert_eq!(x.phase, Phase::Watching, "it starts again rather than pulling into the dark");
    }

    #[test]
    fn interrupted_after_the_bridge_tx_but_before_btc_arrives_it_waits_then_gives_up() {
        let mut x = p(Phase::Bridging { sent_tx: [9u8; 32], expect_sats: 100_000, give_up_at: 5_000 });
        let mut o = seen(VaultState::Pulled, 1_000);
        assert_eq!(x.next(&o), Act::Wait);
        o.now = 6_000;
        assert_eq!(x.next(&o), Act::Halt { id: 7, why: Stuck::Bridge });
        assert_eq!(x.phase, Phase::Stuck { why: Stuck::Bridge });
    }

    #[test]
    fn the_bridge_is_believed_only_when_bitcoin_says_so() {
        let mut x = p(Phase::Bridging { sent_tx: [9u8; 32], expect_sats: 100_000, give_up_at: 5_000 });
        let mut o = seen(VaultState::Pulled, 1_000);
        o.btc_sats = 99_999;
        assert_eq!(x.next(&o), Act::Wait, "short of what we expect is not arrival");
        o.btc_sats = 100_000;
        assert_eq!(x.next(&o), Act::GetQuote { id: 7 });
        assert_eq!(x.phase, Phase::Funded);
    }

    #[test]
    fn a_pulled_purchase_that_already_has_btc_does_not_bridge_again() {
        // The crash happened between broadcasting the bridge transaction and recording it.
        let mut x = p(Phase::Pulled { deadline: 100_000 });
        let mut o = seen(VaultState::Pulled, 1_000);
        o.btc_sats = 100_000;
        assert_eq!(x.next(&o), Act::GetQuote { id: 7 }, "Bitcoin already has it, whatever we remember");
    }

    #[test]
    fn interrupted_after_signing_but_before_broadcast_it_sends_the_saved_bytes() {
        let raw = vec![1, 2, 3, 4];
        let mut x = p(Phase::Signed { txid: [7u8; 32], raw_tx: raw.clone(), quote: quote() });
        assert_eq!(
            x.next(&seen(VaultState::Pulled, 1_000)),
            Act::Broadcast { id: 7, txid: [7u8; 32], raw_tx: raw },
            "the same inputs, so the same txid: re-broadcasting costs nothing and cannot double-spend"
        );
    }

    #[test]
    fn interrupted_after_broadcast_it_asks_bitcoin_rather_than_the_venue() {
        let mut x = p(Phase::Broadcast { txid: [7u8; 32], quote: quote() });
        let o = seen(VaultState::Pulled, 1_000);
        assert_eq!(x.next(&o), Act::ReadBackBitcoin { id: 7, txid: [7u8; 32] });
    }

    #[test]
    fn delivery_is_recognised_only_when_bitcoin_shows_the_units_and_a_confirmation() {
        let mut x = p(Phase::Broadcast { txid: [7u8; 32], quote: quote() });
        let mut o = seen(VaultState::Pulled, 1_000);
        o.delivered_units = 1_000;
        o.btc_tx_confirmed = None;
        assert_eq!(x.next(&o), Act::ReadBackBitcoin { id: 7, txid: [7u8; 32] }, "units seen but unconfirmed is not delivery");
        o.btc_tx_confirmed = Some(true);
        assert_eq!(
            x.next(&o),
            Act::Settle { id: 7, txid: [7u8; 32], rune: RuneId { block: 840_000, tx: 3 }, units: 1_000, sats: 50_000 }
        );
    }

    #[test]
    fn interrupted_between_delivery_and_settle_it_settles() {
        let mut x = p(Phase::Delivered {
            txid: [7u8; 32],
            rune: RuneId { block: 840_000, tx: 3 },
            units: 1_000,
            sats: 40_000,
        });
        assert_eq!(
            x.next(&seen(VaultState::Pulled, 1_000)),
            Act::Settle { id: 7, txid: [7u8; 32], rune: RuneId { block: 840_000, tx: 3 }, units: 1_000, sats: 40_000 }
        );
    }

    #[test]
    fn interrupted_between_settle_and_its_reply_the_vault_says_it_is_done() {
        let mut x = p(Phase::Delivered {
            txid: [7u8; 32],
            rune: RuneId { block: 840_000, tx: 3 },
            units: 1_000,
            sats: 40_000,
        });
        assert_eq!(x.next(&seen(VaultState::Settled, 1_000)), Act::Wait);
        assert_eq!(x.phase, Phase::Settled, "and it never settles twice");
    }

    #[test]
    fn a_pulled_purchase_past_its_deadline_returns_the_money() {
        let mut x = p(Phase::Pulled { deadline: 1_000 });
        assert_eq!(x.next(&seen(VaultState::Pulled, 1_001)), Act::ReturnFunds { id: 7, amount_wei: x.amount_wei });
        assert_eq!(x.phase, Phase::Returning);
    }

    #[test]
    fn a_returned_purchase_is_terminal_however_it_got_there() {
        for v in [VaultState::Returned, VaultState::WrittenOff] {
            let mut x = p(Phase::Returning);
            assert_eq!(x.next(&seen(v, 1_000)), Act::Wait);
            assert_eq!(x.phase, Phase::Returned);
        }
    }

    #[test]
    fn terminal_states_never_act_again() {
        for (phase, vault) in [(Phase::Settled, VaultState::Settled), (Phase::Returned, VaultState::Returned)] {
            let mut x = p(phase.clone());
            for now in [1u64, 1_000, 10_000_000] {
                assert_eq!(x.next(&seen(vault, now)), Act::Wait);
                assert_eq!(x.phase, phase, "and it does not leave");
            }
        }
    }

    #[test]
    fn replaying_the_same_state_many_times_never_produces_a_second_spend() {
        // The property behind every row of the table: calling next() repeatedly without the world changing asks for the same
        // act, and asking for it twice is safe because each act is idempotent against the chain it touches.
        let cases = vec![
            (Phase::Signed { txid: [1u8; 32], raw_tx: vec![9], quote: quote() }, VaultState::Pulled),
            (Phase::Broadcast { txid: [1u8; 32], quote: quote() }, VaultState::Pulled),
            (Phase::Returning, VaultState::Pulled),
            (Phase::Watching, VaultState::None),
        ];
        for (phase, vault) in cases {
            let mut x = p(phase);
            let first = x.next(&seen(vault, 5_000));
            for _ in 0..5 {
                assert_eq!(x.next(&seen(vault, 5_000)), first, "the same state asks for the same act");
            }
        }
    }

    #[test]
    fn a_quote_is_recorded_once_and_only_once_funded() {
        let mut x = p(Phase::Funded);
        assert_eq!(x.record_quote(quote()), Ok(()));
        assert_eq!(x.phase, Phase::Quoted { quote: quote(), signing: None });
        let mut steered = quote();
        steered.max_spend_sats *= 10;
        assert!(x.record_quote(steered).unwrap_err().contains("already has a recorded quote"), "a venue cannot re-quote");
        assert_eq!(x.phase, Phase::Quoted { quote: quote(), signing: None }, "and the refusal changes nothing");
        for ph in [Phase::Watching, Phase::Pulled { deadline: 9 }, Phase::Settled] {
            assert!(p(ph).record_quote(quote()).unwrap_err().contains("only once it is funded"));
        }
        // the loop itself still reads the recorded quote
        assert_eq!(x.next(&seen(VaultState::Pulled, 1)), Act::RequestAndVerifyPsbt { id: 7, quote: quote() });
    }

    #[test]
    fn one_quote_signs_for_one_transaction() {
        let (a, b) = ([0xaa; 32], [0xbb; 32]);
        let mut x = p(Phase::Funded);
        assert!(x.quote_to_sign(&a).unwrap_err().contains("only against a recorded quote"), "no quote, no signature");
        x.record_quote(quote()).unwrap();
        assert_eq!(x.quote_to_sign(&a), Ok(quote()));
        x.bind(a);
        assert_eq!(x.quote_to_sign(&a), Ok(quote()), "another input of the same transaction");
        assert!(x.quote_to_sign(&b).unwrap_err().contains("already signing transaction aaaa"), "a second transaction is refused");
        x.bind(b);
        assert_eq!(x.phase, Phase::Quoted { quote: quote(), signing: Some(a) }, "binding again never moves it");
    }
}
