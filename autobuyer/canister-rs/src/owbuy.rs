//! Buying an Ordinals Wallet **protected** listing (passthrough v4) from the float: the judge (docs/v8/ow-protected.md §3.1).
//!
//! Pure: no network, no keys, no clock. [`judge_protected`] decides whether the float may sign its inputs of a sale PSBT that
//! Ordinals Wallet built, given the passthrough it spends (the parent), the listing as the runner read it, and the facts the
//! canister read itself (the float and custody scripts, ord's location of the inscription, the cap). It is a port of the SDK's
//! `verify_sale` / `verify_setup` held to the canister's own, stricter rules (P1–P10 of the design's table):
//!
//! - the parent spends exactly the located output into `tr(NUMS, {sale(S, C), recovery(S)})` with the **pinned** co-signer `C`,
//!   rebuilt here from the seller key its leaf names (P1, P2, P4);
//! - exactly one sale input spends the parent, with float inputs on both sides of it and nothing else (P3, P5);
//! - the seller is paid exactly `escrow_price` at the escrow's index (P6);
//! - the inscribed sat lands in the **only** custody output, which carries exactly the escrow's sats (P7);
//! - every other output is the float's change, the pinned marketplace fee address or a creator pinned in the install's config
//!   (never the listing's word); the fee and royalties are bounded by the seller's own `escrow_price`, never the API's
//!   `satoshi_price` (P8; protected-listing review P-01, 2026-10-09);
//! - what leaves the float is within the cap and the miner fee within `frogbuy::fee_bound` and, when the canister read a fee
//!   rate, within twice that rate on the sale's estimated size (P9; review P-02);
//! - version 2, locktime 0 on the sale, the setup and the parent, and no relative lock on a float input (review P-06);
//! - a setup, when present, moves the float to two float outputs and the sale is funded by exactly those (P10).
//!
//! The canister's `SIGHASH_DEFAULT` key-path signature commits to every input and output, so the co-signer cannot change a
//! byte of what was judged here: it can only complete the sale or not.

use crate::custody::Outpoint;
use crate::frogbuy::{fee_bound, Seller};
use crate::psbt::{self, InputFields, Psbt};
use crate::taptree::{self, hex32, NUMS, TAPSCRIPT};

/// Ordinals Wallet's co-signer, pinned (`GET /market/secure-purchase/capabilities`). Rotating it requires an upgrade.
pub const COSIGNER: [u8; 32] = hex32("1d08b7c71f6f1e97a0a4cf005db7a977c85e34652a0c9365842aee25997c7dee");
/// Where the marketplace fee is paid. Pinned for the same reason.
pub const MARKET_FEE_ADDRESS: &str = "bc1p6yd49679azsaxqgtr52ff6jjvj2wv5dlaqwhaxarkamevgle2jaqs8vlnr";
/// The seller's sale pre-signature on the escrow input.
pub const SIGHASH_SINGLE_ANYONECANPAY: u8 = 0x83;
/// Creator royalties, at most this share of the seller's `escrow_price` (basis points), rounded down, and only to a creator pinned
/// in the install's config (review P-01).
pub const MAX_ROYALTY_BPS: u64 = 1_000;
/// Review P-01: the marketplace fee is at most `max(MARKET_FEE_MIN_SATS, ⌈MARKET_FEE_BPS × escrow_price⌉)` plus a tolerance of
/// `⌈MARKET_FEE_TOLERANCE_BPS × escrow_price⌉`, all of the seller's own price. Ordinals Wallet's own vectors charge 1,350 on a
/// 50,000-sat escrow (2.7 %): the bound there is 1,500.
pub const MARKET_FEE_MIN_SATS: u64 = 1_000;
pub const MARKET_FEE_BPS: u64 = 200;
pub const MARKET_FEE_TOLERANCE_BPS: u64 = 100;
/// Review P-02: the miner fees may be at most this multiple of the canister's own fee rate on the estimated size.
pub const FEE_RATE_SLACK: u64 = 2;

/// Review P-01: the most the marketplace may take on a seller price of `escrow` sats.
pub fn market_fee_max(escrow: u64) -> u64 {
    let bps = |b: u64| (escrow as u128 * b as u128).div_ceil(10_000) as u64;
    MARKET_FEE_MIN_SATS.max(bps(MARKET_FEE_BPS)).saturating_add(bps(MARKET_FEE_TOLERANCE_BPS))
}

/// Review P-02: a transaction's virtual size once signed, estimated from its unsigned bytes: key-path float inputs (a 64-byte
/// signature each) and, for `escrow`, the co-signed script path (two signatures, the 68-byte leaf, a 33-byte control block).
pub fn est_vsize(tx: &psbt::Tx, escrow: Option<usize>) -> u64 {
    let compact = |n: usize| if n < 0xfd { 1 } else { 3 };
    let base = 4 + compact(tx.inputs.len()) + 41 * tx.inputs.len() + compact(tx.outputs.len())
        + tx.outputs.iter().map(|o| 8 + compact(o.script.len()) + o.script.len()).sum::<usize>() + 4;
    let wit: usize = 2 + (0..tx.inputs.len()).map(|i| if Some(i) == escrow { 1 + 65 + 66 + 69 + 34 } else { 1 + 65 }).sum::<usize>();
    (base as u64) + (wit as u64).div_ceil(4)
}
/// What the passthrough may take from the postage for its own fee: nothing, or 12 sats [OW].
pub const PASSTHROUGH_FEES: [u64; 2] = [0, 12];
/// The least an escrow (P2TR) output may hold.
pub const DUST_P2TR: u64 = crate::custody::DUST_P2TR;

const SIGHASH_DEFAULT: u32 = 0x00;
const MAINNET: &str = "bc";

/// The listing as the runner read it (`GET /market/escrow/:id`). Every address is checked against the PSBT, never trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// The output the listing sells (the passthrough's one input).
    pub outpoint: Outpoint,
    /// Where the seller is paid (mainnet bech32/bech32m).
    pub seller_address: String,
    /// The buyer-facing price, as the API says it. Review P-01: it bounds nothing (only a listing without one is refused).
    pub satoshi_price: u64,
    /// What the seller listed for: the payout must equal it. Unknown is refused.
    pub escrow_price: Option<u64>,
    /// The collection creator's address as the API says it. Review P-01: never believed; royalties go only to the creators pinned in
    /// the install's config ([`ProtectedFacts::creators`]).
    pub creator_address: Option<String>,
}

/// What a sale is judged against. Every field is the canister's own reading.
#[derive(Debug, Clone)]
pub struct ProtectedFacts<'a> {
    /// The float's script: our inputs carry it, our change returns to it.
    pub float: &'a [u8],
    /// Where the inscription must land.
    pub custody: &'a [u8],
    /// Where ord says the inscription is (the seller's output, before the passthrough), its sat's offset and the output's value.
    pub inscription: Seller,
    /// The most that may leave the float, the setup's fee included.
    pub cap_sats: u64,
    /// Review P-01: the creators' scripts pinned for the collection in the install's config: the only royalty recipients. Empty:
    /// no royalty is paid.
    pub creators: &'a [Vec<u8>],
    /// Review P-02: the canister's own fee rate (sats/vB), when read: the miner fees may be at most `FEE_RATE_SLACK` times it on the
    /// estimated size.
    pub fee_rate: Option<u64>,
}

/// A judged setup: its txid and the float inputs to sign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupJudged {
    /// Display order.
    pub txid: [u8; 32],
    pub float_inputs: Vec<usize>,
    pub fee: u64,
    /// Its two outputs, both the float's: what the sale must spend.
    pub outputs: Vec<psbt::TxOut>,
}

/// What a passed judgement says: what to sign, what to watch for, and what it cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judged {
    /// The sale's float inputs: the only ones the canister signs.
    pub float_inputs: Vec<usize>,
    /// The escrow (passthrough) input: never signed by the canister.
    pub escrow_input: usize,
    pub custody_vout: u32,
    /// What leaves the float: the sale's float inputs less its float outputs, plus the setup's fee.
    pub spend: u64,
    /// The miner fees: the sale's plus the setup's.
    pub fee: u64,
    pub sale_fee: u64,
    pub seller_payout: u64,
    pub market_fee: u64,
    pub royalty: u64,
    /// The sale's txid (non-witness serialisation), display order: what the canister watches for.
    pub sale_txid: [u8; 32],
    /// The parent's txid, display order.
    pub parent_txid: [u8; 32],
    pub setup: Option<SetupJudged>,
}

/// Why a protected sale is refused. Each is a reason not to sign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// The sale PSBT, the parent or the setup PSBT does not parse.
    Unparsable { what: &'static str, why: String },
    /// An address the listing gave is not a mainnet segwit address.
    BadAddress { what: &'static str, address: String },
    /// The listing has no price.
    NoPrice,
    /// The listing's `escrow_price` is unknown, so the payout cannot be checked exactly (canister rule).
    EscrowPriceUnknown,
    /// The listing names an output other than the one ord located the inscription in (P1, canister rule).
    ListingNotLocated { listing: Outpoint, located: Outpoint },
    /// The sale does not have the escrow input between float inputs (P3; SDK `sale_input_count`).
    InputLayout { escrow: Option<usize>, inputs: usize },
    /// A sale input does not say what it spends (SDK `missing_witness_utxo`).
    UnknownInputValue(usize),
    /// Not exactly one sale input spends the parent, or it spends an output other than 0 (P3).
    EscrowInputs(usize),
    /// The parent has not exactly one input and one output (P1; SDK `parent_source_mismatch`).
    ParentShape { inputs: usize, outputs: usize },
    /// The parent does not spend the listed outpoint (P1; SDK `parent_source_mismatch`).
    ParentSource,
    /// The escrow input's `witness_utxo` is not the parent's output (P3; SDK `sale_prevout_mismatch`).
    EscrowPrevout(usize),
    /// The escrow input carries neither the sale leaf nor a co-signed witness (P4; SDK `sale_not_cosigned`).
    EscrowNotCosignable(usize),
    /// The escrow input's template internal key is not `NUMS` (P4; SDK `sale_template_internal_key`).
    EscrowInternalKey(usize),
    /// The escrow input's witness is not a 2-of-2 script path whose seller signature is `0x83` (P4; SDK `sale_witness_*`).
    EscrowWitness(usize),
    /// The sale leaf is not co-signed by the pinned key (P4; SDK `sale_leaf_unpinned`).
    LeafUnpinned(usize),
    /// The parent's output is not the escrow rebuilt from the leaf's seller key and the pinned co-signer (P2; SDK
    /// `sale_escrow_mismatch`).
    EscrowScript,
    /// The escrow is under dust, or the passthrough took other than 0 or 12 sats from the postage (P2, canister rule).
    EscrowValue { postage: u64, escrow: u64 },
    /// The inscription's offset is not within the escrow output (P2, canister rule).
    OffsetBeyondEscrow { offset: u64, escrow: u64 },
    /// A non-escrow input is not the float's (P5; SDK `funding_not_yours`).
    ForeignInput(usize),
    /// A float input already carries a signature or final witness (P5; SDK `funding_prefilled`).
    InputSigned(usize),
    /// A float input asks for a sighash other than `DEFAULT` (P5; SDK `buyer_sighash`, which also admits `ALL`: the canister signs
    /// only `SIGHASH_DEFAULT`, 64-byte signatures, so a PSBT asking for anything else is refused rather than signed differently).
    Sighash { input: usize, sighash: u32 },
    /// The output at the escrow's index does not pay the seller (P6; SDK `sale_payout_mismatch`).
    NoPayout(usize),
    /// The seller is paid other than the listed `escrow_price` (P6; SDK `sale_payout_mismatch`).
    PayoutPrice { paid: u64, listed: u64 },
    /// An `OP_RETURN` output: a runestone could move runes onto custody's output (P8, canister rule).
    OpReturn(usize),
    /// Custody and the float are the same script: custody's output could not be told from change (P7, canister rule).
    CustodyIsFloat,
    /// Not exactly one output pays custody (P7, canister rule).
    CustodyOutputs(usize),
    /// The inscribed sat lands on an output that is not custody's (`None`: to the miner) (P7; SDK `sale_asset_mismatch`).
    NotToCustody(Option<usize>),
    /// Custody's output does not carry exactly the escrow's sats (P7; SDK `sale_asset_mismatch`).
    CustodyValue { value: u64, escrow: u64 },
    /// Output 0 is not the float's change (P8; SDK `sale_change_mismatch`).
    ChangeFirst,
    /// An output pays someone who is neither the float, the marketplace nor the listed creator (P8; SDK `sale_unknown_output`).
    UnknownOutput(usize),
    /// The marketplace fee is over its bound on the seller's price (P8, review P-01; covers SDK `sale_overcharge`).
    MarketFee { paid: u64, max: u64 },
    /// Royalties over their ceiling, a share of the seller's price (P8, review P-01; SDK `sale_royalty`).
    Royalty { paid: u64, max: u64 },
    /// Outputs are worth at least the inputs: no fee to relay with (SDK `sale_fee`, `setup_fee`).
    NoNetworkFee { inputs: u64, outputs: u64 },
    /// More would leave the float than the cap (P9, canister rule).
    OverCap { spend: u64, cap: u64 },
    /// The miner fees are over `frogbuy::fee_bound(spend)` (P9, canister rule; covers SDK `sale_fee`, `setup_fee`).
    FeeTooHigh { fee: u64, bound: u64 },
    /// The miner fees are over `FEE_RATE_SLACK` times the canister's own fee rate on the estimated size (review P-02; the SDK's
    /// `max_network_fee`).
    FeeRateTooHigh { fee: u64, max: u64 },
    /// A transaction that is not version 2 (review P-06).
    Version { what: &'static str, version: i32 },
    /// A transaction with a locktime (review P-06): it could not be mined now.
    LockTime { what: &'static str, locktime: u32 },
    /// A float input with a relative lock (BIP 68: version 2 and the disable bit clear) (review P-06).
    RelativeLock { what: &'static str, input: usize },
    /// The setup does not have exactly two outputs (P10; SDK `setup_output_shape`).
    SetupOutputs(usize),
    /// A setup output does not pay the float (P10; SDK `setup_output_not_yours`).
    SetupOutputNotFloat(usize),
    /// A sale float input does not spend one of the setup's outputs, at that output's value and script (P10; SDK
    /// `chain_funding_mismatch`, which checks the outpoint only).
    NotFundedBySetup(usize),
    /// A rule broken by the setup rather than the sale (its inputs, sighash, fee).
    InSetup(Box<Refused>),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Refused::*;
        match self {
            Unparsable { what, why } => write!(f, "the {what} does not parse: {why}"),
            BadAddress { what, address } => write!(f, "the {what} address {address:?} is not a mainnet segwit address"),
            NoPrice => write!(f, "the listing has no price"),
            EscrowPriceUnknown => write!(f, "the listing's escrow price is unknown, so the seller's payout cannot be checked"),
            ListingNotLocated { listing, located } => {
                write!(f, "the listing sells {}, but ord locates the inscription in {}", listing.text(), located.text())
            }
            InputLayout { escrow: Some(k), inputs } => {
                write!(f, "the escrow is input {k} of {inputs}: it needs a float input before it and one after it")
            }
            InputLayout { escrow: None, inputs } => write!(f, "none of the sale's {inputs} inputs spends the passthrough"),
            UnknownInputValue(i) => write!(f, "sale input {i} does not say what it spends"),
            EscrowInputs(n) => write!(f, "{n} sale inputs spend the passthrough; exactly one, spending its output 0, is required"),
            ParentShape { inputs, outputs } => {
                write!(f, "the passthrough has {inputs} inputs and {outputs} outputs; it must have one of each")
            }
            ParentSource => write!(f, "the passthrough does not spend the listed output"),
            EscrowPrevout(i) => write!(f, "sale input {i} misstates the passthrough output it spends"),
            EscrowNotCosignable(i) => write!(f, "escrow input {i} carries neither its sale leaf nor a co-signed witness"),
            EscrowInternalKey(i) => write!(f, "escrow input {i} does not use the unspendable internal key"),
            EscrowWitness(i) => write!(f, "escrow input {i}'s witness is not a 2-of-2 whose seller signature is SINGLE|ANYONECANPAY"),
            LeafUnpinned(i) => write!(f, "escrow input {i} is not co-signed by Ordinals Wallet's pinned key"),
            EscrowScript => write!(f, "the passthrough's output is not the escrow its leaf names"),
            EscrowValue { postage, escrow } => {
                write!(f, "the passthrough turns {postage} sats of postage into {escrow}: under dust, or a fee other than 0 or 12")
            }
            OffsetBeyondEscrow { offset, escrow } => {
                write!(f, "the inscription is at offset {offset}, beyond the escrow's {escrow} sats")
            }
            ForeignInput(i) => write!(f, "input {i} is not the float's"),
            InputSigned(i) => write!(f, "input {i} already carries a signature or witness"),
            Sighash { input, sighash } => write!(f, "input {input} asks to be signed with sighash 0x{sighash:02x}"),
            NoPayout(i) => write!(f, "output {i} does not pay the seller"),
            PayoutPrice { paid, listed } => write!(f, "the seller is paid {paid} sats, not the {listed} listed"),
            OpReturn(i) => write!(f, "output {i} is an OP_RETURN"),
            CustodyIsFloat => write!(f, "custody's script is the float's"),
            CustodyOutputs(n) => write!(f, "{n} outputs pay custody; exactly one is required"),
            NotToCustody(Some(j)) => write!(f, "the inscribed sat lands on output {j}, not custody's"),
            NotToCustody(None) => write!(f, "the inscribed sat goes to the miner"),
            CustodyValue { value, escrow } => {
                write!(f, "custody's output holds {value} sats that are not exactly the escrow's {escrow}")
            }
            ChangeFirst => write!(f, "output 0 is not the float's change"),
            UnknownOutput(i) => write!(f, "output {i} pays an unexpected party"),
            MarketFee { paid, max } => write!(f, "a marketplace fee of {paid} sats is over the {max} sat bound on the seller's price"),
            Royalty { paid, max } => write!(f, "royalties of {paid} sats are over the {max} sat ceiling"),
            NoNetworkFee { inputs, outputs } => write!(f, "outputs of {outputs} sats leave no fee from inputs of {inputs}"),
            OverCap { spend, cap } => write!(f, "{spend} sats would leave the float, over the {cap} sat cap"),
            FeeTooHigh { fee, bound } => write!(f, "a miner fee of {fee} sats is over the {bound} sat bound"),
            FeeRateTooHigh { fee, max } => write!(f, "a miner fee of {fee} sats is over the {max} sats our fee rate allows"),
            Version { what, version } => write!(f, "the {what} is version {version}, not 2"),
            LockTime { what, locktime } => write!(f, "the {what} has locktime {locktime}, not 0"),
            RelativeLock { what, input } => write!(f, "{what} input {input} carries a relative lock"),
            SetupOutputs(n) => write!(f, "the setup has {n} outputs; exactly two are required"),
            SetupOutputNotFloat(i) => write!(f, "setup output {i} does not pay the float"),
            NotFundedBySetup(i) => write!(f, "sale input {i} does not spend one of the setup's outputs as the setup states it"),
            InSetup(r) => write!(f, "in the setup: {r}"),
        }
    }
}

fn script_for(what: &'static str, address: &str) -> Result<Vec<u8>, Refused> {
    crate::policy::script_of(address, MAINNET).map_err(|_| Refused::BadAddress { what, address: address.to_string() })
}

fn sum(values: impl Iterator<Item = u64>) -> u64 {
    values.fold(0u64, |a, v| a.saturating_add(v))
}

/// Float inputs: the float's script, nothing signed, no sighash but `DEFAULT` if one is stated.
fn check_float_input(i: usize, utxo: Option<&psbt::TxOut>, f: &InputFields, float: &[u8]) -> Result<(), Refused> {
    if utxo.is_none_or(|u| u.script != float) {
        return Err(Refused::ForeignInput(i));
    }
    if f.signed() {
        return Err(Refused::InputSigned(i));
    }
    if let Some(t) = f.sighash_type {
        if t != SIGHASH_DEFAULT {
            return Err(Refused::Sighash { input: i, sighash: t });
        }
    }
    Ok(())
}

/// Review P-06: version 2 and locktime 0.
fn check_shape(what: &'static str, tx: &psbt::Tx) -> Result<(), Refused> {
    if tx.version != 2 {
        return Err(Refused::Version { what, version: tx.version });
    }
    if tx.locktime != 0 {
        return Err(Refused::LockTime { what, locktime: tx.locktime });
    }
    Ok(())
}

/// Review P-06: a float input's sequence must disable BIP 68 (bit 31 set): no relative lock on our coins.
fn check_sequence(what: &'static str, i: usize, sequence: u32) -> Result<(), Refused> {
    if sequence & (1 << 31) == 0 {
        return Err(Refused::RelativeLock { what, input: i });
    }
    Ok(())
}

/// Judge a setup on its own (P10's shape): only the float's unsigned inputs, exactly two outputs to the float, some fee. Its fee
/// is bounded with the sale's in [`judge_protected`], where the spend it is a share of is known.
pub fn judge_setup(setup_psbt: &[u8], float: &[u8]) -> Result<SetupJudged, Refused> {
    let unparsable = |e: psbt::ParseError| Refused::Unparsable { what: "setup PSBT", why: format!("{e:?}") };
    let p = Psbt::parse(setup_psbt).map_err(unparsable)?;
    let fields = psbt::input_fields(setup_psbt).map_err(unparsable)?;
    check_shape("setup", &p.tx)?;
    let mut float_inputs = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        check_float_input(i, p.witness_utxos[i].as_ref(), f, float).map_err(|r| Refused::InSetup(Box::new(r)))?;
        check_sequence("setup", i, p.tx.inputs[i].sequence)?;
        float_inputs.push(i);
    }
    let outs = &p.tx.outputs;
    if outs.len() != 2 {
        return Err(Refused::SetupOutputs(outs.len()));
    }
    if let Some(j) = outs.iter().position(|o| o.script != float) {
        return Err(Refused::SetupOutputNotFloat(j));
    }
    let inputs = sum(p.witness_utxos.iter().flatten().map(|u| u.value));
    let outputs = sum(outs.iter().map(|o| o.value));
    if outputs >= inputs {
        return Err(Refused::InSetup(Box::new(Refused::NoNetworkFee { inputs, outputs })));
    }
    Ok(SetupJudged { txid: p.txid(), float_inputs, fee: inputs - outputs, outputs: outs.clone() })
}

/// Judge a protected sale (P1–P10). `parent_raw` is the passthrough, seller-signed or witness-stripped; `setup_psbt` the
/// build's setup when the float had fewer than two coins.
pub fn judge_protected(
    sale_psbt: &[u8],
    parent_raw: &[u8],
    listing: &Listing,
    facts: &ProtectedFacts,
    setup_psbt: Option<&[u8]>,
) -> Result<Judged, Refused> {
    let unparsable = |what: &'static str| move |e: psbt::ParseError| Refused::Unparsable { what, why: format!("{e:?}") };
    let sale = Psbt::parse(sale_psbt).map_err(unparsable("sale PSBT"))?;
    let fields = psbt::input_fields(sale_psbt).map_err(unparsable("sale PSBT"))?;
    let seller_script = script_for("seller", &listing.seller_address)?;
    let market_script = script_for("marketplace fee", MARKET_FEE_ADDRESS)?;
    if listing.satoshi_price == 0 {
        return Err(Refused::NoPrice);
    }
    if listing.outpoint != facts.inscription.outpoint {
        return Err(Refused::ListingNotLocated { listing: listing.outpoint, located: facts.inscription.outpoint });
    }
    let tx = &sale.tx;
    let n = tx.inputs.len();
    if n < 3 {
        return Err(Refused::InputLayout { escrow: None, inputs: n });
    }
    let parent = psbt::parse_raw_tx(parent_raw).map_err(unparsable("parent"))?;
    check_shape("sale", tx)?;
    check_shape("parent", &parent.tx)?;
    let mut values = Vec::with_capacity(n);
    for (i, u) in sale.witness_utxos.iter().enumerate() {
        match u {
            Some(u) => values.push(u.value),
            None => return Err(Refused::UnknownInputValue(i)),
        }
    }

    // P3: the one escrow input, between float inputs
    let mut parent_internal = parent.txid;
    parent_internal.reverse();
    let spending: Vec<usize> = (0..n).filter(|&i| tx.inputs[i].txid == parent_internal).collect();
    let k = match spending.first() {
        Some(&k) if k >= 1 && k + 1 < n => k,
        first => return Err(Refused::InputLayout { escrow: first.copied(), inputs: n }),
    };
    // P5: every other input is the float's, unsigned, DEFAULT or ALL
    let float_inputs: Vec<usize> = (0..n).filter(|&i| i != k).collect();
    for &i in &float_inputs {
        check_float_input(i, sale.witness_utxos[i].as_ref(), &fields[i], facts.float)?;
        check_sequence("sale", i, tx.inputs[i].sequence)?;
    }
    if spending.len() != 1 || tx.inputs[k].vout != 0 {
        return Err(Refused::EscrowInputs(spending.len()));
    }
    // P1: the parent moves exactly the listed (and located) output, into one output
    if parent.tx.inputs.len() != 1 || parent.tx.outputs.len() != 1 {
        return Err(Refused::ParentShape { inputs: parent.tx.inputs.len(), outputs: parent.tx.outputs.len() });
    }
    if crate::custody::outpoint_of(&parent.tx.inputs[0]) != listing.outpoint {
        return Err(Refused::ParentSource);
    }
    let escrow_out = &parent.tx.outputs[0];
    if sale.witness_utxos[k].as_ref() != Some(escrow_out) {
        return Err(Refused::EscrowPrevout(k));
    }
    // P4: the sale leaf, from a co-signed witness or from the template
    let f = &fields[k];
    let leaf = if let Some(w) = &f.final_script_witness {
        if w.len() != 4 || w[0].len() != 64 || w[1].len() != 65 || w[1][64] != SIGHASH_SINGLE_ANYONECANPAY {
            return Err(Refused::EscrowWitness(k));
        }
        w[2].clone()
    } else {
        let Some(t) = f.tap_leaf_scripts.iter().find(|t| t.leaf_version == TAPSCRIPT && taptree::parse_sale_leaf(&t.script).is_some())
        else {
            return Err(Refused::EscrowNotCosignable(k));
        };
        if f.tap_internal_key != Some(NUMS) {
            return Err(Refused::EscrowInternalKey(k));
        }
        t.script.clone()
    };
    let Some((seller_key, _)) = taptree::parse_sale_leaf(&leaf).filter(|(_, c)| *c == COSIGNER) else {
        return Err(Refused::LeafUnpinned(k));
    };
    // P2: the parent's output is the escrow rebuilt here, worth the postage less 0 or 12, holding the inscription
    match taptree::passthrough_escrow(&seller_key, &COSIGNER) {
        Ok(e) if e.script == escrow_out.script => {}
        _ => return Err(Refused::EscrowScript),
    }
    let (postage, escrow) = (facts.inscription.value, escrow_out.value);
    if escrow < DUST_P2TR || !PASSTHROUGH_FEES.iter().any(|d| postage.checked_sub(*d) == Some(escrow)) {
        return Err(Refused::EscrowValue { postage, escrow });
    }
    if facts.inscription.offset >= escrow {
        return Err(Refused::OffsetBeyondEscrow { offset: facts.inscription.offset, escrow });
    }

    // P6: the seller, paid exactly what they listed for, at the escrow's index
    let payout = match tx.outputs.get(k) {
        Some(o) if o.script == seller_script => o.value,
        _ => return Err(Refused::NoPayout(k)),
    };
    let listed = listing.escrow_price.ok_or(Refused::EscrowPriceUnknown)?;
    if payout != listed {
        return Err(Refused::PayoutPrice { paid: payout, listed });
    }

    // P8 (first part): no OP_RETURN anywhere
    if let Some(j) = tx.outputs.iter().position(|o| o.script.first() == Some(&0x6a)) {
        return Err(Refused::OpReturn(j));
    }
    // P7: the inscribed sat lands in the one custody output, which holds exactly the escrow's sats
    if facts.custody == facts.float {
        return Err(Refused::CustodyIsFloat);
    }
    let to_custody: Vec<usize> = (0..tx.outputs.len()).filter(|&j| tx.outputs[j].script == facts.custody).collect();
    if to_custody.len() != 1 {
        return Err(Refused::CustodyOutputs(to_custody.len()));
    }
    let c = to_custody[0];
    let escrow_start = sum(values[..k].iter().copied());
    let pos = escrow_start + facts.inscription.offset;
    let mut start = 0u64;
    let mut landed = None;
    for (j, o) in tx.outputs.iter().enumerate() {
        if pos >= start && pos - start < o.value {
            landed = Some((j, start));
            break;
        }
        start = start.saturating_add(o.value);
    }
    match landed {
        Some((j, _)) if j != c => return Err(Refused::NotToCustody(Some(j))),
        None => return Err(Refused::NotToCustody(None)),
        Some((_, s)) if s != escrow_start || tx.outputs[c].value != escrow => {
            return Err(Refused::CustodyValue { value: tx.outputs[c].value, escrow })
        }
        Some(_) => {}
    }

    // P8: every other output is change, the marketplace fee or the creator's royalty
    let (mut market_fee, mut royalty) = (0u64, 0u64);
    for (j, o) in tx.outputs.iter().enumerate() {
        if j == k || j == c {
            continue;
        }
        if j == 0 {
            if o.script != facts.float {
                return Err(Refused::ChangeFirst);
            }
            continue;
        }
        if o.script == facts.float {
            continue;
        }
        if o.script == market_script {
            market_fee = market_fee.saturating_add(o.value);
        } else if facts.creators.contains(&o.script) {
            royalty = royalty.saturating_add(o.value);
        } else {
            return Err(Refused::UnknownOutput(j));
        }
    }
    let in_total = sum(values.iter().copied());
    let out_total = sum(tx.outputs.iter().map(|o| o.value));
    if out_total >= in_total {
        return Err(Refused::NoNetworkFee { inputs: in_total, outputs: out_total });
    }
    let sale_fee = in_total - out_total;
    // review P-01: both bounded by what the seller signed for (payout == escrow_price, checked above), never satoshi_price
    let max = market_fee_max(listed);
    if market_fee > max {
        return Err(Refused::MarketFee { paid: market_fee, max });
    }
    let max = listed.saturating_mul(MAX_ROYALTY_BPS) / 10_000;
    if royalty > max {
        return Err(Refused::Royalty { paid: royalty, max });
    }

    // P10: the setup, and the sale funded by exactly its outputs
    let setup = match setup_psbt {
        Some(b) => {
            let s = judge_setup(b, facts.float)?;
            let mut internal = s.txid;
            internal.reverse();
            let funded = |i: usize| {
                let vout = tx.inputs[i].vout as usize;
                tx.inputs[i].txid == internal && s.outputs.get(vout).is_some_and(|o| sale.witness_utxos[i].as_ref() == Some(o))
            };
            if let Some(&i) = float_inputs.iter().find(|&&i| !funded(i)) {
                return Err(Refused::NotFundedBySetup(i));
            }
            Some(s)
        }
        None => None,
    };

    // P9: the cap and the fee bound, over everything that leaves the float
    let float_in = sum(float_inputs.iter().map(|&i| values[i]));
    let float_out = sum(tx.outputs.iter().filter(|o| o.script == facts.float).map(|o| o.value));
    let setup_fee = setup.as_ref().map_or(0, |s| s.fee);
    let spend = float_in.saturating_sub(float_out).saturating_add(setup_fee);
    if spend > facts.cap_sats {
        return Err(Refused::OverCap { spend, cap: facts.cap_sats });
    }
    let fee = sale_fee.saturating_add(setup_fee);
    let bound = fee_bound(spend);
    if fee > bound {
        return Err(Refused::FeeTooHigh { fee, bound });
    }
    // review P-02: and against our own fee rate, so a fee chosen by the API for a sale it never relays costs us at most a normal one
    if let Some(rate) = facts.fee_rate {
        let vsize = est_vsize(tx, Some(k)) + setup_psbt.and_then(|b| Psbt::parse(b).ok()).map_or(0, |p| est_vsize(&p.tx, None));
        let max = vsize.saturating_mul(rate.max(1)).saturating_mul(FEE_RATE_SLACK);
        if fee > max {
            return Err(Refused::FeeRateTooHigh { fee, max });
        }
    }
    Ok(Judged {
        float_inputs,
        escrow_input: k,
        custody_vout: c as u32,
        spend,
        fee,
        sale_fee,
        seller_payout: payout,
        market_fee,
        royalty,
        sale_txid: sale.txid(),
        parent_txid: parent.txid,
        setup,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::psbt::{Tx, TxIn, TxOut};

    fn p2tr(tag: u8) -> Vec<u8> {
        [vec![0x51, 0x20], vec![tag; 32]].concat()
    }
    const FLOAT: u8 = 1;
    const CUSTODY: u8 = 2;
    const SELLER_KEY: [u8; 32] = [0x11; 32];

    fn compact(n: usize) -> Vec<u8> {
        if n < 0xfd { vec![n as u8] } else { [vec![0xfd], (n as u16).to_le_bytes().to_vec()].concat() }
    }
    fn ser_out(o: &TxOut) -> Vec<u8> {
        [o.value.to_le_bytes().to_vec(), compact(o.script.len()), o.script.clone()].concat()
    }
    fn ser_tx(tx: &Tx) -> Vec<u8> {
        let mut b = tx.version.to_le_bytes().to_vec();
        b.extend(compact(tx.inputs.len()));
        for i in &tx.inputs {
            b.extend_from_slice(&i.txid);
            b.extend_from_slice(&i.vout.to_le_bytes());
            b.push(0);
            b.extend_from_slice(&i.sequence.to_le_bytes());
        }
        b.extend(compact(tx.outputs.len()));
        for o in &tx.outputs {
            b.extend(ser_out(o));
        }
        b.extend_from_slice(&tx.locktime.to_le_bytes());
        b
    }
    fn pair(b: &mut Vec<u8>, key: &[u8], value: &[u8]) {
        b.extend(compact(key.len()));
        b.extend_from_slice(key);
        b.extend(compact(value.len()));
        b.extend_from_slice(value);
    }
    /// A PSBT: the transaction, each input's prevout and extra pairs.
    fn psbt(tx: &Tx, utxos: &[TxOut], extra: &[Vec<(Vec<u8>, Vec<u8>)>]) -> Vec<u8> {
        let mut b = b"psbt\xff".to_vec();
        pair(&mut b, &[0], &ser_tx(tx));
        b.push(0);
        for (i, u) in utxos.iter().enumerate() {
            pair(&mut b, &[1], &ser_out(u));
            for (k, v) in &extra[i] {
                pair(&mut b, k, v);
            }
            b.push(0);
        }
        b.extend(std::iter::repeat_n(0u8, tx.outputs.len()));
        b
    }
    fn inp(tag: u8, vout: u32) -> TxIn {
        TxIn { txid: [tag; 32], vout, sequence: 0xffff_fffd }
    }

    /// One honest protected sale, built the way Ordinals Wallet lays it out.
    struct World {
        listing: Listing,
        parent: Tx,
        sale: Tx,
        utxos: Vec<TxOut>,
        extra: Vec<Vec<(Vec<u8>, Vec<u8>)>>,
        postage: u64,
        offset: u64,
        cap: u64,
        setup: Option<Vec<u8>>,
        custody: Vec<u8>,
        /// Where ord locates the inscription.
        located: Outpoint,
        /// The creators pinned in the config (review P-01) and the canister's fee rate (P-02).
        creators: Vec<Vec<u8>>,
        fee_rate: Option<u64>,
    }

    fn world() -> World {
        let escrow = taptree::passthrough_escrow(&SELLER_KEY, &COSIGNER).unwrap();
        let listed = Outpoint { txid: [0xaa; 32], vout: 0 };
        let parent = Tx { version: 2, inputs: vec![TxIn { txid: listed.internal_txid(), vout: 0, sequence: 0xffff_ffff }],
            outputs: vec![TxOut { value: 546, script: escrow.script.clone() }], locktime: 0 };
        let seller_pay = [0x33; 32];
        let listing = Listing {
            outpoint: listed,
            seller_address: crate::policy::taproot_address(&seller_pay, "bc"),
            satoshi_price: 51_000,
            escrow_price: Some(50_000),
            creator_address: Some(crate::policy::taproot_address(&[0x44; 32], "bc")),
        };
        let market = crate::policy::script_of(MARKET_FEE_ADDRESS, "bc").unwrap();
        let mut w = World {
            listing, parent, sale: Tx { version: 2, inputs: vec![], outputs: vec![], locktime: 0 }, utxos: vec![], extra: vec![],
            postage: 546, offset: 0, cap: 1_000_000, setup: None, custody: p2tr(CUSTODY), located: listed,
            creators: vec![[vec![0x51, 0x20], vec![0x44; 32]].concat()], fee_rate: None,
        };
        let parent_txid = w.parent_txid();
        w.sale.inputs = vec![inp(0xa1, 0), TxIn { txid: parent_txid, vout: 0, sequence: 0xffff_ffff }, inp(0xa2, 1)];
        w.utxos = vec![
            TxOut { value: 60_000, script: p2tr(FLOAT) },
            w.parent.outputs[0].clone(),
            TxOut { value: 100_000, script: p2tr(FLOAT) },
        ];
        w.sale.outputs = vec![
            TxOut { value: 10_000, script: p2tr(FLOAT) },
            TxOut { value: 50_000, script: [vec![0x51, 0x20], seller_pay.to_vec()].concat() },
            TxOut { value: 546, script: p2tr(CUSTODY) },
            TxOut { value: 1_000, script: market },
            TxOut { value: 500, script: [vec![0x51, 0x20], vec![0x44; 32]].concat() },
            TxOut { value: 97_500, script: p2tr(FLOAT) },
        ];
        let mut leaf = vec![0x15];
        leaf.extend_from_slice(&escrow.sale_control_block);
        let mut leaf_value = escrow.sale_leaf.clone();
        leaf_value.push(TAPSCRIPT);
        w.extra = vec![vec![], vec![(leaf, leaf_value), (vec![0x17], NUMS.to_vec())], vec![]];
        w
    }

    impl World {
        fn parent_txid(&self) -> [u8; 32] {
            let mut t = psbt::parse_raw_tx(&ser_tx(&self.parent)).unwrap().txid;
            t.reverse();
            t
        }
        fn run(&self) -> Result<Judged, Refused> {
            let float = p2tr(FLOAT);
            let facts = ProtectedFacts {
                float: &float,
                custody: &self.custody,
                inscription: Seller { outpoint: self.located, offset: self.offset, value: self.postage },
                cap_sats: self.cap,
                creators: &self.creators,
                fee_rate: self.fee_rate,
            };
            let sale = psbt(&self.sale, &self.utxos, &self.extra);
            judge_protected(&sale, &ser_tx(&self.parent), &self.listing, &facts, self.setup.as_deref())
        }
        /// Re-point the escrow input at the (changed) parent.
        fn relink(&mut self) {
            self.sale.inputs[1].txid = self.parent_txid();
            self.utxos[1] = self.parent.outputs[0].clone();
        }
    }

    #[test]
    fn an_honest_protected_sale_is_judged_and_says_what_to_sign() {
        let w = world();
        let j = w.run().unwrap();
        assert_eq!((j.float_inputs.clone(), j.escrow_input, j.custody_vout), (vec![0, 2], 1, 2));
        // float in 160,000, float out 107,500: 50,000 + 1,000 + 500 + a 1,000 fee
        assert_eq!((j.spend, j.fee, j.sale_fee), (52_500, 1_000, 1_000));
        assert_eq!((j.seller_payout, j.market_fee, j.royalty), (50_000, 1_000, 500));
        assert_eq!(j.sale_txid, Psbt::parse(&psbt(&w.sale, &w.utxos, &w.extra)).unwrap().txid());
        assert_eq!(j.setup, None);
    }

    #[test]
    fn the_passthrough_may_take_twelve_sats_and_the_inscription_may_sit_anywhere_in_the_escrow() {
        let mut w = world();
        w.postage = 558; // 558 − 12 = 546
        assert!(w.run().is_ok());
        w.postage = 550;
        assert_eq!(w.run().unwrap_err(), Refused::EscrowValue { postage: 550, escrow: 546 });
        let mut w = world();
        w.offset = 545;
        assert!(w.run().is_ok(), "the last sat of the escrow still lands on custody");
        w.offset = 546;
        assert_eq!(w.run().unwrap_err(), Refused::OffsetBeyondEscrow { offset: 546, escrow: 546 });
        // an escrow under dust
        let mut w = world();
        w.parent.outputs[0].value = 329;
        w.postage = 329;
        w.relink();
        assert_eq!(w.run().unwrap_err(), Refused::EscrowValue { postage: 329, escrow: 329 });
    }

    #[test]
    fn the_listing_must_be_where_ord_says_the_inscription_is() {
        let mut w = world();
        w.located = Outpoint { txid: [0xab; 32], vout: 0 };
        assert!(matches!(w.run().unwrap_err(), Refused::ListingNotLocated { .. }));
        // the listing and ord agree, but the passthrough spends another output
        let mut w = world();
        w.parent.inputs[0].vout = 1;
        w.relink();
        assert_eq!(w.run().unwrap_err(), Refused::ParentSource);
        // a parent with a second output
        let mut w = world();
        w.parent.outputs.push(TxOut { value: 1, script: p2tr(9) });
        w.relink();
        assert_eq!(w.run().unwrap_err(), Refused::ParentShape { inputs: 1, outputs: 2 });
    }

    #[test]
    fn custody_gets_exactly_one_output_with_exactly_the_escrows_sats_and_no_op_return() {
        let mut w = world();
        w.sale.outputs[5].value -= 1_000;
        w.sale.outputs.push(TxOut { value: 1_000, script: p2tr(CUSTODY) });
        assert_eq!(w.run().unwrap_err(), Refused::CustodyOutputs(2));
        let mut w = world();
        w.sale.outputs.push(TxOut { value: 0, script: vec![0x6a, 0x5d, 0x01, 0x00] });
        assert_eq!(w.run().unwrap_err(), Refused::OpReturn(6));
        let mut w = world();
        w.custody = p2tr(FLOAT);
        assert_eq!(w.run().unwrap_err(), Refused::CustodyIsFloat);
        // custody's output a sat larger than the escrow: one of the float's sats beside the inscription
        let mut w = world();
        w.sale.outputs[2].value += 1;
        w.sale.outputs[5].value -= 1;
        assert_eq!(w.run().unwrap_err(), Refused::CustodyValue { value: 547, escrow: 546 });
        // the change before the payout one sat short: custody's output starts a sat early, so it is not the escrow's sats
        let mut w = world();
        w.sale.outputs[0].value -= 1;
        w.sale.outputs[5].value += 1;
        assert_eq!(w.run().unwrap_err(), Refused::CustodyValue { value: 546, escrow: 546 });
        // 600 sats more before the payout: the inscription lands on the seller's payout
        let mut w = world();
        w.sale.outputs[0].value += 600;
        w.sale.outputs[5].value -= 600;
        assert_eq!(w.run().unwrap_err(), Refused::NotToCustody(Some(1)));
    }

    #[test]
    fn one_escrow_input_spending_output_zero_and_only_the_floats_inputs_beside_it() {
        let mut w = world();
        w.sale.inputs[1].vout = 1;
        assert_eq!(w.run().unwrap_err(), Refused::EscrowInputs(1));
        let mut w = world();
        w.utxos[2].script = p2tr(CUSTODY);
        assert_eq!(w.run().unwrap_err(), Refused::ForeignInput(2), "a purchase never spends custody");
        let mut w = world();
        w.extra[0].push((vec![0x13], vec![7; 64]));
        assert_eq!(w.run().unwrap_err(), Refused::InputSigned(0), "a key-path signature already on our input");
        let mut w = world();
        w.extra[2].push((vec![0x03], 0x83u32.to_le_bytes().to_vec()));
        assert_eq!(w.run().unwrap_err(), Refused::Sighash { input: 2, sighash: 0x83 });
        let mut w = world();
        w.extra[0].push((vec![0x03], 1u32.to_le_bytes().to_vec()));
        assert_eq!(w.run().unwrap_err(), Refused::Sighash { input: 0, sighash: 1 }, "SIGHASH_ALL is refused: the canister signs DEFAULT only (P5)");
    }

    #[test]
    fn the_leaf_must_name_the_pinned_cosigner_and_the_unspendable_key() {
        let mut w = world();
        let rogue = taptree::passthrough_escrow(&SELLER_KEY, &[0x55; 32]).unwrap();
        let mut v = rogue.sale_leaf.clone();
        v.push(TAPSCRIPT);
        w.extra[1][0].1 = v;
        assert_eq!(w.run().unwrap_err(), Refused::LeafUnpinned(1));
        let mut w = world();
        w.extra[1][1].1 = vec![0x66; 32];
        assert_eq!(w.run().unwrap_err(), Refused::EscrowInternalKey(1));
        let mut w = world();
        w.extra[1].clear();
        assert_eq!(w.run().unwrap_err(), Refused::EscrowNotCosignable(1));
        // a leaf for another seller than the escrow's
        let mut w = world();
        let other = taptree::passthrough_escrow(&[0x12; 32], &COSIGNER).unwrap();
        let mut v = other.sale_leaf;
        v.push(TAPSCRIPT);
        w.extra[1][0].1 = v;
        assert_eq!(w.run().unwrap_err(), Refused::EscrowScript);
    }

    #[test]
    fn the_money_royalties_the_price_the_cap_and_the_fee() {
        let mut w = world();
        w.listing.escrow_price = None;
        assert_eq!(w.run().unwrap_err(), Refused::EscrowPriceUnknown);
        // review P-01: satoshi_price bounds nothing; the marketplace fee is bounded on the seller's 50,000: max(1,000, 1,000) + 500
        let mut w = world();
        w.listing.satoshi_price = 50_999;
        assert!(w.run().is_ok(), "satoshi_price is the API's word: it bounds nothing");
        let mut w = world();
        w.sale.outputs[3].value = 1_501;
        w.sale.outputs[5].value -= 501;
        assert_eq!(w.run().unwrap_err(), Refused::MarketFee { paid: 1_501, max: 1_500 });
        // royalties: 10% of the seller's 50,000 is 5,000
        let mut w = world();
        w.sale.outputs[4].value = 5_001;
        w.sale.outputs[5].value -= 4_501;
        assert_eq!(w.run().unwrap_err(), Refused::Royalty { paid: 5_001, max: 5_000 });
        // royalties go only to a creator pinned in the config, whatever the listing says
        let mut w = world();
        w.creators.clear();
        assert_eq!(w.run().unwrap_err(), Refused::UnknownOutput(4), "no creator pinned, no royalty");
        let mut w = world();
        w.listing.creator_address = None;
        assert!(w.run().is_ok(), "the listing's creator is not read: the pinned one is paid");
        let mut w = world();
        w.cap = 52_499;
        assert_eq!(w.run().unwrap_err(), Refused::OverCap { spend: 52_500, cap: 52_499 });
        let mut w = world();
        w.sale.outputs[5].value -= 1_000; // a 2,000 fee on 53,500 of spend; the bound is 1,605
        assert_eq!(w.run().unwrap_err(), Refused::FeeTooHigh { fee: 2_000, bound: 1_605 });
        let mut w = world();
        w.sale.outputs[5].value += 1_000;
        assert_eq!(w.run().unwrap_err(), Refused::NoNetworkFee { inputs: 160_546, outputs: 160_546 });
    }

    fn setup_for(w: &mut World, fee: u64) -> Vec<u8> {
        let tx = Tx { version: 2, inputs: vec![inp(0x99, 0)], outputs: vec![
            TxOut { value: 60_000, script: p2tr(FLOAT) }, TxOut { value: 100_000, script: p2tr(FLOAT) }], locktime: 0 };
        let b = psbt(&tx, &[TxOut { value: 160_000 + fee, script: p2tr(FLOAT) }], &[vec![]]);
        let mut id = Psbt::parse(&b).unwrap().txid();
        id.reverse();
        w.sale.inputs[0] = TxIn { txid: id, vout: 0, sequence: 0xffff_fffd };
        w.sale.inputs[2] = TxIn { txid: id, vout: 1, sequence: 0xffff_fffd };
        b
    }

    #[test]
    fn a_setup_funds_the_sale_and_its_fee_counts_against_the_cap_and_the_bound() {
        let mut w = world();
        let s = setup_for(&mut w, 300);
        w.setup = Some(s.clone());
        let j = w.run().unwrap();
        let setup = j.setup.clone().unwrap();
        assert_eq!((setup.fee, setup.float_inputs.clone()), (300, vec![0]));
        assert_eq!(setup.txid, Psbt::parse(&s).unwrap().txid());
        assert_eq!((j.spend, j.fee), (52_800, 1_300));
        // its fee is bounded with the sale's: 700 more is 2,000 on 53,500
        let mut w = world();
        w.setup = Some(setup_for(&mut w, 1_000));
        assert_eq!(w.run().unwrap_err(), Refused::FeeTooHigh { fee: 2_000, bound: 1_605 });
        // a sale input not from the setup, and one that misstates a setup output's value
        let mut w = world();
        w.setup = Some(setup_for(&mut w, 300));
        w.sale.inputs[2] = inp(0xa2, 1);
        assert_eq!(w.run().unwrap_err(), Refused::NotFundedBySetup(2));
        let mut w = world();
        w.setup = Some(setup_for(&mut w, 300));
        w.utxos[2].value += 1;
        w.sale.outputs[5].value += 1;
        assert_eq!(w.run().unwrap_err(), Refused::NotFundedBySetup(2));
    }

    #[test]
    fn a_setup_is_the_floats_inputs_into_two_float_outputs_with_a_fee() {
        let float = p2tr(FLOAT);
        let two = |outs: Vec<TxOut>, input: TxOut| {
            let tx = Tx { version: 2, inputs: vec![inp(0x99, 0)], outputs: outs, locktime: 0 };
            psbt(&tx, &[input], &[vec![]])
        };
        let f = |v| TxOut { value: v, script: p2tr(FLOAT) };
        assert!(judge_setup(&two(vec![f(1), f(2)], f(10)), &float).is_ok());
        assert_eq!(judge_setup(&two(vec![f(1), f(2), f(3)], f(10)), &float).unwrap_err(), Refused::SetupOutputs(3));
        assert_eq!(judge_setup(&two(vec![f(1), TxOut { value: 2, script: p2tr(9) }], f(10)), &float).unwrap_err(), Refused::SetupOutputNotFloat(1));
        assert_eq!(judge_setup(&two(vec![f(5), f(5)], f(10)), &float).unwrap_err(),
            Refused::InSetup(Box::new(Refused::NoNetworkFee { inputs: 10, outputs: 10 })));
        assert_eq!(judge_setup(&two(vec![f(1), f(2)], TxOut { value: 10, script: p2tr(9) }), &float).unwrap_err(),
            Refused::InSetup(Box::new(Refused::ForeignInput(0))));
    }

    /// Review P-01: the API's own price and creator no longer let it skim up to the cap (the review's proof of concept, fixed).
    #[test]
    fn poc_api_supplied_price_and_creator_no_longer_let_the_api_skim() {
        let mut w = world();
        let attacker = [0xee; 32];
        w.listing.satoshi_price = 200_000;
        w.listing.creator_address = Some(crate::policy::taproot_address(&attacker, "bc"));
        w.sale.outputs[3].value = 129_000;
        w.sale.outputs[4] = TxOut { value: 20_000, script: [vec![0x51, 0x20], attacker.to_vec()].concat() };
        w.utxos[2].value = 300_000;
        w.sale.outputs[5].value = 300_000 + 60_000 - 10_000 - 50_000 - 129_000 - 20_000 - 3_000;
        assert_eq!(w.run().unwrap_err(), Refused::UnknownOutput(4), "the API's creator is not a royalty recipient");
        w.sale.outputs[4] = TxOut { value: 500, script: [vec![0x51, 0x20], vec![0x44; 32]].concat() };
        w.sale.outputs[5].value += 19_500;
        assert_eq!(w.run().unwrap_err(), Refused::MarketFee { paid: 129_000, max: 1_500 }, "the fee is bounded on the seller's price");
    }

    /// Review P-02: with the canister's fee rate read, the miner fee is bounded by twice it on the estimated size.
    #[test]
    fn the_miner_fee_is_bounded_by_our_own_fee_rate() {
        let mut w = world();
        let vsize = est_vsize(&w.sale, Some(1));
        assert!((380..=520).contains(&vsize), "a 3-in 6-out protected sale is about 450 vB: {vsize}");
        w.fee_rate = Some(2);
        assert!(w.run().is_ok(), "1,000 sats is under 2 × 2 sat/vB × {vsize}");
        w.fee_rate = Some(1);
        assert_eq!(w.run().unwrap_err(), Refused::FeeRateTooHigh { fee: 1_000, max: 2 * vsize }, "1,000 is over 2 × 1 sat/vB");
    }

    /// Review P-06: version 2 and locktime 0 on the sale and the parent; no relative lock on a float input (the escrow's sequence is
    /// free).
    #[test]
    fn versions_locktimes_and_relative_locks_are_refused() {
        let mut w = world();
        w.sale.version = 1;
        assert_eq!(w.run().unwrap_err(), Refused::Version { what: "sale", version: 1 });
        let mut w = world();
        w.sale.locktime = 900_000;
        assert_eq!(w.run().unwrap_err(), Refused::LockTime { what: "sale", locktime: 900_000 });
        let mut w = world();
        w.parent.version = 1;
        w.relink();
        assert_eq!(w.run().unwrap_err(), Refused::Version { what: "parent", version: 1 });
        let mut w = world();
        w.sale.inputs[2].sequence = 6; // BIP 68: six blocks
        assert_eq!(w.run().unwrap_err(), Refused::RelativeLock { what: "sale", input: 2 });
        let mut w = world();
        w.sale.inputs[1].sequence = 6;
        assert!(w.run().is_ok(), "the escrow input's sequence is Ordinals Wallet's");
        let tx = Tx { version: 2, inputs: vec![TxIn { txid: [0x99; 32], vout: 0, sequence: 5 }], outputs: vec![
            TxOut { value: 1, script: p2tr(FLOAT) }, TxOut { value: 2, script: p2tr(FLOAT) }], locktime: 0 };
        let b = psbt(&tx, &[TxOut { value: 10, script: p2tr(FLOAT) }], &[vec![]]);
        assert_eq!(judge_setup(&b, &p2tr(FLOAT)).unwrap_err(), Refused::RelativeLock { what: "setup", input: 0 });
    }

    #[test]
    fn every_refusal_reads_as_a_sentence() {
        assert_eq!(Refused::FeeTooHigh { fee: 2, bound: 1 }.to_string(), "a miner fee of 2 sats is over the 1 sat bound");
        assert_eq!(Refused::InSetup(Box::new(Refused::ForeignInput(0))).to_string(), "in the setup: input 0 is not the float's");
        assert_eq!(Refused::NotToCustody(None).to_string(), "the inscribed sat goes to the miner");
        assert_eq!(Refused::EscrowWitness(1).to_string(),
            "escrow input 1's witness is not a 2-of-2 whose seller signature is SINGLE|ANYONECANPAY");
    }
}
