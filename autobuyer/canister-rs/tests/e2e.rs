//! The whole loop, hop by hop: a purchase from the moment a vault can afford one to the receipt on chain 4663.
//!
//! It runs against fixtures for the two hops that need somebody else's permission or money — the bridge and the venue — and
//! against the real verifier, the real runestone decoder and the real state machine for everything else. Every hop prints,
//! so `cargo test --test e2e -- --nocapture` reads as an account of what the buyer did and why.
//!
//! What this does **not** prove is stated at the end of the run, on screen, so nobody mistakes a green test for a live one.

use etch_autobuyer::bridge::{self, Bridge, BridgeQuote, Fixture as BridgeFixture};
use etch_autobuyer::psbt::{Psbt, Quote, Refusal};
use etch_autobuyer::runestone::RuneId;
use etch_autobuyer::state::{Act, Observed, Phase, Purchase, VaultState};
use etch_autobuyer::venue::{Fixture as VenueFixture, Venue};

const OUR: &[u8] = &[0x51, 0x20, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1];
const WINNER: &[u8] = &[0x51, 0x20, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2];
const SELLER: &[u8] = &[0x00, 0x14, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7];
const STRANGER: &[u8] = &[0x51, 0x20, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9];
const RUNE: RuneId = RuneId { block: 840_000, tx: 3 };
const ONE_ETH: u128 = 1_000_000_000_000_000_000;

fn leb(mut v: u128) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return out;
        }
        out.push(b | 0x80);
    }
}

fn stone(amount: u128, to: u128) -> Vec<u8> {
    let mut p = leb(0);
    p.extend(leb(RUNE.block as u128));
    p.extend(leb(RUNE.tx as u128));
    p.extend(leb(amount));
    p.extend(leb(to));
    let mut s = vec![0x6a, 0x5d, p.len() as u8];
    s.extend(p);
    s
}

fn compact(n: u64) -> Vec<u8> {
    if n <= 0xfc {
        vec![n as u8]
    } else {
        let mut v = vec![0xfd];
        v.extend_from_slice(&(n as u16).to_le_bytes());
        v
    }
}

fn ser_out(value: u64, script: &[u8]) -> Vec<u8> {
    let mut v = value.to_le_bytes().to_vec();
    v.extend(compact(script.len() as u64));
    v.extend_from_slice(script);
    v
}

/// Build a PSBT the way a venue would return one: one input of ours, and the outputs it proposes.
fn psbt_bytes(in_value: u64, outs: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let mut tx = 2i32.to_le_bytes().to_vec();
    tx.extend(compact(1));
    tx.extend_from_slice(&[1u8; 32]);
    tx.extend_from_slice(&0u32.to_le_bytes());
    tx.push(0);
    tx.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
    tx.extend(compact(outs.len() as u64));
    for (v, s) in outs {
        tx.extend(ser_out(*v, s));
    }
    tx.extend_from_slice(&0u32.to_le_bytes());

    let mut p = b"psbt\xff".to_vec();
    p.push(1);
    p.push(0x00);
    p.extend(compact(tx.len() as u64));
    p.extend_from_slice(&tx);
    p.push(0);
    let u = ser_out(in_value, OUR);
    p.push(1);
    p.push(0x01);
    p.extend(compact(u.len() as u64));
    p.extend_from_slice(&u);
    p.push(0);
    for _ in outs {
        p.push(0);
    }
    p
}

fn quote() -> Quote {
    Quote {
        rune: RUNE,
        units: 1_000,
        max_spend_sats: 50_000,
        max_fee_sats: 5_000,
        recipient_script: WINNER.to_vec(),
        our_script: OUR.to_vec(),
    }
}

fn hop(n: u32, what: &str) {
    println!("  {n:>2}. {what}");
}

#[test]
fn the_whole_loop_from_a_vault_that_can_afford_one_to_a_receipt_on_chain() {
    println!("\n=== etched v2: one purchase, hop by hop ===\n");

    // The recipients are known before anything is bought: the draw has resolved and the winner posted an address. The asset
    // is never bought "for the vault" and therefore never rests with us.
    let mut p = Purchase { id: 41, phase: Phase::Watching, amount_wei: ONE_ETH, order: None, pending_evm: None };
    let mut seen = Observed { vault: VaultState::None, now: 1_000, ..Default::default() };

    let act = p.next(&seen);
    assert_eq!(act, Act::Announce { amount_wei: ONE_ETH });
    hop(1, "the vault can afford it and the winner's address is known → announce, which starts the public hour");

    // The vault reserved it. An hour passes in public, where anyone can see what is about to be bought.
    p.phase = Phase::Announced { ready_at: 4_600 };
    seen.vault = VaultState::Announced;
    assert_eq!(p.next(&seen), Act::Wait);
    hop(2, "inside the public hour → wait. The owner can still pause; nobody can redirect it");

    seen.now = 4_600;
    assert_eq!(p.next(&seen), Act::Pull { id: 41 });
    hop(3, "the hour is up → pull, and the vault can send to one address: the buyer's own");

    // The reply was lost. The vault's own state is believed over our memory, so the pull is not repeated.
    seen.vault = VaultState::Pulled;
    let act = p.next(&seen);
    assert!(matches!(p.phase, Phase::Pulled { .. }));
    assert_eq!(act, Act::Bridge { id: 41, amount_wei: ONE_ETH });
    hop(4, "the pull's reply was lost; the chain says it happened → move on rather than pull twice");

    // The bridge's own offer is judged against our own reference rate before we sign for it.
    let b = BridgeFixture {
        quote: BridgeQuote {
            send_wei: ONE_ETH,
            expect_sats: 100_000,
            to: [0xb4; 20],
            calldata: vec![0xde, 0xad],
            recipient_script: OUR.to_vec(),
        },
    };
    let bq = b.quote(ONE_ETH, OUR).unwrap();
    bridge::check(&bq, ONE_ETH, OUR, &[[0xb4; 20]], 95_000).expect("the bridge's offer clears our own floor");
    assert_eq!(
        bridge::check(&bq, ONE_ETH, OUR, &[[0xff; 20]], 95_000),
        Err(bridge::QuoteRefusal::UnknownContract([0xb4; 20])),
        "and a contract we do not allow is refused"
    );
    hop(5, "bridge quote checked against our own rate floor and an allowlist → sign and send");

    // Only Bitcoin is believed about arrival, never the bridge.
    p.phase = Phase::Bridging { sent_tx: [0xaa; 32], expect_sats: 100_000, give_up_at: 100_000 };
    seen.btc_sats = 99_999;
    assert_eq!(p.next(&seen), Act::Wait);
    hop(6, "the bridge says it paid; Bitcoin shows 99,999 of 100,000 sats → not arrived, so wait");

    seen.btc_sats = 100_000;
    assert_eq!(p.next(&seen), Act::GetQuote { id: 41 });
    hop(7, "Bitcoin shows the sats → funded, ask the venue what the rune costs");

    // The quote is recorded BEFORE the PSBT is asked for. Everything after is judged against it.
    let venue = VenueFixture {
        quote: quote(),
        psbt: psbt_bytes(
            100_000,
            &[
                (0, stone(1_000, 1)),
                (546, WINNER.to_vec()),
                (40_000, SELLER.to_vec()),
                (58_454, OUR.to_vec()),
            ],
        ),
        to_sign: vec![0],
    };
    let q = venue.quote("840000:3", 1_000, WINNER).unwrap();
    assert_eq!(p.phase, Phase::Funded, "a quote is recorded only once the purchase is funded");
    p.record_quote(q.clone()).expect("recorded once");
    assert!(p.record_quote(q.clone()).is_err(), "and never replaced: the venue cannot re-quote after seeing the request");
    assert_eq!(p.next(&seen), Act::RequestAndVerifyPsbt { id: 41, quote: q.clone() });
    hop(8, "quote recorded first: 1,000 units of 840000:3, at most 50,000 sats, to the winner's script");

    // The PSBT arrives over a NON-REPLICATED outcall — one node wrote this response, and it is not trusted.
    let (bytes, to_sign) = venue.psbt(&q).unwrap();
    let parsed = Psbt::parse(&bytes).expect("it is at least a PSBT");
    let signable = parsed.verify(&q, &to_sign).expect("and it does what we asked");
    assert_eq!(signable, vec![0]);
    hop(9, "PSBT verified against the recorded quote: the edict names the winner, the spend is under the cap, change is ours");

    // The same node, lying, in four ways. Each is refused, and the purchase fails rather than going wrong.
    let hostile: Vec<(&str, Vec<u8>, Refusal)> = vec![
        (
            "sends the runes to itself",
            psbt_bytes(
                100_000,
                &[
                    (0, stone(1_000, 2)),
                    (546, WINNER.to_vec()),
                    (546, STRANGER.to_vec()),
                    (40_000, SELLER.to_vec()),
                    (57_908, OUR.to_vec()),
                ],
            ),
            Refusal::AssetNotDelivered { wanted: 1_000, found: 0 },
        ),
        (
            "raises the price past the cap",
            psbt_bytes(
                100_000,
                &[
                    (0, stone(1_000, 1)),
                    (546, WINNER.to_vec()),
                    (78_454, SELLER.to_vec()),
                    (20_000, OUR.to_vec()),
                ],
            ),
            Refusal::OverCap { spend: 80_000, cap: 50_000 },
        ),
        (
            "keeps our change",
            psbt_bytes(
                100_000,
                &[
                    (0, stone(1_000, 1)),
                    (546, WINNER.to_vec()),
                    (40_000, SELLER.to_vec()),
                    (58_454, STRANGER.to_vec()),
                ],
            ),
            Refusal::OverCap { spend: 100_000, cap: 50_000 },
        ),
        (
            "delivers one unit short",
            psbt_bytes(
                100_000,
                &[
                    (0, stone(999, 1)),
                    (546, WINNER.to_vec()),
                    (40_000, SELLER.to_vec()),
                    (58_454, OUR.to_vec()),
                ],
            ),
            Refusal::AssetNotDelivered { wanted: 1_000, found: 999 },
        ),
    ];
    for (what, bytes, expected) in hostile {
        let got = Psbt::parse(&bytes).unwrap().verify(&q, &[0]).unwrap_err();
        assert_eq!(got, expected, "a node that {what}");
        hop(10, &format!("a lying node that {what} → refused: {got:?}"));
    }

    // Signed. The bytes are saved before anything is broadcast, so a crash here re-sends the same transaction.
    let txid = [0x5a; 32];
    p.phase = Phase::Signed { txid, raw_tx: bytes.clone(), quote: q.clone() };
    assert_eq!(p.next(&seen), Act::Broadcast { id: 41, txid, raw_tx: bytes.clone() });
    hop(11, "signed, and the bytes saved BEFORE broadcasting → a crash here re-sends the same transaction, never a second one");

    // The HTTP answer is ignored entirely; Bitcoin is asked instead.
    p.phase = Phase::Broadcast { txid, quote: q.clone() };
    seen.delivered_units = 1_000;
    seen.btc_tx_confirmed = None;
    assert_eq!(p.next(&seen), Act::ReadBackBitcoin { id: 41, txid });
    hop(12, "the venue's reply is ignored; units seen but unconfirmed → keep reading Bitcoin");

    seen.btc_tx_confirmed = Some(true);
    let act = p.next(&seen);
    assert_eq!(act, Act::Settle { id: 41, txid, rune: RUNE, units: 1_000, sats: 50_000 });
    hop(13, "Bitcoin confirms the winner holds 1,000 units → settle, posting the receipt on 4663");

    seen.vault = VaultState::Settled;
    assert_eq!(p.next(&seen), Act::Wait);
    assert_eq!(p.phase, Phase::Settled);
    hop(14, "the vault records the receipt → done. Nothing leaves a terminal state");

    println!("\n  Real here: the runestone decoder, the PSBT verifier, the bridge check, the state machine, and every refusal.");
    println!("  Fixture here: the bridge's reply and the venue's reply. Buying at a venue needs UniSat's written permission");
    println!("  for commercial use, which has been asked for and not received, so no marketplace call was made.");
    println!("  Not proven here at all: a multi-node subnet, a real Bitcoin broadcast, and a real bridge transfer.\n");
}

#[test]
fn a_purchase_that_never_gets_its_btc_freezes_the_vault_rather_than_guessing() {
    let mut p = Purchase { id: 42, phase: Phase::Bridging { sent_tx: [1; 32], expect_sats: 100_000, give_up_at: 5_000 }, amount_wei: ONE_ETH, order: None, pending_evm: None };
    let seen = Observed { vault: VaultState::Pulled, now: 6_000, ..Default::default() };
    assert_eq!(p.next(&seen), Act::Halt { id: 42, why: etch_autobuyer::state::Stuck::Bridge });
    println!("\n  A bridge that never paid halts the buyer and freezes the vault. The owner writes it off in public; no code");
    println!("  can recall money that left this chain, and the design says so rather than implying otherwise.\n");
}
