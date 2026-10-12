//! The last check before anything leaves custody: the signed bytes themselves must do one of the few things custody is for, and
//! nothing else (docs/v5/custody-hardening.md, H4; docs/v5/custody-threat-model.md, T8, T10, T16).
//!
//! **Wired** (docs/v5/custody-hardening.md, H4): `redeem::owed` and `frogflow::give_back` run [`verify_btc`] on the finalized
//! bytes before saving them, the ticket read at a finalized 4663 block with its `Redeemed` log (H9). Still pure: no `crate::`
//! import; `tests/hardening.rs` also runs it on transactions built by the real `custody::build`.
//!
//! `custody.rs` builds a delivery correctly and `delivery::decide` says when it may go. This module does not trust either. It
//! re-reads the **finalized signed bytes** — what would actually reach Bitcoin, or the chain — and checks them against a
//! [`Purpose`] stated separately, from the ticket or the deposit record. It is a second, independent reading of the same act,
//! against a short list of what custody may do:
//!
//! - **Delivery**: the item's output goes, whole, to the destination **the ticket names**, and the ticket is `Resolved`, read at a
//!   finalized block, with its `Redeemed` event naming the same destination;
//! - **DepositReturn**: a refused deposit goes, whole, back to the script that sent it;
//! - **ConflictOut**: an input whose signed spend leaked (broadcast, evicted, then refused) is spent back to custody, so the
//!   leaked bytes can never be mined (H8);
//! - for EVM custody: an ERC-721 `transferFrom(custodian, ticket destination, token id)` and nothing else, or a nonce burn.
//!
//! What it catches: a builder bug, a new code path that signs with the custody key for a reason nobody listed, a destination
//! swapped between the ticket read and the signing, change sent anywhere but custody, a fee that eats the item, an approval.
//! What it cannot catch: a ticket that providers forged consistently at a finalized block. That is H3's rate limit and H10's
//! independent providers.

use serde::{Deserialize, Serialize};

/// A fee above this is refused: it is the item's value leaking to miners, or a builder bug (`custody::MAX_DELIVERY_FEE_SATS`).
pub const MAX_FEE_SATS: u64 = 25_000;
/// A conflict-out may pay up to this many times the policy's `max_fee_sats` (audit R2, N-2): it pays nobody but custody, and it
/// must be able to outbid a delivery bumped to its cap plus a child the redeemer hung on it. Deliveries and returns keep the cap.
pub const CONFLICT_FEE_FACTOR: u64 = 4;

// ---- a Bitcoin transaction, read from its raw bytes -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxIn {
    /// Internal byte order (as serialised), not an explorer's display order.
    pub txid: [u8; 32],
    pub vout: u32,
    pub script_sig: Vec<u8>,
    pub sequence: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxOut {
    pub value: u64,
    pub script: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tx {
    pub version: u32,
    pub inputs: Vec<TxIn>,
    pub outputs: Vec<TxOut>,
    pub locktime: u32,
    pub segwit: bool,
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.i.checked_add(n).filter(|e| *e <= self.b.len()).ok_or("the transaction ends early")?;
        let s = &self.b[self.i..end];
        self.i = end;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8")))
    }
    fn varint(&mut self) -> Result<u64, String> {
        let n = self.take(1)?[0];
        let v = match n {
            0xfd => u16::from_le_bytes(self.take(2)?.try_into().expect("2")) as u64,
            0xfe => self.u32()? as u64,
            0xff => self.u64()?,
            x => x as u64,
        };
        // minimal encoding only: two spellings of one transaction are two transactions to a reader like this one
        let minimal = match n {
            0xfd => v >= 0xfd,
            0xfe => v > 0xffff,
            0xff => v > 0xffff_ffff,
            _ => true,
        };
        if !minimal {
            return Err("a non-minimal length".into());
        }
        Ok(v)
    }
    fn bytes(&mut self) -> Result<Vec<u8>, String> {
        let n = self.varint()?;
        let n = usize::try_from(n).map_err(|_| "a length that does not fit")?;
        Ok(self.take(n)?.to_vec())
    }
}

/// Parse a raw transaction (BIP144 segwit or legacy). Refuses trailing bytes, a segwit marker with no witness, and counts larger
/// than the bytes could hold.
pub fn parse_tx(raw: &[u8]) -> Result<Tx, String> {
    let mut r = Reader { b: raw, i: 0 };
    let version = r.u32()?;
    let segwit = raw.len() > 6 && raw[4] == 0x00 && raw[5] == 0x01;
    if segwit {
        r.take(2)?;
    }
    let n_in = r.varint()?;
    if n_in == 0 || n_in > (raw.len() / 41) as u64 {
        return Err("an impossible number of inputs".into());
    }
    let mut inputs = Vec::new();
    for _ in 0..n_in {
        let txid: [u8; 32] = r.take(32)?.try_into().expect("32");
        let vout = r.u32()?;
        let script_sig = r.bytes()?;
        let sequence = r.u32()?;
        inputs.push(TxIn { txid, vout, script_sig, sequence });
    }
    let n_out = r.varint()?;
    if n_out == 0 || n_out > (raw.len() / 9) as u64 {
        return Err("an impossible number of outputs".into());
    }
    let mut outputs = Vec::new();
    for _ in 0..n_out {
        let value = r.u64()?;
        let script = r.bytes()?;
        outputs.push(TxOut { value, script });
    }
    if segwit {
        let mut any = false;
        for _ in 0..inputs.len() {
            let items = r.varint()?;
            for _ in 0..items {
                any = true;
                r.bytes()?;
            }
        }
        if !any {
            return Err("a segwit marker with no witness".into());
        }
    }
    let locktime = r.u32()?;
    if r.i != raw.len() {
        return Err("bytes after the transaction".into());
    }
    Ok(Tx { version, inputs, outputs, locktime, segwit })
}

// ---- what custody may do on Bitcoin -----------------------------------------------------------------------------------------------

/// An output being spent, as the canister verified it (explorers agreeing): where it is, what it holds, whose it is.
#[derive(candid::CandidType, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Prevout {
    /// Internal byte order.
    pub txid: [u8; 32],
    pub vout: u32,
    pub value: u64,
    pub script: Vec<u8>,
}

/// A ticket as read from the wrapper, with how it was read.
#[derive(candid::CandidType, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TicketFacts {
    pub ticket: u64,
    /// `ticket(id).stage == Resolved` at the fast pin, AND the ticket's `Resolved` event is at or below the finalized block
    /// (audit D, H-1: `redeem::facts_of`).
    pub resolved: bool,
    /// The facts rest on reads that a reorganisation cannot change: the ticket's `Redeemed` and `Resolved` events at or below the
    /// chain's finalized block, every provider agreeing (H9, through events: the Phase-1 providers cannot serve STATE that deep).
    pub finalized: bool,
    /// The ticket's destination as a scriptPubKey (decoded from its address for the installed network).
    pub destination_script: Vec<u8>,
    /// The destination in the ticket's final `Redeemed` event, decoded the same way; `None` if not read.
    pub event_destination_script: Option<Vec<u8>>,
    /// The ticket names this redemption's acquisition, twice: in its state at the fast pin and in its final `Resolved` event
    /// (two separate reads that must agree). What it does NOT tie (audit D, I-1): that acquisition to the outpoint spent. The
    /// wiring locates the acquisition's inscription (the vault's `acquisition(id).assetRef`, or for a deposit frog the leaf this
    /// canister minted) through two ordinals providers, and `custody::frog_at` checks that output carries exactly that inscription;
    /// the verifier trusts that step and checks the transaction against the facts, not the other way round.
    pub names_this_item: bool,
}

#[derive(candid::CandidType, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    Delivery { ticket: TicketFacts, item: Prevout },
    DepositReturn { deposit: Prevout, sender_script: Vec<u8> },
    ConflictOut { spend: Prevout },
}

/// The rules the wiring configures. `require_finalized` and `require_event` exist so the verifier can be wired before H9's
/// finalized reads are; `docs/v5/launch-gates.md` requires both true before value enters custody.
#[derive(candid::CandidType, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BtcPolicy {
    pub custody_script: Vec<u8>,
    pub max_fee_sats: u64,
    pub require_finalized: bool,
    pub require_event: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Unparsable(String),
    /// The prevouts given do not describe this transaction's inputs, in order.
    PrevoutsDoNotMatch,
    /// An input is neither the purpose's own output nor a custody output.
    ForeignInput(usize),
    /// Input 0 is not the item / deposit / leaked input the purpose names.
    WrongFirstInput,
    TicketNotResolved,
    TicketNotFinalized,
    /// The ticket names a different item.
    WrongItem,
    /// The destination in the ticket and the one in its event disagree, or the event was not read.
    EventDisagrees,
    /// Output 0 does not pay the destination, or not exactly the item's value.
    WrongDestination,
    WrongValue { expected: u64, got: u64 },
    /// A delivery or return to custody itself moves nothing and would take an item off the books.
    DestinationIsCustody,
    /// An output other than output 0 pays someone other than custody.
    ForeignOutput(usize),
    TooManyOutputs(usize),
    FeeTooHigh { fee: u64, max: u64 },
    /// Outputs worth more than the inputs: arithmetic that cannot be a valid transaction.
    Overspend,
    /// A conflict-out that does not spend the leaked input.
    ConflictMissesInput,
    /// Not a P2TR script where one is required.
    NotTaproot,
}

fn is_p2tr(s: &[u8]) -> bool {
    s.len() == 34 && s[0] == 0x51 && s[1] == 0x20
}

fn same_outpoint(i: &TxIn, p: &Prevout) -> bool {
    i.txid == p.txid && i.vout == p.vout
}

/// What a checked transaction does, for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub fee: u64,
    pub to: Vec<u8>,
    pub value: u64,
}

/// Judge the signed bytes of one custody spend against its purpose. `prevouts` must describe every input, in order.
pub fn verify_btc(raw: &[u8], prevouts: &[Prevout], purpose: &Purpose, p: &BtcPolicy) -> Result<Checked, Refusal> {
    let tx = parse_tx(raw).map_err(Refusal::Unparsable)?;
    if !is_p2tr(&p.custody_script) {
        return Err(Refusal::NotTaproot);
    }
    if prevouts.len() != tx.inputs.len() || !tx.inputs.iter().zip(prevouts).all(|(i, po)| same_outpoint(i, po)) {
        return Err(Refusal::PrevoutsDoNotMatch);
    }
    let first = match purpose {
        Purpose::Delivery { item, .. } => item,
        Purpose::DepositReturn { deposit, .. } => deposit,
        Purpose::ConflictOut { spend } => spend,
    };
    // every input: the purpose's own output (for a delivery or a return, only as input 0), or custody's
    for (k, po) in prevouts.iter().enumerate() {
        let own = po == first;
        let allowed = match purpose {
            Purpose::ConflictOut { .. } => own || po.script == p.custody_script,
            _ => (k == 0 && own) || (k > 0 && !own && po.script == p.custody_script),
        };
        if !allowed {
            return Err(if k == 0 && !matches!(purpose, Purpose::ConflictOut { .. }) { Refusal::WrongFirstInput } else { Refusal::ForeignInput(k) });
        }
    }
    let total_in = prevouts.iter().try_fold(0u64, |s, po| s.checked_add(po.value)).ok_or(Refusal::Overspend)?;
    let total_out = tx.outputs.iter().try_fold(0u64, |s, o| s.checked_add(o.value)).ok_or(Refusal::Overspend)?;
    let fee = total_in.checked_sub(total_out).ok_or(Refusal::Overspend)?;
    let max = match purpose {
        Purpose::ConflictOut { .. } => p.max_fee_sats.saturating_mul(CONFLICT_FEE_FACTOR),
        _ => p.max_fee_sats,
    };
    if fee > max {
        return Err(Refusal::FeeTooHigh { fee, max });
    }

    match purpose {
        Purpose::ConflictOut { spend } => {
            if !prevouts.contains(spend) {
                return Err(Refusal::ConflictMissesInput);
            }
            if let Some(k) = tx.outputs.iter().position(|o| o.script != p.custody_script) {
                return Err(Refusal::ForeignOutput(k));
            }
            Ok(Checked { fee, to: p.custody_script.clone(), value: total_out })
        }
        Purpose::Delivery { ticket, item } => {
            if !ticket.resolved {
                return Err(Refusal::TicketNotResolved);
            }
            if p.require_finalized && !ticket.finalized {
                return Err(Refusal::TicketNotFinalized);
            }
            if !ticket.names_this_item {
                return Err(Refusal::WrongItem);
            }
            match &ticket.event_destination_script {
                Some(e) if *e != ticket.destination_script => return Err(Refusal::EventDisagrees),
                None if p.require_event => return Err(Refusal::EventDisagrees),
                _ => {}
            }
            whole_to(&tx, item, &ticket.destination_script, p).map(|value| Checked { fee, to: ticket.destination_script.clone(), value })
        }
        Purpose::DepositReturn { deposit, sender_script } => {
            whole_to(&tx, deposit, sender_script, p).map(|value| Checked { fee, to: sender_script.clone(), value })
        }
    }
}

/// Output 0 pays `dest` exactly the item's value; at most one more output, and it pays custody.
fn whole_to(tx: &Tx, item: &Prevout, dest: &[u8], p: &BtcPolicy) -> Result<u64, Refusal> {
    if !is_p2tr(dest) {
        return Err(Refusal::NotTaproot);
    }
    if dest == p.custody_script.as_slice() {
        return Err(Refusal::DestinationIsCustody);
    }
    if tx.outputs.len() > 2 {
        return Err(Refusal::TooManyOutputs(tx.outputs.len()));
    }
    let o0 = &tx.outputs[0];
    if o0.script != dest {
        return Err(Refusal::WrongDestination);
    }
    if o0.value != item.value {
        return Err(Refusal::WrongValue { expected: item.value, got: o0.value });
    }
    if let Some(o1) = tx.outputs.get(1) {
        if o1.script != p.custody_script {
            return Err(Refusal::ForeignOutput(1));
        }
    }
    Ok(o0.value)
}

// ---- what custody may do on an EVM chain ------------------------------------------------------------------------------------------

/// `transferFrom(address,address,uint256)`.
pub const TRANSFER_FROM: [u8; 4] = [0x23, 0xb8, 0x72, 0xdd];
/// Named so the refusal can say what it refused: `safeTransferFrom(address,address,uint256)`, `…,bytes)`, `approve`,
/// `setApprovalForAll`.
pub const SAFE_TRANSFER_FROM: [u8; 4] = [0x42, 0x84, 0x2e, 0x0e];
pub const SAFE_TRANSFER_FROM_DATA: [u8; 4] = [0xb8, 0x8d, 0x4f, 0xde];
pub const APPROVE: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];
pub const SET_APPROVAL_FOR_ALL: [u8; 4] = [0xa2, 0x2c, 0xb4, 0x65];

/// One EVM call custody would sign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmCall {
    pub chain_id: u64,
    pub to: [u8; 20],
    pub value: u128,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvmPurpose {
    /// Deliver a resolved ticket's NFT. `destination` is the ticket's, read at a finalized 4663 block.
    NftDelivery {
        chain_id: u64,
        collection: [u8; 20],
        custodian: [u8; 20],
        destination: [u8; 20],
        token_id: [u8; 32],
        resolved: bool,
        finalized: bool,
    },
    /// Burn a nonce whose signed transaction leaked: 0 value to ourselves, no data.
    NonceBurn { chain_id: u64, custodian: [u8; 20] },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvmRefusal {
    WrongChain,
    WrongContract,
    ValueSent(u128),
    /// `safeTransferFrom` lets a contract destination refuse delivery and re-roll after the refund (docs/v4/nft-wrapper.md §5.3).
    SafeTransfer,
    /// Custody never approves anyone for anything.
    Approval,
    UnknownCall([u8; 4]),
    /// The call's from / to / token id differ from the ticket's.
    WrongArguments,
    BadDestination,
    TicketNotResolved,
    TicketNotFinalized,
    NotEmpty,
}

fn word_addr(a: &[u8; 20]) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a);
    w
}

/// `transferFrom(from, to, tokenId)` calldata, exactly.
pub fn transfer_from(from: &[u8; 20], to: &[u8; 20], token_id: &[u8; 32]) -> Vec<u8> {
    [TRANSFER_FROM.as_slice(), &word_addr(from), &word_addr(to), token_id].concat()
}

pub fn verify_evm(c: &EvmCall, purpose: &EvmPurpose, require_finalized: bool) -> Result<(), EvmRefusal> {
    if let Some(sel) = c.data.get(..4) {
        let sel: [u8; 4] = sel.try_into().expect("4");
        if sel == SAFE_TRANSFER_FROM || sel == SAFE_TRANSFER_FROM_DATA {
            return Err(EvmRefusal::SafeTransfer);
        }
        if sel == APPROVE || sel == SET_APPROVAL_FOR_ALL {
            return Err(EvmRefusal::Approval);
        }
    }
    match purpose {
        EvmPurpose::NonceBurn { chain_id, custodian } => {
            if c.chain_id != *chain_id {
                return Err(EvmRefusal::WrongChain);
            }
            if c.to != *custodian {
                return Err(EvmRefusal::WrongContract);
            }
            if c.value != 0 {
                return Err(EvmRefusal::ValueSent(c.value));
            }
            if !c.data.is_empty() {
                return Err(EvmRefusal::NotEmpty);
            }
            Ok(())
        }
        EvmPurpose::NftDelivery { chain_id, collection, custodian, destination, token_id, resolved, finalized } => {
            if c.chain_id != *chain_id {
                return Err(EvmRefusal::WrongChain);
            }
            if !resolved {
                return Err(EvmRefusal::TicketNotResolved);
            }
            if require_finalized && !finalized {
                return Err(EvmRefusal::TicketNotFinalized);
            }
            if c.to != *collection {
                return Err(EvmRefusal::WrongContract);
            }
            if c.value != 0 {
                return Err(EvmRefusal::ValueSent(c.value));
            }
            if *destination == [0u8; 20] || destination == custodian {
                return Err(EvmRefusal::BadDestination);
            }
            match c.data.get(..4) {
                Some(s) if s == TRANSFER_FROM => {}
                Some(s) => return Err(EvmRefusal::UnknownCall(s.try_into().expect("4"))),
                None => return Err(EvmRefusal::UnknownCall([0; 4])),
            }
            if c.data != transfer_from(custodian, destination, token_id) {
                return Err(EvmRefusal::WrongArguments);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p2tr(b: u8) -> Vec<u8> {
        [vec![0x51, 0x20], vec![b; 32]].concat()
    }

    fn ser(tx: &Tx, witness: bool) -> Vec<u8> {
        let mut o = tx.version.to_le_bytes().to_vec();
        if witness {
            o.extend([0x00, 0x01]);
        }
        o.push(tx.inputs.len() as u8);
        for i in &tx.inputs {
            o.extend(i.txid);
            o.extend(i.vout.to_le_bytes());
            o.push(i.script_sig.len() as u8);
            o.extend(&i.script_sig);
            o.extend(i.sequence.to_le_bytes());
        }
        o.push(tx.outputs.len() as u8);
        for x in &tx.outputs {
            o.extend(x.value.to_le_bytes());
            o.push(x.script.len() as u8);
            o.extend(&x.script);
        }
        if witness {
            for _ in &tx.inputs {
                o.extend([0x01, 0x40]);
                o.extend([0xaa; 64]);
            }
        }
        o.extend(tx.locktime.to_le_bytes());
        o
    }

    const CUSTODY: u8 = 0xc0;
    const REDEEMER: u8 = 0xde;

    fn item() -> Prevout {
        Prevout { txid: [1; 32], vout: 0, value: 546, script: p2tr(CUSTODY) }
    }
    fn fee_in() -> Prevout {
        Prevout { txid: [2; 32], vout: 3, value: 20_000, script: p2tr(CUSTODY) }
    }
    fn policy() -> BtcPolicy {
        BtcPolicy { custody_script: p2tr(CUSTODY), max_fee_sats: MAX_FEE_SATS, require_finalized: true, require_event: true }
    }
    fn ticket() -> TicketFacts {
        TicketFacts { ticket: 7, resolved: true, finalized: true, destination_script: p2tr(REDEEMER),
            event_destination_script: Some(p2tr(REDEEMER)), names_this_item: true }
    }
    fn delivery(outs: Vec<TxOut>) -> (Vec<u8>, Vec<Prevout>) {
        let (a, b) = (item(), fee_in());
        let tx = Tx {
            version: 2, segwit: true, locktime: 0,
            inputs: vec![TxIn { txid: a.txid, vout: a.vout, script_sig: vec![], sequence: 0xffff_fffd },
                TxIn { txid: b.txid, vout: b.vout, script_sig: vec![], sequence: 0xffff_fffd }],
            outputs: outs,
        };
        (ser(&tx, true), vec![a, b])
    }
    fn honest() -> (Vec<u8>, Vec<Prevout>) {
        delivery(vec![TxOut { value: 546, script: p2tr(REDEEMER) }, TxOut { value: 20_000 - 1_000, script: p2tr(CUSTODY) }])
    }

    #[test]
    fn the_parser_reads_what_it_wrote_and_refuses_the_rest() {
        let (raw, _) = honest();
        let tx = parse_tx(&raw).unwrap();
        assert_eq!((tx.inputs.len(), tx.outputs.len(), tx.segwit, tx.outputs[0].value), (2, 2, true, 546));
        assert!(parse_tx(&raw[..raw.len() - 1]).is_err());
        assert!(parse_tx(&[raw.clone(), vec![0]].concat()).unwrap_err().contains("after"));
        let mut legacy = Tx { version: 2, inputs: vec![], outputs: vec![], locktime: 0, segwit: false };
        legacy.inputs.push(TxIn { txid: [9; 32], vout: 1, script_sig: vec![0x51], sequence: 0 });
        legacy.outputs.push(TxOut { value: 5, script: p2tr(1) });
        assert_eq!(parse_tx(&ser(&legacy, false)).unwrap(), legacy);
        // a non-minimal varint (0xfd 0x02 0x00 for 2 inputs) is a second spelling: refused
        let mut nm = raw.clone();
        nm.splice(6..7, [0xfd, 0x02, 0x00]);
        assert!(parse_tx(&nm).unwrap_err().contains("non-minimal"));
        assert!(parse_tx(&[]).is_err());
    }

    #[test]
    fn an_honest_delivery_passes() {
        let (raw, pre) = honest();
        let c = verify_btc(&raw, &pre, &Purpose::Delivery { ticket: ticket(), item: item() }, &policy()).unwrap();
        assert_eq!((c.fee, c.value, c.to), (1_000, 546, p2tr(REDEEMER)));
        // with no change output too
        let (raw, pre) = delivery(vec![TxOut { value: 546, script: p2tr(REDEEMER) }]);
        assert!(verify_btc(&raw, &pre, &Purpose::Delivery { ticket: ticket(), item: item() }, &BtcPolicy { max_fee_sats: 20_000, ..policy() }).is_ok());
    }

    #[test]
    fn a_delivery_to_anyone_but_the_tickets_destination_is_refused() {
        let d = Purpose::Delivery { ticket: ticket(), item: item() };
        let (raw, pre) = delivery(vec![TxOut { value: 546, script: p2tr(0xee) }, TxOut { value: 19_000, script: p2tr(CUSTODY) }]);
        assert_eq!(verify_btc(&raw, &pre, &d, &policy()), Err(Refusal::WrongDestination));
        // change to a stranger
        let (raw, pre) = delivery(vec![TxOut { value: 546, script: p2tr(REDEEMER) }, TxOut { value: 19_000, script: p2tr(0xee) }]);
        assert_eq!(verify_btc(&raw, &pre, &d, &policy()), Err(Refusal::ForeignOutput(1)));
        // a third output
        let (raw, pre) = delivery(vec![TxOut { value: 546, script: p2tr(REDEEMER) }, TxOut { value: 9_000, script: p2tr(CUSTODY) },
            TxOut { value: 9_000, script: p2tr(CUSTODY) }]);
        assert_eq!(verify_btc(&raw, &pre, &d, &policy()), Err(Refusal::TooManyOutputs(3)));
        // less than the item's whole output: the inscribed sat could land elsewhere
        let (raw, pre) = delivery(vec![TxOut { value: 545, script: p2tr(REDEEMER) }, TxOut { value: 19_001, script: p2tr(CUSTODY) }]);
        assert_eq!(verify_btc(&raw, &pre, &d, &policy()), Err(Refusal::WrongValue { expected: 546, got: 545 }));
        // a ticket naming custody itself
        let mut t = ticket();
        t.destination_script = p2tr(CUSTODY);
        t.event_destination_script = Some(p2tr(CUSTODY));
        let (raw, pre) = delivery(vec![TxOut { value: 546, script: p2tr(CUSTODY) }]);
        assert_eq!(verify_btc(&raw, &pre, &Purpose::Delivery { ticket: t, item: item() }, &BtcPolicy { max_fee_sats: 20_000, ..policy() }),
            Err(Refusal::DestinationIsCustody));
    }

    #[test]
    fn the_ticket_must_be_resolved_finalized_about_this_item_and_agree_with_its_event() {
        let (raw, pre) = honest();
        let go = |t: TicketFacts, p: &BtcPolicy| verify_btc(&raw, &pre, &Purpose::Delivery { ticket: t, item: item() }, p);
        assert_eq!(go(TicketFacts { resolved: false, ..ticket() }, &policy()), Err(Refusal::TicketNotResolved));
        assert_eq!(go(TicketFacts { finalized: false, ..ticket() }, &policy()), Err(Refusal::TicketNotFinalized));
        assert!(go(TicketFacts { finalized: false, ..ticket() }, &BtcPolicy { require_finalized: false, ..policy() }).is_ok(),
            "before H9 is wired the flag lets it through; the launch gate requires it on");
        assert_eq!(go(TicketFacts { names_this_item: false, ..ticket() }, &policy()), Err(Refusal::WrongItem));
        assert_eq!(go(TicketFacts { event_destination_script: Some(p2tr(0xee)), ..ticket() }, &policy()), Err(Refusal::EventDisagrees));
        assert_eq!(go(TicketFacts { event_destination_script: None, ..ticket() }, &policy()), Err(Refusal::EventDisagrees));
        assert!(go(TicketFacts { event_destination_script: None, ..ticket() }, &BtcPolicy { require_event: false, ..policy() }).is_ok());
    }

    #[test]
    fn inputs_are_the_item_first_then_only_custody() {
        let d = Purpose::Delivery { ticket: ticket(), item: item() };
        let (raw, mut pre) = honest();
        // the explorers' account of the fee input says it is not custody's
        pre[1].script = p2tr(0x55);
        assert_eq!(verify_btc(&raw, &pre, &d, &policy()), Err(Refusal::ForeignInput(1)));
        // prevouts that do not describe the transaction
        let (raw, mut pre) = honest();
        pre.swap(0, 1);
        assert_eq!(verify_btc(&raw, &pre, &d, &policy()), Err(Refusal::PrevoutsDoNotMatch));
        let (raw, pre) = honest();
        assert_eq!(verify_btc(&raw, &pre[..1], &d, &policy()), Err(Refusal::PrevoutsDoNotMatch));
        // a delivery whose first input is some other custody output (another frog)
        let other = Prevout { txid: [1; 32], vout: 0, value: 546, script: p2tr(CUSTODY) };
        let wrong_item = Prevout { txid: [7; 32], ..other.clone() };
        assert_eq!(verify_btc(&raw, &pre, &Purpose::Delivery { ticket: ticket(), item: wrong_item }, &policy()), Err(Refusal::WrongFirstInput));
    }

    #[test]
    fn the_fee_is_bounded_and_arithmetic_is_checked() {
        let d = Purpose::Delivery { ticket: ticket(), item: item() };
        let (raw, pre) = delivery(vec![TxOut { value: 546, script: p2tr(REDEEMER) }]);
        // 20,546 in, 546 out: 20,000 fee, under 25,000
        assert!(verify_btc(&raw, &pre, &d, &policy()).is_ok());
        assert_eq!(verify_btc(&raw, &pre, &d, &BtcPolicy { max_fee_sats: 19_999, ..policy() }), Err(Refusal::FeeTooHigh { fee: 20_000, max: 19_999 }));
        let (raw, pre) = delivery(vec![TxOut { value: 546, script: p2tr(REDEEMER) }, TxOut { value: 30_000, script: p2tr(CUSTODY) }]);
        assert_eq!(verify_btc(&raw, &pre, &d, &policy()), Err(Refusal::Overspend));
    }

    #[test]
    fn a_deposit_goes_back_only_to_its_sender() {
        let dep = Prevout { txid: [4; 32], vout: 1, value: 330, script: p2tr(CUSTODY) };
        let tx = Tx { version: 2, segwit: true, locktime: 0,
            inputs: vec![TxIn { txid: dep.txid, vout: 1, script_sig: vec![], sequence: 0xffff_fffd },
                TxIn { txid: fee_in().txid, vout: fee_in().vout, script_sig: vec![], sequence: 0xffff_fffd }],
            outputs: vec![TxOut { value: 330, script: p2tr(0x5e) }, TxOut { value: 19_000, script: p2tr(CUSTODY) }] };
        let pre = vec![dep.clone(), fee_in()];
        let raw = ser(&tx, true);
        assert!(verify_btc(&raw, &pre, &Purpose::DepositReturn { deposit: dep.clone(), sender_script: p2tr(0x5e) }, &policy()).is_ok());
        assert_eq!(verify_btc(&raw, &pre, &Purpose::DepositReturn { deposit: dep, sender_script: p2tr(0x5f) }, &policy()),
            Err(Refusal::WrongDestination));
    }

    #[test]
    fn a_conflict_out_spends_the_leaked_input_and_pays_only_custody() {
        let leaked = fee_in();
        let tx = |outs: Vec<TxOut>| Tx { version: 2, segwit: true, locktime: 0,
            inputs: vec![TxIn { txid: leaked.txid, vout: leaked.vout, script_sig: vec![], sequence: 0xffff_fffd }], outputs: outs };
        let pre = vec![leaked.clone()];
        let ok = ser(&tx(vec![TxOut { value: 17_000, script: p2tr(CUSTODY) }]), true);
        assert!(verify_btc(&ok, &pre, &Purpose::ConflictOut { spend: leaked.clone() }, &policy()).is_ok());
        let out = ser(&tx(vec![TxOut { value: 17_000, script: p2tr(0xee) }]), true);
        assert_eq!(verify_btc(&out, &pre, &Purpose::ConflictOut { spend: leaked.clone() }, &policy()), Err(Refusal::ForeignOutput(0)));
        // a "conflict" that spends some other custody output instead of the leaked one
        let other = Prevout { txid: [8; 32], ..leaked.clone() };
        let t2 = Tx { inputs: vec![TxIn { txid: other.txid, vout: other.vout, script_sig: vec![], sequence: 0xffff_fffd }], ..tx(vec![TxOut { value: 17_000, script: p2tr(CUSTODY) }]) };
        assert_eq!(verify_btc(&ser(&t2, true), &[other], &Purpose::ConflictOut { spend: leaked }, &policy()), Err(Refusal::ConflictMissesInput));
    }

    // ---- EVM ----

    const COLL: [u8; 20] = [0xbc; 20];
    const US: [u8; 20] = [0xcb; 20];
    const HOLDER: [u8; 20] = [0x70; 20];

    fn nft() -> EvmPurpose {
        EvmPurpose::NftDelivery { chain_id: 1, collection: COLL, custodian: US, destination: HOLDER, token_id: [0x26; 32], resolved: true, finalized: true }
    }
    fn call(data: Vec<u8>) -> EvmCall {
        EvmCall { chain_id: 1, to: COLL, value: 0, data }
    }

    #[test]
    fn an_nft_leaves_only_by_transfer_from_to_the_tickets_destination() {
        assert!(verify_evm(&call(transfer_from(&US, &HOLDER, &[0x26; 32])), &nft(), true).is_ok());
        assert_eq!(transfer_from(&US, &HOLDER, &[0x26; 32]).len(), 100);
        assert_eq!(verify_evm(&call(transfer_from(&US, &[0x66; 20], &[0x26; 32])), &nft(), true), Err(EvmRefusal::WrongArguments));
        assert_eq!(verify_evm(&call(transfer_from(&US, &HOLDER, &[0x27; 32])), &nft(), true), Err(EvmRefusal::WrongArguments));
        let mut extra = transfer_from(&US, &HOLDER, &[0x26; 32]);
        extra.push(0);
        assert_eq!(verify_evm(&call(extra), &nft(), true), Err(EvmRefusal::WrongArguments), "trailing bytes");
        let safe = [SAFE_TRANSFER_FROM.to_vec(), transfer_from(&US, &HOLDER, &[0x26; 32])[4..].to_vec()].concat();
        assert_eq!(verify_evm(&call(safe), &nft(), true), Err(EvmRefusal::SafeTransfer));
        assert_eq!(verify_evm(&call(SET_APPROVAL_FOR_ALL.to_vec()), &nft(), true), Err(EvmRefusal::Approval));
        assert_eq!(verify_evm(&call(APPROVE.to_vec()), &nft(), true), Err(EvmRefusal::Approval));
        assert_eq!(verify_evm(&EvmCall { to: [0x11; 20], ..call(transfer_from(&US, &HOLDER, &[0x26; 32])) }, &nft(), true), Err(EvmRefusal::WrongContract));
        assert_eq!(verify_evm(&EvmCall { value: 1, ..call(transfer_from(&US, &HOLDER, &[0x26; 32])) }, &nft(), true), Err(EvmRefusal::ValueSent(1)));
        assert_eq!(verify_evm(&EvmCall { chain_id: 8453, ..call(transfer_from(&US, &HOLDER, &[0x26; 32])) }, &nft(), true), Err(EvmRefusal::WrongChain));
        assert_eq!(verify_evm(&call(vec![]), &nft(), true), Err(EvmRefusal::UnknownCall([0; 4])));
        assert_eq!(verify_evm(&call(vec![1, 2, 3, 4]), &nft(), true), Err(EvmRefusal::UnknownCall([1, 2, 3, 4])));
    }

    #[test]
    fn the_nft_ticket_must_be_resolved_and_final_and_name_a_real_destination() {
        let data = transfer_from(&US, &HOLDER, &[0x26; 32]);
        let with = |f: &dyn Fn(&mut EvmPurpose)| { let mut p = nft(); f(&mut p); p };
        let unresolved = with(&|p| if let EvmPurpose::NftDelivery { resolved, .. } = p { *resolved = false });
        assert_eq!(verify_evm(&call(data.clone()), &unresolved, true), Err(EvmRefusal::TicketNotResolved));
        let soft = with(&|p| if let EvmPurpose::NftDelivery { finalized, .. } = p { *finalized = false });
        assert_eq!(verify_evm(&call(data.clone()), &soft, true), Err(EvmRefusal::TicketNotFinalized));
        assert!(verify_evm(&call(data), &soft, false).is_ok());
        let to_us = with(&|p| if let EvmPurpose::NftDelivery { destination, .. } = p { *destination = US });
        assert_eq!(verify_evm(&call(transfer_from(&US, &US, &[0x26; 32])), &to_us, true), Err(EvmRefusal::BadDestination));
        let to_zero = with(&|p| if let EvmPurpose::NftDelivery { destination, .. } = p { *destination = [0; 20] });
        assert_eq!(verify_evm(&call(transfer_from(&US, &[0; 20], &[0x26; 32])), &to_zero, true), Err(EvmRefusal::BadDestination));
    }

    #[test]
    fn a_nonce_burn_is_nothing_to_ourselves() {
        let burn = EvmPurpose::NonceBurn { chain_id: 1, custodian: US };
        assert!(verify_evm(&EvmCall { chain_id: 1, to: US, value: 0, data: vec![] }, &burn, true).is_ok());
        assert_eq!(verify_evm(&EvmCall { chain_id: 1, to: HOLDER, value: 0, data: vec![] }, &burn, true), Err(EvmRefusal::WrongContract));
        assert_eq!(verify_evm(&EvmCall { chain_id: 1, to: US, value: 5, data: vec![] }, &burn, true), Err(EvmRefusal::ValueSent(5)));
        assert_eq!(verify_evm(&EvmCall { chain_id: 1, to: US, value: 0, data: vec![0] }, &burn, true), Err(EvmRefusal::NotEmpty));
    }

    #[test]
    fn the_selectors_are_the_standards() {
        use sha3::{Digest, Keccak256};
        let sel = |s: &str| -> [u8; 4] { Keccak256::digest(s.as_bytes())[..4].try_into().unwrap() };
        assert_eq!(sel("transferFrom(address,address,uint256)"), TRANSFER_FROM);
        assert_eq!(sel("safeTransferFrom(address,address,uint256)"), SAFE_TRANSFER_FROM);
        assert_eq!(sel("safeTransferFrom(address,address,uint256,bytes)"), SAFE_TRANSFER_FROM_DATA);
        assert_eq!(sel("approve(address,uint256)"), APPROVE);
        assert_eq!(sel("setApprovalForAll(address,bool)"), SET_APPROVAL_FOR_ALL);
    }
}
