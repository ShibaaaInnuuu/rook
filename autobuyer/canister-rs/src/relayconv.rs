//! Relay conversion: a graduation's own ETH pays for its frog (docs/v6/relay-conversion.md).
//!
//! The Floor Module advances one unit's budget to our EVM address; the runner fetches a Relay quote for exactly what we ask
//! (ETH on 4663 → native BTC at our float address) and hands it in; this module judges it **without trusting anything in it
//! that it does not recompute**. The key fact (measured 2026-09-26, docs §0.2): the deposit's `id` is Relay's
//! `hashStruct(Order)` — the EIP-712 struct hash of `protocol.v2.orderData`, which names the Bitcoin recipient, the minimum
//! sats, the output chain and currency, the deadline and the refund recipients. Recomputing it binds the transaction we sign to
//! the order we inspected, so no outcall to Relay is made and no relayer can substitute a recipient.
//!
//! Pure: no network, no keys, no clock. The flow (`frogflow.rs`) supplies the canister's own reads in [`Want`].
//! Supersedes `relayquote::judge` (2026-09-23), which had to trust `details.recipient`.

use crate::custody::Outpoint;
use crate::evmtx::keccak;
use serde_json::Value;

/// Relay's depository on 4663 (docs §0.1: `/chains` `protocol.v2.depository`, every quote's `to`, and the address's own code;
/// not a proxy). Non-upgradeable, so the address pins the code.
pub const RELAY_DEPOSITORY_4663: [u8; 20] = crate::relayquote::RELAY_DEPOSITORY_4663;
/// The depository's `allocator()` on 2026-09-26: the only key that releases deposits. A rotation (by Relay's security council)
/// changes the trust accepted here, so conversions stop until a reviewed upgrade names the new one.
pub const RELAY_ALLOCATOR_4663: [u8; 20] = [0x63, 0xc1, 0xd3, 0xe9, 0xc6, 0x46, 0x18, 0x45, 0x29, 0xc5, 0x69, 0x46, 0x30, 0xa0, 0x1c,
    0x00, 0xdf, 0x17, 0x1b, 0x56];
/// `allocator()`.
pub const ALLOCATOR: [u8; 4] = [0xaa, 0x5d, 0xce, 0xcc];
/// `depositNative(address depositor, bytes32 id)`.
pub const DEPOSIT_NATIVE: [u8; 4] = crate::bridgecall::RELAY_DEPOSIT_NATIVE;
/// Review round 6 (C6-L-1): the depository's `RelayCallExecuted(bytes32 id, (address to, bytes data, uint256 value, bool allowFailure)
/// call)` (read from its bytecode on both chains, 2026-09-28: the one event beside the two deposits' and the ownership events), emitted
/// only by its allocator-signed `execute`: Relay's own record of a payment out of the depository.
pub const RELAY_CALL_EXECUTED: [u8; 32] = [0x4b, 0xe1, 0x09, 0x45, 0x3e, 0xf7, 0xe8, 0x95, 0xdc, 0x72, 0x15, 0xc9, 0x29, 0xff, 0xf9, 0xb7,
    0x6b, 0x51, 0x48, 0x3d, 0x56, 0xa4, 0xd0, 0x45, 0x48, 0xb4, 0x86, 0x6e, 0x9a, 0xa7, 0xc5, 0xea];

/// Review round 6 (C6-L-1): a `RelayCallExecuted` log as `(id, call.to, call.value)`, the id indexed or not; `None` for any other log.
pub fn call_executed(l: &crate::ethnft::LogV) -> Option<([u8; 32], [u8; 20], u128)> {
    if l.topics.first() != Some(&RELAY_CALL_EXECUTED) {
        return None;
    }
    let word = |i: usize| -> Option<[u8; 32]> { l.data.get(i..i + 32)?.try_into().ok() };
    let num = |w: [u8; 32]| -> Option<usize> { (w[..24] == [0u8; 24]).then(|| u64::from_be_bytes(w[24..].try_into().expect("8")) as usize) };
    // review round 7 (C7-L-1): only the real encoding (one topic, the id in the data), never an indexed id the contract does not emit
    if l.topics.len() != 1 {
        return None;
    }
    let (id, base) = (word(0)?, num(word(32)?)?);
    let to = word(base)?;
    let value = word(base.checked_add(64)?)?;
    if to[..12] != [0u8; 12] || value[..16] != [0u8; 16] {
        return None;
    }
    Some((id, to[12..].try_into().expect("20"), u128::from_be_bytes(value[16..].try_into().expect("16"))))
}

/// Review round 7 (C7-L-1): the depository's `execute(((address,bytes,uint256,bool)[],uint256,uint256),bytes)` (RelayDepository.sol:
/// `execute(CallRequest request, bytes signature)`; the selector of every real call read 2026-09-28).
pub const EXECUTE: [u8; 4] = [0x2d, 0x9f, 0xb4, 0x78];

/// Review round 7 (C7-L-1): one call of a `CallRequest`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepCall {
    pub to: [u8; 20],
    pub data: Vec<u8>,
    pub value: u128,
    pub allow_failure: bool,
}

/// Review round 7 (C7-L-1): a depository `execute`'s request: its calls, the allocator's nonce and the expiry (both as words).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallRequest {
    pub calls: Vec<DepCall>,
    pub nonce: [u8; 32],
    pub expiration: [u8; 32],
}

/// Review round 7 (C7-L-1): a transaction input decoded as the depository's `execute`, every offset and length bounds-checked; `None`
/// for anything else (another selector, a malformed or out-of-range encoding, a value over 128 bits).
pub fn decode_execute(input: &[u8]) -> Option<CallRequest> {
    if input.get(..4)? != EXECUTE {
        return None;
    }
    let a = &input[4..];
    let word = |i: usize| -> Option<[u8; 32]> { a.get(i..i.checked_add(32)?)?.try_into().ok() };
    let num = |i: usize| -> Option<usize> {
        let w = word(i)?;
        (w[..24] == [0u8; 24]).then(|| u64::from_be_bytes(w[24..].try_into().expect("8"))).and_then(|n| usize::try_from(n).ok())
    };
    let req = num(0)?;
    let calls_at = req.checked_add(num(req)?)?;
    let nonce = word(req.checked_add(32)?)?;
    let expiration = word(req.checked_add(64)?)?;
    let n = num(calls_at)?;
    if n > 64 {
        return None;
    }
    let base = calls_at.checked_add(32)?;
    let mut calls = Vec::with_capacity(n);
    for i in 0..n {
        let at = base.checked_add(num(base.checked_add(i.checked_mul(32)?)?)?)?;
        let to = word(at)?;
        let data_at = at.checked_add(num(at.checked_add(32)?)?)?;
        let value = word(at.checked_add(64)?)?;
        let flag = word(at.checked_add(96)?)?;
        let len = num(data_at)?;
        let start = data_at.checked_add(32)?;
        let data = a.get(start..start.checked_add(len)?)?.to_vec();
        if to[..12] != [0u8; 12] || value[..16] != [0u8; 16] || flag[..31] != [0u8; 31] || flag[31] > 1 {
            return None;
        }
        calls.push(DepCall { to: to[12..].try_into().expect("20"), data, value: u128::from_be_bytes(value[16..].try_into().expect("16")),
            allow_failure: flag[31] == 1 });
    }
    Some(CallRequest { calls, nonce, expiration })
}

/// Review round 7 (C7-L-1): the ABI encoding of `execute(request, signature)`, the inverse of [`decode_execute`] (for the tests and
/// the runbook's measurement: nothing signs or sends one).
pub fn encode_execute(r: &CallRequest, signature: &[u8]) -> Vec<u8> {
    let w = |v: usize| {
        let mut x = [0u8; 32];
        x[24..].copy_from_slice(&(v as u64).to_be_bytes());
        x.to_vec()
    };
    let pad = |b: &[u8]| [b.to_vec(), vec![0u8; (32 - b.len() % 32) % 32]].concat();
    let n = r.calls.len();
    let (mut heads, mut bodies) = (Vec::new(), Vec::new());
    for c in &r.calls {
        heads.extend(w(32 * n + bodies.len()));
        let mut to = [0u8; 32];
        to[12..].copy_from_slice(&c.to);
        let mut value = [0u8; 32];
        value[16..].copy_from_slice(&c.value.to_be_bytes());
        bodies.extend([to.to_vec(), w(0x80), value.to_vec(), w(c.allow_failure as usize), w(c.data.len()), pad(&c.data)].concat());
    }
    let req = [w(0x60), r.nonce.to_vec(), r.expiration.to_vec(), w(n), heads, bodies].concat();
    let sig = [w(signature.len()), pad(signature)].concat();
    [EXECUTE.to_vec(), w(0x40), w(0x40 + req.len()), req, sig].concat()
}

/// Review round 7 (C7-L-1): the EIP-712 struct hash of a `CallRequest` (RelayDepository.sol `_hashCallRequest`): the `id` of every
/// `RelayCallExecuted` its `execute` emits. Recomputed from a real transaction's input it equals its logs' id (the test on
/// 0x86eee61f…d84d).
pub fn call_request_hash(r: &CallRequest) -> [u8; 32] {
    let w = |v: u128| {
        let mut x = [0u8; 32];
        x[16..].copy_from_slice(&v.to_be_bytes());
        x
    };
    let call_th = keccak(b"Call(address to,bytes data,uint256 value,bool allowFailure)");
    let req_th = keccak(b"CallRequest(Call[] calls,uint256 nonce,uint256 expiration)Call(address to,bytes data,uint256 value,bool allowFailure)");
    let mut hashes = Vec::with_capacity(32 * r.calls.len());
    for c in &r.calls {
        let mut to = [0u8; 32];
        to[12..].copy_from_slice(&c.to);
        hashes.extend_from_slice(&keccak(&[call_th.as_slice(), &to, &keccak(&c.data), &w(c.value), &w(c.allow_failure as u128)].concat()));
    }
    keccak(&[req_th.as_slice(), &keccak(&hashes), &r.nonce, &r.expiration].concat())
}
/// Relay's chain ids and currency ids.
pub const RELAY_BITCOIN: u64 = 8_253_038;
pub const BTC_NATIVE: &str = "bc1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqmql8k8";
pub const EVM_NATIVE: &str = "0x0000000000000000000000000000000000000000";
/// Relay's protocol names for the two chains a conversion touches.
pub const ORIGIN: &str = "robinhood";
pub const DESTINATION: &str = "bitcoin";

/// The fee bound's default and its ceiling, basis points under our reference rate (docs §3.1 rule 10, §3.5).
pub const DEFAULT_MAX_FEE_BPS: u16 = 300;
pub const MAX_FEE_BPS_CEILING: u16 = 500;
pub const MIN_FEE_BPS_FLOOR: u16 = 50;
/// A handed-in quote is signed within this of its hand-in, or not at all (rule 11).
pub const QUOTE_TTL_SECS: u64 = 300;
/// The order's deadlines must lie in [now + MIN, now + MAX] (rule 11).
pub const MIN_DEADLINE_SECS: u64 = 86_400;
pub const MAX_DEADLINE_SECS: u64 = 8 * 86_400;
/// Three blocks, with the block hash re-checked, before an arrival funds a buy (docs §5.5).
pub const ARRIVAL_CONFS: u64 = 3;
/// A refund is our balance risen by at least this share of the deposit since it mined (docs §5.1).
pub const REFUND_SHARE_BPS: u128 = 5_000;
/// Quotes per advance before the ETH goes back to the module.
pub const MAX_ATTEMPTS: u32 = 3;
/// Kept at our EVM address from each advance for that purchase's own gas (docs §4).
pub const GAS_KEEP_WEI: u128 = 200_000_000_000_000;
/// Past the order's deadline by this with no arrival and no refund: stuck.
pub const DEADLINE_GRACE_SECS: u64 = 3_600;
/// The largest quote accepted from the runner.
pub const MAX_QUOTE_BYTES: usize = 64 * 1024;
/// Conversion review C-01 (2026-10-09): an arrival that has funded no buy for this long may be given up by the operator
/// (`relay_arrived_exit`): the leg is freed and what is at our 4663 address goes back to the module; the sats stay in the float,
/// recorded on the conversion (`abandoned_sats`).
pub const ARRIVED_EXIT_SECS: u64 = 7 * 86_400;
/// Conversion review C-08: a module whose advance came back waits this long before it advances again (the Ethereum side's
/// `RETURN_COOLDOWN_SECS`).
pub const RETURN_COOLDOWN_SECS: u64 = 86_400;

const WEI: u128 = 1_000_000_000_000_000_000;
pub const BPS: u128 = 10_000;

// ---- configuration -------------------------------------------------------------------------------------------------------------

/// The install's `frog_funding` (docs §3.5). `None` in the argument is [`FrogFunding::default`]: Relay, 3 %, no float.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct FrogFunding {
    pub max_fee_bps: u16,
    /// Buy from an operator float first when it covers the frog (reimbursed by `itemArrived`, as before); else Relay.
    pub float_first: bool,
}

impl Default for FrogFunding {
    fn default() -> Self {
        FrogFunding { max_fee_bps: DEFAULT_MAX_FEE_BPS, float_first: false }
    }
}

impl FrogFunding {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_fee_bps < MIN_FEE_BPS_FLOOR || self.max_fee_bps > MAX_FEE_BPS_CEILING {
            return Err(format!("frog_funding.max_fee_bps {} is outside [{MIN_FEE_BPS_FLOOR}, {MAX_FEE_BPS_CEILING}]", self.max_fee_bps));
        }
        Ok(())
    }
}

// ---- the record ------------------------------------------------------------------------------------------------------------------

/// Exactly what the runner must ask Relay for (docs §1 step 3).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct ConvAsk {
    pub amount_wei: u128,
    pub user: String,
    pub recipient: String,
    pub refund_to: String,
    pub origin_chain_id: u64,
    pub destination_chain_id: u64,
    pub origin_currency: String,
    pub destination_currency: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub enum CPhase {
    /// The advance is the next EVM act.
    NeedAdvance,
    /// `advance(amount)` is out.
    Advancing,
    /// The ETH is at our EVM address; the runner is asked for a quote.
    Funded { wei: u128, since: u64 },
    /// The deposit is out.
    Depositing { order_id: [u8; 32], value: u128, min_sats: u64, deadline: u64 },
    /// The deposit mined; watching the float for the arrival and our balance for a refund.
    Converting { order_id: [u8; 32], value: u128, min_sats: u64, deadline: u64, since: u64, balance_after: u128 },
    /// The BTC is in the float, three deep: the frog may be bought with `sats`.
    Arrived { order_id: [u8; 32], sats: u64 },
    /// `returnAdvance{value: wei}` is the next (or current) EVM act.
    Returning { wei: u128 },
    Returned { wei: u128 },
    /// `settleAdvance(cost)` mined: the frog backs the unit.
    Settled { cost: u128, spare_sats: u64 },
    /// No arrival, no refund, past the order's deadline (or an inconsistency): nothing new starts. Loud.
    Stuck { why: String },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Arrival {
    pub outpoint: Outpoint,
    pub sats: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct ConvRec {
    pub frog: u64,
    pub module: [u8; 20],
    /// What the module advanced (0 until the advance is sent).
    pub advanced: u128,
    /// Deposits sent (a refund counts the next one).
    pub attempts: u32,
    pub phase: CPhase,
    /// Every order id accepted for this conversion, in order.
    pub orders: Vec<[u8; 32]>,
    /// Every deposit transaction hash, in order.
    pub deposits: Vec<[u8; 32]>,
    pub arrival: Option<Arrival>,
    /// Some while `Funded`: exactly what the runner must quote.
    pub ask: Option<ConvAsk>,
    /// The last refusal or wait, in words.
    pub note: String,
    /// The float's outpoints just before the deposit was sent: an arrival is an output not among them.
    #[serde(default)]
    pub float_before: Option<Vec<Outpoint>>,
    /// Our 4663 balance just before the deposit was sent.
    #[serde(default)]
    pub balance_before: Option<u128>,
    /// The deposit being watched (kept through `Stuck`, so a late fill or a late refund is still seen).
    #[serde(default)]
    pub last: Option<Watch>,
    /// Conversion review C-01: since when the arrived sats have funded no buy (set at the arrival, and again each time its frog
    /// goes back to wanted): the clock of the operator's exit (`ARRIVED_EXIT_SECS`).
    #[serde(default)]
    pub arrived_at: Option<u64>,
    /// Conversion review C-01: the sats of an arrival the operator gave up (`relay_arrived_exit`): left in the float, owed to the
    /// module in the books. Loud.
    #[serde(default)]
    pub abandoned_sats: Option<u64>,
    /// Conversion review C-03: the live order of a stuck conversion the operator gave back, still watched (and still holding the
    /// Relay leg) until its refund lands and is paid to the module, its fill lands at the float, or the operator releases it.
    #[serde(default)]
    pub late: Option<LateConv>,
    /// Conversion review C-08: when its advance came back (`Returned`), for `RETURN_COOLDOWN_SECS`.
    #[serde(default)]
    pub returned_at: Option<u64>,
}

/// Conversion review C-03: a written-off order (the conversion gave back while it was `Stuck`), kept so its late refund is the
/// module's and is read by nobody else (the Ethereum side's `LateOrder`, C5-M-1).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct LateConv {
    pub watch: Watch,
    pub written_off_at: u64,
    /// Our 4663 balance as the give-back leaves it: a refund raises it by at least half the order's value.
    pub balance_after: u128,
    /// The refund seen, to be paid into the module (`fundReserve`); the act is next while it is set and not credited.
    pub refund_wei: Option<u128>,
    /// What `fundReserve` paid the module, once mined.
    pub credited: Option<u128>,
    /// The order's fill, had it landed at the float after all (the sats stay in the float, recorded here).
    pub filled: Option<Arrival>,
    /// The operator's release (`relay_late_release`), after looking at Relay's own records.
    pub released_at: Option<u64>,
}

impl LateConv {
    /// Still holding the Relay leg: nothing credited, filled or released.
    pub fn open(&self) -> bool {
        self.credited.is_none() && self.filled.is_none() && self.released_at.is_none()
    }
}

/// What a mined deposit is watched for.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, candid::CandidType)]
pub struct Watch {
    pub order_id: [u8; 32],
    pub value: u128,
    pub min_sats: u64,
    pub deadline: u64,
    /// Our balance as the deposit left it (before less the deposit): a refund raises it again.
    pub balance_after: u128,
}

impl ConvRec {
    pub fn new(frog: u64, module: [u8; 20]) -> Self {
        ConvRec { frog, module, advanced: 0, attempts: 0, phase: CPhase::NeedAdvance, orders: vec![], deposits: vec![], arrival: None,
            ask: None, note: String::new(), float_before: None, balance_before: None, last: None, arrived_at: None, abandoned_sats: None,
            late: None, returned_at: None }
    }
    /// Finished records hold nothing; everything else (including `Stuck`) holds the line.
    pub fn finished(&self) -> bool {
        matches!(self.phase, CPhase::Returned { .. } | CPhase::Settled { .. })
    }
    /// The module has paid: the purchase is committed and never released for "no longer needed".
    pub fn committed(&self) -> bool {
        !matches!(self.phase, CPhase::NeedAdvance)
    }
    /// Conversion review C-03: a written-off order still watched.
    pub fn late_open(&self) -> bool {
        self.late.as_ref().is_some_and(|l| l.open())
    }
    pub fn arrived_sats(&self) -> Option<u64> {
        match self.phase {
            CPhase::Arrived { sats, .. } => Some(sats),
            _ => None,
        }
    }
}

/// Every order id ever accepted by any conversion (rule 12).
pub fn used_orders(convs: &[ConvRec]) -> Vec<[u8; 32]> {
    convs.iter().flat_map(|c| c.orders.iter().copied()).collect()
}

/// The ask for `wei` available: all of it less nothing (the gas keep was taken off when `wei` was computed).
pub fn ask(amount_wei: u128, our_evm: &[u8; 20], float_address: &str, chain_id: u64) -> ConvAsk {
    let evm = format!("0x{}", hex::encode(our_evm));
    ConvAsk { amount_wei, user: evm.clone(), recipient: float_address.to_string(), refund_to: evm, origin_chain_id: chain_id,
        destination_chain_id: RELAY_BITCOIN, origin_currency: EVM_NATIVE.into(), destination_currency: BTC_NATIVE.into() }
}

/// What an advance converts: all of it but the gas keep. `None` when nothing would be left.
pub fn convertible(advanced: u128) -> Option<u128> {
    advanced.checked_sub(GAS_KEEP_WEI).filter(|w| *w > 0)
}

/// The least sats `amount_wei` must buy: our reference rate less the fee bound, rounded up (rule 10).
pub fn min_sats_floor(amount_wei: u128, sats_per_eth: u64, max_fee_bps: u16) -> Result<u64, String> {
    if sats_per_eth == 0 {
        return Err("no rate".into());
    }
    let at_rate = amount_wei.checked_mul(sats_per_eth as u128).ok_or("sats: overflow")? / WEI;
    let floor = (at_rate * (BPS - max_fee_bps as u128)).div_ceil(BPS);
    u64::try_from(floor).map_err(|_| "sats: out of range".into())
}

/// Our balance says the deposit came back (docs §5.1): risen by at least half of it since the deposit mined.
pub fn refunded(balance_now: u128, balance_after: u128, value: u128) -> bool {
    balance_now >= balance_after.saturating_add(value * REFUND_SHARE_BPS / BPS)
}

/// The arrival among the float's confirmed outputs: not there when the deposit mined, at least `min_sats`; the largest.
pub fn arrival_among(utxos: &[crate::custody::Utxo], before: &[Outpoint], min_sats: u64) -> Option<Arrival> {
    utxos.iter().filter(|u| u.confirmed && u.value >= min_sats && !before.contains(&u.outpoint))
        .max_by_key(|u| (u.value, std::cmp::Reverse(u.outpoint)))
        .map(|u| Arrival { outpoint: u.outpoint, sats: u.value })
}

/// Conversion review C-05: a fill split across outputs, or Relay's refund on the bitcoin chain (every real quote carries one, to
/// our float, and it may be under the minimum): the float's confirmed outputs that were not there before the deposit, summed, when
/// they reach `REFUND_SHARE_BPS` of `min_sats`. Read only past the order's deadline. The largest names the arrival; `sats` is the
/// sum.
pub fn arrival_summed(utxos: &[crate::custody::Utxo], before: &[Outpoint], min_sats: u64) -> Option<(Arrival, Vec<Outpoint>)> {
    let new: Vec<&crate::custody::Utxo> = utxos.iter().filter(|u| u.confirmed && !before.contains(&u.outpoint)).collect();
    let sum = new.iter().fold(0u64, |a, u| a.saturating_add(u.value));
    if new.is_empty() || (sum as u128) < (min_sats as u128) * REFUND_SHARE_BPS / BPS {
        return None;
    }
    let top = new.iter().max_by_key(|u| (u.value, std::cmp::Reverse(u.outpoint)))?;
    Some((Arrival { outpoint: top.outpoint, sats: sum }, new.iter().map(|u| u.outpoint).collect()))
}

/// The frog's share of the advance: `⌈advanced × spend / arrived⌉`, never above `advanced` (docs §1 step 7).
pub fn cost_of(advanced: u128, spend: u64, arrived: u64) -> Result<u128, String> {
    if arrived == 0 || spend == 0 {
        return Err("no sats arrived or none spent".into());
    }
    if spend > arrived {
        return Err(format!("a buy of {spend} sats is over the {arrived} that arrived"));
    }
    Ok(advanced.checked_mul(spend as u128).ok_or("cost: overflow")?.div_ceil(arrived as u128).min(advanced))
}

// ---- calls -----------------------------------------------------------------------------------------------------------------------

pub const ADVANCE: [u8; 4] = [0x71, 0xd6, 0xdd, 0xd6];
pub const SETTLE_ADVANCE: [u8; 4] = [0x15, 0x97, 0x69, 0x1c];
pub const RETURN_ADVANCE: [u8; 4] = [0xf9, 0x04, 0x4b, 0xf0];
pub const TRANSIT_UNITS: [u8; 4] = [0x12, 0xbc, 0x60, 0xd6];

pub fn advance_call(amount: u128) -> Vec<u8> {
    crate::abi::encode_call(ADVANCE, &[crate::abi::Arg::Word(crate::abi::uint_word(amount))])
}
pub fn settle_call(cost: u128) -> Vec<u8> {
    crate::abi::encode_call(SETTLE_ADVANCE, &[crate::abi::Arg::Word(crate::abi::uint_word(cost))])
}
pub fn return_call() -> Vec<u8> {
    RETURN_ADVANCE.to_vec()
}

// ---- the order hash (EIP-712 `hashStruct`, Relay's settlement SDK `getOrderId`) --------------------------------------------------

const T_ORDER: &str = "Order(string version,string solverChainId,address solver,uint256 salt,Input[] inputs,Output output,Fee[] fees)";
const T_FEE: &str = "Fee(string recipientChainId,bytes recipient,string currencyChainId,bytes currency,uint256 amount)";
const T_INPUT: &str = "Input(InputPayment payment,InputRefund[] refunds)";
const T_INPUT_PAYMENT: &str = "InputPayment(string chainId,bytes currency,uint256 amount,uint256 weight)";
const T_INPUT_REFUND: &str = "InputRefund(string chainId,bytes recipient,bytes currency,uint256 minimumAmount,uint32 deadline,bytes extraData)";
const T_OUTPUT: &str = "Output(string chainId,OutputPayment[] payments,uint32 deadline,bytes[] calls,bytes extraData)";
const T_OUTPUT_PAYMENT: &str = "OutputPayment(bytes recipient,bytes currency,uint256 minimumAmount,uint256 expectedAmount)";

fn type_hash(parts: &[&str]) -> [u8; 32] {
    keccak(parts.concat().as_bytes())
}

/// How Relay encodes a chain's addresses: the chains a conversion may name, and nothing guessed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Vm {
    Evm,
    Bitcoin,
    /// Relay's `encodeAddress(addr, "solana-vm")`: base58, exactly 32 bytes (spike Q5 §2, docs/v9/spikes/q5-relay-solana-legs.md).
    Solana,
}

fn vm_of(chain: &str) -> Result<Vm, String> {
    match chain {
        "robinhood" | "base" | "ethereum" | "arbitrum" | "optimism" => Ok(Vm::Evm),
        "bitcoin" => Ok(Vm::Bitcoin),
        "solana" => Ok(Vm::Solana),
        c => Err(format!("the order names chain {c:?}, which this judge does not encode")),
    }
}

/// Relay's `encodeAddress`: EVM, 20 bytes; Solana, the 32 raw bytes of the base58 address; Bitcoin segwit (bech32 for version 0, bech32m above), the witness version ‖ the
/// program, **without** segwit's program-length rules (Relay's native-BTC id `bc1qqq…mql8k8` is a 19-byte "program": it is an
/// id, not an address). Legacy (Base58) addresses are refused: a conversion names only our own taproot address and that id.
pub fn encode_address(addr: &str, chain: &str) -> Result<Vec<u8>, String> {
    match vm_of(chain)? {
        Vm::Evm => Ok(crate::evmtx::parse_address(addr)?.to_vec()),
        Vm::Bitcoin => bech32_words(addr),
        Vm::Solana => crate::soltx::pubkey(addr).map(|k| k.to_vec()),
    }
}

const CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

fn polymod(values: &[u8]) -> u32 {
    const GEN: [u32; 5] = [0x3b6a_57b2, 0x2650_8e6d, 0x1ea1_19fa, 0x3d42_33dd, 0x2a14_62b3];
    let mut chk: u32 = 1;
    for v in values {
        let b = chk >> 25;
        chk = ((chk & 0x01ff_ffff) << 5) ^ *v as u32;
        for (i, g) in GEN.iter().enumerate() {
            if (b >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

/// `bc1…` → witness version ‖ the 8-bit regrouping of the rest (no padding beyond 4 zero bits), as the SDK's `bech32.fromWords`.
fn bech32_words(addr: &str) -> Result<Vec<u8>, String> {
    if addr.chars().any(|c| c.is_ascii_uppercase()) {
        return Err(format!("{addr:?}: only lower-case addresses are encoded"));
    }
    let pos = addr.rfind('1').ok_or_else(|| format!("{addr:?} is not bech32"))?;
    let (hrp, rest) = (&addr[..pos], &addr[pos + 1..]);
    if hrp != "bc" || rest.len() < 7 || addr.len() > 90 {
        return Err(format!("{addr:?} is not a Bitcoin mainnet segwit address"));
    }
    let data: Vec<u8> = rest.bytes().map(|c| CHARSET.iter().position(|x| *x == c).map(|p| p as u8)).collect::<Option<_>>()
        .ok_or_else(|| format!("{addr:?} has a non-bech32 character"))?;
    let mut exp: Vec<u8> = hrp.bytes().map(|c| c >> 5).collect();
    exp.push(0);
    exp.extend(hrp.bytes().map(|c| c & 31));
    exp.extend_from_slice(&data);
    let want = if data[0] == 0 { 1 } else { 0x2bc8_30a3 };
    if polymod(&exp) != want || data[0] > 16 {
        return Err(format!("{addr:?}: bad checksum for its witness version"));
    }
    let mut out = vec![data[0]];
    let (mut acc, mut bits) = (0u32, 0u32);
    for v in &data[1..data.len() - 6] {
        acc = (acc << 5) | *v as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if bits >= 5 || acc != 0 {
        return Err(format!("{addr:?}: bad padding"));
    }
    Ok(out)
}

pub(crate) fn get<'a>(v: &'a Value, k: &str, at: &str) -> Result<&'a Value, String> {
    v.get(k).ok_or_else(|| format!("{at} has no {k}"))
}
pub(crate) fn text<'a>(v: &'a Value, k: &str, at: &str) -> Result<&'a str, String> {
    get(v, k, at)?.as_str().ok_or_else(|| format!("{at}.{k} is not a string"))
}
pub(crate) fn list<'a>(v: &'a Value, k: &str, at: &str) -> Result<&'a Vec<Value>, String> {
    get(v, k, at)?.as_array().ok_or_else(|| format!("{at}.{k} is not a list"))
}
/// A decimal string, a `0x` hex string or a JSON number, as a 256-bit word.
fn uint_word(v: &Value, k: &str, at: &str) -> Result<[u8; 32], String> {
    let x = get(v, k, at)?;
    let s = x.as_str().map(|s| s.to_string()).or_else(|| x.as_u64().map(|n| n.to_string()))
        .ok_or_else(|| format!("{at}.{k} is not a number"))?;
    let mut w = [0u8; 32];
    if let Some(h) = s.strip_prefix("0x") {
        if h.is_empty() || h.len() > 64 {
            return Err(format!("{at}.{k} is not a 256-bit hex number"));
        }
        let b = hex::decode(format!("{h:0>64}")).map_err(|_| format!("{at}.{k} is not hex"))?;
        w.copy_from_slice(&b);
    } else {
        let n: u128 = s.parse().map_err(|_| format!("{at}.{k} = {s:?} is not a whole number (or over 128 bits)"))?;
        w[16..].copy_from_slice(&n.to_be_bytes());
    }
    Ok(w)
}
pub(crate) fn uint_of(v: &Value, k: &str, at: &str) -> Result<u128, String> {
    let w = uint_word(v, k, at)?;
    if w[..16].iter().any(|b| *b != 0) {
        return Err(format!("{at}.{k} is over 128 bits"));
    }
    Ok(u128::from_be_bytes(w[16..].try_into().expect("16 bytes")))
}
pub(crate) fn bytes_of(v: &Value, k: &str, at: &str) -> Result<Vec<u8>, String> {
    let s = text(v, k, at)?;
    hex::decode(s.strip_prefix("0x").ok_or_else(|| format!("{at}.{k} is not 0x-hex"))?).map_err(|_| format!("{at}.{k} is not hex"))
}
fn h_str(s: &str) -> [u8; 32] {
    keccak(s.as_bytes())
}
fn h_bytes(b: &[u8]) -> [u8; 32] {
    keccak(b)
}
fn struct_hash(th: [u8; 32], fields: &[[u8; 32]]) -> [u8; 32] {
    let mut enc = th.to_vec();
    for f in fields {
        enc.extend_from_slice(f);
    }
    keccak(&enc)
}
fn array_hash(items: &[[u8; 32]]) -> [u8; 32] {
    keccak(&items.concat())
}
fn addr_word(a: &str, at: &str) -> Result<[u8; 32], String> {
    let b = crate::evmtx::parse_address(a).map_err(|e| format!("{at}: {e}"))?;
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(&b);
    Ok(w)
}

/// `hashStruct(Order)` of `protocol.v2.orderData`: Relay's order id.
pub fn order_id(o: &Value) -> Result<[u8; 32], String> {
    let at = "orderData";
    let t_input = type_hash(&[T_INPUT, T_INPUT_PAYMENT, T_INPUT_REFUND]);
    let t_payment = type_hash(&[T_INPUT_PAYMENT]);
    let t_refund = type_hash(&[T_INPUT_REFUND]);
    let t_output = type_hash(&[T_OUTPUT, T_OUTPUT_PAYMENT]);
    let t_out_payment = type_hash(&[T_OUTPUT_PAYMENT]);
    let t_fee = type_hash(&[T_FEE]);
    let t_order = type_hash(&[T_ORDER, T_FEE, T_INPUT, T_INPUT_PAYMENT, T_INPUT_REFUND, T_OUTPUT, T_OUTPUT_PAYMENT]);

    let mut inputs = Vec::new();
    for (i, input) in list(o, "inputs", at)?.iter().enumerate() {
        let at = format!("orderData.inputs[{i}]");
        let p = get(input, "payment", &at)?;
        let pat = format!("{at}.payment");
        let pc = text(p, "chainId", &pat)?;
        let payment = struct_hash(t_payment, &[h_str(pc), h_bytes(&encode_address(text(p, "currency", &pat)?, pc)?),
            uint_word(p, "amount", &pat)?, uint_word(p, "weight", &pat)?]);
        let mut refunds = Vec::new();
        for (j, r) in list(input, "refunds", &at)?.iter().enumerate() {
            let rat = format!("{at}.refunds[{j}]");
            let rc = text(r, "chainId", &rat)?;
            refunds.push(struct_hash(t_refund, &[h_str(rc), h_bytes(&encode_address(text(r, "recipient", &rat)?, rc)?),
                h_bytes(&encode_address(text(r, "currency", &rat)?, rc)?), uint_word(r, "minimumAmount", &rat)?,
                uint_word(r, "deadline", &rat)?, h_bytes(&bytes_of(r, "extraData", &rat)?)]));
        }
        inputs.push(struct_hash(t_input, &[payment, array_hash(&refunds)]));
    }
    let out = get(o, "output", at)?;
    let oat = "orderData.output";
    let oc = text(out, "chainId", oat)?;
    let mut payments = Vec::new();
    for (i, p) in list(out, "payments", oat)?.iter().enumerate() {
        let pat = format!("{oat}.payments[{i}]");
        payments.push(struct_hash(t_out_payment, &[h_bytes(&encode_address(text(p, "recipient", &pat)?, oc)?),
            h_bytes(&encode_address(text(p, "currency", &pat)?, oc)?), uint_word(p, "minimumAmount", &pat)?,
            uint_word(p, "expectedAmount", &pat)?]));
    }
    let mut calls = Vec::new();
    for (i, c) in list(out, "calls", oat)?.iter().enumerate() {
        let s = c.as_str().ok_or_else(|| format!("{oat}.calls[{i}] is not a string"))?;
        calls.push(h_bytes(&hex::decode(s.trim_start_matches("0x")).map_err(|_| format!("{oat}.calls[{i}] is not hex"))?));
    }
    let output = struct_hash(t_output, &[h_str(oc), array_hash(&payments), uint_word(out, "deadline", oat)?, array_hash(&calls),
        h_bytes(&bytes_of(out, "extraData", oat)?)]);
    let mut fees = Vec::new();
    for (i, f) in list(o, "fees", at)?.iter().enumerate() {
        let fat = format!("orderData.fees[{i}]");
        let (rc, cc) = (text(f, "recipientChainId", &fat)?, text(f, "currencyChainId", &fat)?);
        fees.push(struct_hash(t_fee, &[h_str(rc), h_bytes(&encode_address(text(f, "recipient", &fat)?, rc)?), h_str(cc),
            h_bytes(&encode_address(text(f, "currency", &fat)?, cc)?), uint_word(f, "amount", &fat)?]));
    }
    Ok(struct_hash(t_order, &[h_str(text(o, "version", at)?), h_str(text(o, "solverChainId", at)?), addr_word(text(o, "solver", at)?, at)?,
        uint_word(o, "salt", at)?, array_hash(&inputs), output, array_hash(&fees)]))
}

// ---- the judge -------------------------------------------------------------------------------------------------------------------

/// Everything the quote is judged against: the canister's own values and reads (docs §3.1).
pub struct Want<'a> {
    pub chain_id: u64,
    pub our_evm: [u8; 20],
    pub float_address: &'a str,
    pub amount_wei: u128,
    pub sats_per_eth: u64,
    pub max_fee_bps: u16,
    pub allocator: [u8; 20],
    pub now: u64,
    pub handed_in_at: u64,
    pub used: &'a [[u8; 32]],
}

/// The deposit to sign, from a quote that passed every rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvDeposit {
    pub to: [u8; 20],
    pub value: u128,
    pub data: Vec<u8>,
    pub order_id: [u8; 32],
    pub min_sats: u64,
    pub deadline: u64,
    pub request_id: String,
}

fn same_btc(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}
pub(crate) fn same_evm(a: &str, ours: &[u8; 20]) -> bool {
    crate::evmtx::parse_address(a).is_ok_and(|x| x == *ours)
}
pub(crate) fn in_band(t: u64, now: u64) -> bool {
    t >= now.saturating_add(MIN_DEADLINE_SECS) && t <= now.saturating_add(MAX_DEADLINE_SECS)
}

/// Conversion review C-07: a deadline as Relay's order types it (`uint32`): anything above `u32::MAX` is refused, never truncated.
fn deadline_of(v: &Value, k: &str, at: &str) -> Result<u64, String> {
    let d = uint_of(v, k, at)?;
    u32::try_from(d).map(u64::from).map_err(|_| format!("rule 11: {at}.{k} = {d} is over a uint32"))
}

/// Judge the runner's quote. Every refusal names the rule and the field.
pub fn judge(body: &[u8], w: &Want) -> Result<ConvDeposit, String> {
    if body.len() > MAX_QUOTE_BYTES {
        return Err(format!("the quote is {} bytes, over {MAX_QUOTE_BYTES}", body.len()));
    }
    // rule 11 (first: a stale hand-in is not worth parsing)
    if w.now.saturating_sub(w.handed_in_at) > QUOTE_TTL_SECS {
        return Err(format!("stale: handed in {} s ago, over the {QUOTE_TTL_SECS} s a quote lives here", w.now.saturating_sub(w.handed_in_at)));
    }
    let q: Value = serde_json::from_slice(body).map_err(|e| format!("not JSON: {e}"))?;
    // rule 1: one deposit transaction
    let steps = list(&q, "steps", "the quote")?;
    if steps.len() != 1 {
        return Err(format!("rule 1: {} steps; only a single deposit step is signed", steps.len()));
    }
    let step = &steps[0];
    if text(step, "id", "steps[0]")? != "deposit" || text(step, "kind", "steps[0]")? != "transaction" {
        return Err("rule 1: the one step is not a deposit transaction".into());
    }
    let items = list(step, "items", "steps[0]")?;
    if items.len() != 1 {
        return Err(format!("rule 1: {} items; exactly one transaction is signed", items.len()));
    }
    let d = get(&items[0], "data", "steps[0].items[0]")?;
    let dat = "the deposit";
    // rule 2: the chain
    if uint_of(d, "chainId", dat)? != w.chain_id as u128 {
        return Err(format!("rule 2: the deposit is for chain {}, not {}", uint_of(d, "chainId", dat)?, w.chain_id));
    }
    // rule 3: the allowlisted depository, and the allocator we accepted
    let to = crate::evmtx::parse_address(text(d, "to", dat)?)?;
    if to != RELAY_DEPOSITORY_4663 {
        return Err(format!("rule 3: the deposit goes to 0x{}, not Relay's pinned depository", hex::encode(to)));
    }
    if w.allocator != RELAY_ALLOCATOR_4663 {
        return Err(format!("rule 3: the depository's allocator is 0x{}, not the pinned 0x{}: Relay rotated its key; review before converting",
            hex::encode(w.allocator), hex::encode(RELAY_ALLOCATOR_4663)));
    }
    if let Ok(from) = text(d, "from", dat) {
        if !same_evm(from, &w.our_evm) {
            return Err("rule 5: the quote was made for another sender".into());
        }
    }
    // rule 4: the value
    let value = uint_of(d, "value", dat)?;
    if value != w.amount_wei {
        return Err(format!("rule 4: the deposit sends {value} wei, not the {} asked", w.amount_wei));
    }
    // rule 5: the calldata
    let data = bytes_of(d, "data", dat)?;
    if data.len() != 68 || data[..4] != DEPOSIT_NATIVE {
        return Err("rule 5: the calldata is not depositNative(address, bytes32)".into());
    }
    if data[4..16].iter().any(|b| *b != 0) || data[16..36] != w.our_evm {
        return Err("rule 5: the calldata names another depositor".into());
    }
    let id: [u8; 32] = data[36..68].try_into().expect("32 bytes");
    // rule 6: the order is the order we inspect
    let v2 = get(get(&q, "protocol", "the quote")?, "v2", "protocol")?;
    let o = get(v2, "orderData", "protocol.v2")?;
    let computed = order_id(o).map_err(|e| format!("rule 6: {e}"))?;
    if computed != id {
        return Err(format!("rule 6: the calldata commits to 0x{}, but the order hashes to 0x{}", hex::encode(id), hex::encode(computed)));
    }
    let quoted = bytes_of(v2, "orderId", "protocol.v2")?;
    if quoted != id {
        return Err("rule 6: protocol.v2.orderId is not the deposit's id".into());
    }
    let pd = get(v2, "paymentDetails", "protocol.v2")?;
    if crate::evmtx::parse_address(text(pd, "depository", "paymentDetails")?)? != RELAY_DEPOSITORY_4663
        || uint_of(pd, "amount", "paymentDetails")? != value {
        return Err("rule 3/4: paymentDetails names another depository or amount".into());
    }
    // rule 7: the input
    let inputs = list(o, "inputs", "orderData")?;
    if inputs.len() != 1 {
        return Err(format!("rule 7: {} inputs, not one", inputs.len()));
    }
    let p = get(&inputs[0], "payment", "orderData.inputs[0]")?;
    if text(p, "chainId", "payment")? != ORIGIN || !same_evm(text(p, "currency", "payment")?, &[0u8; 20])
        || uint_of(p, "amount", "payment")? != value {
        return Err("rule 7: the order's payment is not native ETH on robinhood for the deposit's value".into());
    }
    // rule 8: every refund is ours
    let refunds = list(&inputs[0], "refunds", "orderData.inputs[0]")?;
    if refunds.is_empty() {
        return Err("rule 8: the order has no refund".into());
    }
    let mut origin_refund = false;
    for (i, r) in refunds.iter().enumerate() {
        let rat = format!("refunds[{i}]");
        let (chain, to, cur) = (text(r, "chainId", &rat)?, text(r, "recipient", &rat)?, text(r, "currency", &rat)?);
        let ok = match chain {
            ORIGIN => same_evm(to, &w.our_evm) && same_evm(cur, &[0u8; 20]),
            DESTINATION => same_btc(to, w.float_address) && same_btc(cur, BTC_NATIVE),
            _ => false,
        };
        if !ok {
            return Err(format!("rule 8: {rat} pays {to} on {chain}, not our own address in its native currency"));
        }
        origin_refund |= chain == ORIGIN;
        // conversion review C-07: the field is a uint32 in Relay's order; never truncated
        let dl = deadline_of(r, "deadline", &rat)?;
        if !in_band(dl, w.now) {
            return Err(format!("rule 11: {rat}'s deadline {dl} is outside [now + 1 day, now + 8 days]"));
        }
    }
    if !origin_refund {
        return Err("rule 8: no refund on robinhood to our EVM address".into());
    }
    // rule 9: the output
    let out = get(o, "output", "orderData")?;
    if text(out, "chainId", "output")? != DESTINATION {
        return Err("rule 9: the order pays out on another chain".into());
    }
    let pays = list(out, "payments", "output")?;
    if pays.len() != 1 {
        return Err(format!("rule 9: {} payments, not one", pays.len()));
    }
    let pay = &pays[0];
    if !same_btc(text(pay, "recipient", "payment")?, w.float_address) {
        return Err(format!("rule 9: the order pays {}, not our float {}", text(pay, "recipient", "payment")?, w.float_address));
    }
    if !same_btc(text(pay, "currency", "payment")?, BTC_NATIVE) {
        return Err("rule 9: the order does not pay native BTC".into());
    }
    if !list(out, "calls", "output")?.is_empty() {
        return Err("rule 9: the order carries calls".into());
    }
    if !list(o, "fees", "orderData")?.is_empty() {
        return Err("rule 9: the order carries fees".into());
    }
    // rule 10: the minimum out, against our own rate
    let min = uint_of(pay, "minimumAmount", "payment")?;
    let min: u64 = min.try_into().map_err(|_| "rule 10: the minimum is out of range")?;
    let floor = min_sats_floor(value, w.sats_per_eth, w.max_fee_bps)?;
    if min < floor {
        return Err(format!("rule 10: the order guarantees {min} sats, under our floor of {floor} ({} sat/ETH less {} bps)",
            w.sats_per_eth, w.max_fee_bps));
    }
    // rule 11: the deadline band
    let deadline = deadline_of(out, "deadline", "output")?;
    if !in_band(deadline, w.now) {
        return Err(format!("rule 11: the order's deadline {deadline} is outside [now + 1 day, now + 8 days]"));
    }
    // rule 12: replay
    if w.used.contains(&id) {
        return Err(format!("rule 12: order 0x{} was accepted before: never twice", hex::encode(id)));
    }
    Ok(ConvDeposit { to, value, data, order_id: id, min_sats: min, deadline,
        request_id: step.get("requestId").and_then(|x| x.as_str()).unwrap_or_default().to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const Q26: &str = include_str!("../tests/fixtures/relay-quote-4663-btc-2026-09-26.json");
    const Q23: &str = include_str!("../tests/fixtures/relay-quote-4663-btc-2026-09-23.json");
    const Q23_BASE: &str = include_str!("../tests/fixtures/seaport/relay-quote-4663-8453-2026-09-23.json");
    const OURS: [u8; 20] = [0xb0, 0x2e, 0x4e, 0xc0, 0x2d, 0xdc, 0xcb, 0x6f, 0x68, 0xcf, 0xa4, 0x7b, 0x84, 0xee, 0xad, 0xeb, 0xc6, 0x38,
        0x6a, 0xb2];
    const FLOAT: &str = "bc1p7ynpknzp0kene8twkvyarncd4740x37955hxu3d8xsczg9vn3a8q4v86hp";
    /// The quote was made at 1,790,441,670; its deadline is 1,791,046,471.
    const NOW: u64 = 1_790_441_700;
    /// ≈ $85,000/BTC at ≈ $2,690/ETH: 3,164,000 sats per ETH (the quote's own 505,655 / 0.16 is 3,160,343).
    const RATE: u64 = 3_164_000;

    fn want(used: &[[u8; 32]]) -> Want<'_> {
        Want { chain_id: 4663, our_evm: OURS, float_address: FLOAT, amount_wei: 160_000_000_000_000_000, sats_per_eth: RATE,
            max_fee_bps: 300, allocator: RELAY_ALLOCATOR_4663, now: NOW, handed_in_at: NOW - 20, used }
    }
    fn edit(f: impl Fn(&mut Value)) -> Vec<u8> {
        let mut v: Value = serde_json::from_str(Q26).unwrap();
        f(&mut v);
        serde_json::to_vec(&v).unwrap()
    }
    fn id_of(q: &str) -> [u8; 32] {
        let v: Value = serde_json::from_str(q).unwrap();
        let h = v["protocol"]["v2"]["orderId"].as_str().unwrap();
        hex::decode(&h[2..]).unwrap().try_into().unwrap()
    }

    #[test]
    fn the_order_id_is_relays_on_three_real_quotes() {
        for q in [Q26, Q23, Q23_BASE] {
            let v: Value = serde_json::from_str(q).unwrap();
            assert_eq!(order_id(&v["protocol"]["v2"]["orderData"]).unwrap(), id_of(q));
        }
        assert_eq!(hex::encode(id_of(Q26)), "fef9de1e3046989ca671815e8444edb2fa9fa472b05c09c6014f6a6505ea5fdf");
    }

    #[test]
    fn selectors_are_the_contracts() {
        assert_eq!(ADVANCE, keccak(b"advance(uint256)")[..4]);
        assert_eq!(SETTLE_ADVANCE, keccak(b"settleAdvance(uint256)")[..4]);
        assert_eq!(RETURN_ADVANCE, keccak(b"returnAdvance()")[..4]);
        assert_eq!(TRANSIT_UNITS, keccak(b"transitUnits()")[..4]);
        assert_eq!(ALLOCATOR, keccak(b"allocator()")[..4]);
        assert_eq!(DEPOSIT_NATIVE, keccak(b"depositNative(address,bytes32)")[..4]);
        assert_eq!(RELAY_CALL_EXECUTED, keccak(b"RelayCallExecuted(bytes32,(address,bytes,uint256,bool))"));
    }

    #[test]
    fn the_real_quote_passes_and_yields_the_deposit() {
        let d = judge(Q26.as_bytes(), &want(&[])).unwrap();
        assert_eq!((d.to, d.value, d.min_sats, d.deadline), (RELAY_DEPOSITORY_4663, 160_000_000_000_000_000, 495_542, 1_791_046_471));
        assert_eq!(d.order_id, id_of(Q26));
        assert_eq!(&d.data[..4], &DEPOSIT_NATIVE);
        assert_eq!(&d.data[16..36], &OURS);
    }

    /// Every rule of docs/v6/relay-conversion.md §3.1, broken one at a time.
    #[test]
    fn every_rule_refuses_its_lie() {
        let other_btc = "bc1pw508d6qejxtdg4y5r3zarvary0c5xw7kw508d6qejxtdg4y5r3zarvary0c5xw7kt5nd6y";
        let beef = "0x000000000000000000000000000000000000beef";
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("rule 1", edit(|v| { let s = v["steps"][0].clone(); v["steps"].as_array_mut().unwrap().push(s); })),
            ("rule 1", edit(|v| v["steps"][0]["kind"] = "signature".into())),
            ("rule 1", edit(|v| { let i = v["steps"][0]["items"][0].clone(); v["steps"][0]["items"].as_array_mut().unwrap().push(i); })),
            ("rule 2", edit(|v| v["steps"][0]["items"][0]["data"]["chainId"] = 8453.into())),
            ("rule 3", edit(|v| v["steps"][0]["items"][0]["data"]["to"] = beef.into())),
            ("rule 3/4", edit(|v| v["protocol"]["v2"]["paymentDetails"]["depository"] = beef.into())),
            ("rule 4", edit(|v| v["steps"][0]["items"][0]["data"]["value"] = "170000000000000000".into())),
            ("rule 5", edit(|v| v["steps"][0]["items"][0]["data"]["from"] = beef.into())),
            ("rule 5", edit(|v| { let d = v["steps"][0]["items"][0]["data"]["data"].as_str().unwrap().replace("b02e4ec0", "deadbeef");
                v["steps"][0]["items"][0]["data"]["data"] = d.into(); })),
            ("rule 5", edit(|v| { let d = v["steps"][0]["items"][0]["data"]["data"].as_str().unwrap().replace("0x49290c1c", "0xe8017952");
                v["steps"][0]["items"][0]["data"]["data"] = d.into(); })),
            // the recipient changed where Relay would read it: the hash no longer matches the calldata
            ("rule 6", edit(|v| v["protocol"]["v2"]["orderData"]["output"]["payments"][0]["recipient"] = other_btc.into())),
            ("rule 6", edit(|v| v["protocol"]["v2"]["orderData"]["output"]["payments"][0]["minimumAmount"] = "1".into())),
            ("rule 6", edit(|v| v["protocol"]["v2"]["orderData"]["inputs"][0]["refunds"][0]["recipient"] = beef.into())),
            ("rule 6", edit(|v| v["protocol"]["v2"]["orderId"] = "0x0000000000000000000000000000000000000000000000000000000000000001".into())),
            ("rule 6", edit(|v| v["protocol"]["v2"]["orderData"]["output"]["chainId"] = "solana".into())),
        ];
        for (rule, body) in cases {
            let e = judge(&body, &want(&[])).unwrap_err();
            assert!(e.contains(rule), "{rule}: {e}");
        }
    }

    /// The rules that a self-consistent order (hash recomputed) must still pass: a relayer that re-hashes its own lie.
    #[test]
    fn a_consistent_order_that_is_not_ours_is_refused() {
        // rebuild a quote around an edited orderData, with the id recomputed everywhere, so rule 6 passes and the content rule bites
        fn consistent(f: impl Fn(&mut Value)) -> Vec<u8> {
            let mut v: Value = serde_json::from_str(Q26).unwrap();
            f(&mut v["protocol"]["v2"]["orderData"]);
            let id = hex::encode(order_id(&v["protocol"]["v2"]["orderData"]).unwrap());
            v["protocol"]["v2"]["orderId"] = format!("0x{id}").into();
            let d = v["steps"][0]["items"][0]["data"]["data"].as_str().unwrap().to_string();
            v["steps"][0]["items"][0]["data"]["data"] = format!("{}{id}", &d[..2 + 72]).into();
            serde_json::to_vec(&v).unwrap()
        }
        let other_btc = "bc1pw508d6qejxtdg4y5r3zarvary0c5xw7kw508d6qejxtdg4y5r3zarvary0c5xw7kt5nd6y";
        let beef = "0x000000000000000000000000000000000000beef";
        assert!(judge(&consistent(|_| {}), &want(&[])).is_ok(), "the rebuild itself is sound");
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("rule 7", consistent(|o| o["inputs"][0]["payment"]["chainId"] = "base".into())),
            ("rule 7", consistent(|o| o["inputs"][0]["payment"]["amount"] = "1".into())),
            ("rule 7", consistent(|o| { let i = o["inputs"][0].clone(); o["inputs"].as_array_mut().unwrap().push(i); })),
            ("rule 8", consistent(|o| o["inputs"][0]["refunds"][0]["recipient"] = beef.into())),
            ("rule 8", consistent(|o| o["inputs"][0]["refunds"][1]["recipient"] = other_btc.into())),
            ("rule 8", consistent(|o| { o["inputs"][0]["refunds"].as_array_mut().unwrap().remove(0); })),
            ("rule 8", consistent(|o| o["inputs"][0]["refunds"] = serde_json::json!([]))),
            ("rule 9", consistent(|o| o["output"]["payments"][0]["recipient"] = other_btc.into())),
            ("rule 9", consistent(|o| { o["output"]["chainId"] = "base".into();
                o["output"]["payments"][0]["recipient"] = "0xb02e4ec02ddccb6f68cfa47b84eeadebc6386ab2".into();
                o["output"]["payments"][0]["currency"] = EVM_NATIVE.into(); })),
            ("rule 9", consistent(|o| o["output"]["calls"] = serde_json::json!(["0x00"]))),
            ("rule 9", consistent(|o| { let p = o["output"]["payments"][0].clone(); o["output"]["payments"].as_array_mut().unwrap().push(p); })),
            ("rule 10", consistent(|o| o["output"]["payments"][0]["minimumAmount"] = "400000".into())),
            ("rule 11", consistent(|o| o["output"]["deadline"] = (NOW + 3_600).into())),
            ("rule 11", consistent(|o| o["output"]["deadline"] = (NOW + 30 * 86_400).into())),
            ("rule 11", consistent(|o| o["inputs"][0]["refunds"][0]["deadline"] = (NOW + 60).into())),
        ];
        for (rule, body) in cases {
            let e = judge(&body, &want(&[])).unwrap_err();
            assert!(e.contains(rule), "{rule}: {e}");
        }
    }

    #[test]
    fn our_own_reads_bind_too() {
        let body = Q26.as_bytes();
        // the rate: a solver 3 % under our reference is refused; the default fee bound passes today's quote
        assert!(judge(body, &Want { sats_per_eth: 3_300_000, ..want(&[]) }).unwrap_err().contains("rule 10"));
        assert!(judge(body, &Want { max_fee_bps: 100, ..want(&[]) }).unwrap_err().contains("rule 10"));
        // a rotated allocator
        assert!(judge(body, &Want { allocator: [7; 20], ..want(&[]) }).unwrap_err().contains("rule 3"));
        // the amount asked
        assert!(judge(body, &Want { amount_wei: 159_999_999_999_999_999, ..want(&[]) }).unwrap_err().contains("rule 4"));
        // another float address (ours changed): the quote names someone else
        assert!(judge(body, &Want { float_address: "bc1pw508d6qejxtdg4y5r3zarvary0c5xw7kw508d6qejxtdg4y5r3zarvary0c5xw7kt5nd6y", ..want(&[]) })
            .unwrap_err().contains("rule 8"));
        // another EVM address of ours: the calldata names someone else
        assert!(judge(body, &Want { our_evm: [9; 20], ..want(&[]) }).unwrap_err().contains("rule 5"));
        // a stale hand-in, and a deadline that has come too close
        assert!(judge(body, &Want { handed_in_at: NOW - QUOTE_TTL_SECS - 1, ..want(&[]) }).unwrap_err().contains("stale"));
        assert!(judge(body, &Want { now: 1_791_046_471 - 3_600, handed_in_at: 1_791_046_471 - 3_600, ..want(&[]) }).unwrap_err().contains("rule 11"));
        // replay
        let used = [id_of(Q26)];
        assert!(judge(body, &want(&used)).unwrap_err().contains("rule 12"));
        // oversize and garbage
        assert!(judge(&vec![b' '; MAX_QUOTE_BYTES + 1], &want(&[])).unwrap_err().contains("bytes"));
        assert!(judge(b"<html>", &want(&[])).unwrap_err().contains("not JSON"));
    }

    /// Conversion review C-07: a deadline over a uint32 is refused, never truncated (`2^64 + t` used to pass as `t`).
    #[test]
    fn c07_a_deadline_over_a_uint32_is_refused_not_truncated() {
        let wrap = |t: u64| ((1u128 << 64) + t as u128).to_string();
        for f in [
            Box::new(|v: &mut Value| v["protocol"]["v2"]["orderData"]["output"]["deadline"] = wrap(NOW + 7 * 86_400).into()) as Box<dyn Fn(&mut Value)>,
            Box::new(|v: &mut Value| v["protocol"]["v2"]["orderData"]["inputs"][0]["refunds"][0]["deadline"] = wrap(NOW + 7 * 86_400).into()),
        ] {
            let mut v: Value = serde_json::from_str(Q26).unwrap();
            f(&mut v);
            let id = hex::encode(order_id(&v["protocol"]["v2"]["orderData"]).unwrap());
            v["protocol"]["v2"]["orderId"] = format!("0x{id}").into();
            let d = v["steps"][0]["items"][0]["data"]["data"].as_str().unwrap().to_string();
            v["steps"][0]["items"][0]["data"]["data"] = format!("{}{id}", &d[..2 + 72]).into();
            let e = judge(&serde_json::to_vec(&v).unwrap(), &want(&[])).unwrap_err();
            assert!(e.contains("rule 11") && e.contains("uint32"), "{e}");
        }
        assert_eq!(deadline_of(&serde_json::json!({"d": u32::MAX.to_string()}), "d", "x").unwrap(), u32::MAX as u64);
    }

    /// Conversion review C-05: new confirmed float outputs, summed, are an arrival at half the minimum; old and unconfirmed ones never count.
    #[test]
    fn c05_split_outputs_sum_to_an_arrival_at_half_the_minimum() {
        let o = |n: u8| Outpoint { txid: [n; 32], vout: 0 };
        let u = |n: u8, v: u64, c: bool| crate::custody::Utxo { outpoint: o(n), value: v, confirmed: c };
        let utxos = [u(1, 9_000_000, true), u(2, 300_000, true), u(3, 250_000, true), u(4, 900_000, false)];
        let (a, outs) = arrival_summed(&utxos, &[o(1)], 1_000_000).unwrap();
        assert_eq!((a.outpoint, a.sats, outs.len()), (o(2), 550_000, 2));
        assert!(arrival_summed(&utxos, &[o(1), o(2)], 1_000_000).is_none(), "250,000 is under half");
    }

    #[test]
    fn money_helpers() {
        // 0.16 ETH at 3,164,000 sat/ETH = 506,240 sats; less 3 % = 491,053 (rounded up)
        assert_eq!(min_sats_floor(160_000_000_000_000_000, 3_164_000, 300).unwrap(), 491_053);
        assert!(min_sats_floor(1, 0, 300).is_err());
        assert_eq!(convertible(GAS_KEEP_WEI), None);
        assert_eq!(convertible(GAS_KEEP_WEI + 5), Some(5));
        assert!(refunded(1_050, 1_000, 100));
        assert!(!refunded(1_049, 1_000, 100));
        // the frog's share of the advance, rounded up, never over it
        assert_eq!(cost_of(1_000, 50, 100).unwrap(), 500);
        assert_eq!(cost_of(1_000, 1, 3).unwrap(), 334);
        assert_eq!(cost_of(1_000, 100, 100).unwrap(), 1_000);
        assert!(cost_of(1_000, 101, 100).is_err());
        let o = |n: u8| Outpoint { txid: [n; 32], vout: 0 };
        let u = |n: u8, v: u64, c: bool| crate::custody::Utxo { outpoint: o(n), value: v, confirmed: c };
        let utxos = [u(1, 600_000, true), u(2, 495_542, true), u(3, 700_000, false), u(4, 100, true)];
        assert_eq!(arrival_among(&utxos, &[o(1)], 495_542), Some(Arrival { outpoint: o(2), sats: 495_542 }));
        assert_eq!(arrival_among(&utxos, &[o(1), o(2)], 495_542), None, "unconfirmed and small outputs are not arrivals");
        assert!(FrogFunding::default().validate().is_ok());
        assert!(FrogFunding { max_fee_bps: 501, float_first: false }.validate().is_err());
        assert!(FrogFunding { max_fee_bps: 49, float_first: false }.validate().is_err());
    }

    #[test]
    fn addresses_encode_as_relays_sdk_does() {
        assert_eq!(encode_address(BTC_NATIVE, "bitcoin").unwrap(), [vec![0u8], vec![0u8; 19]].concat(), "the SDK's 19-byte id");
        let e = encode_address(FLOAT, "bitcoin").unwrap();
        assert_eq!((e[0], e.len()), (1, 33));
        assert!(encode_address("1BoatSLRHtKNngkdXEeobR76b53LETtpyT", "bitcoin").is_err(), "legacy addresses are refused, not guessed");
        assert!(encode_address(FLOAT, "solana").is_err());
        assert_eq!(encode_address("0x000000000000000000000000000000000000bEEF", "robinhood").unwrap().len(), 20);
    }
}
