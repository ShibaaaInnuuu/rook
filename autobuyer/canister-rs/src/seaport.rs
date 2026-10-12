//! Seaport 1.6 basic orders: decode what a relayer hands in, refuse anything but the purchase we recorded, and build our own
//! calldata (`docs/v4/eth-nft-canister.md` §5–6).
//!
//! Pure: no network, no keys, no chain. Every rule in [`verify`] is tested against real orders captured read-only from
//! Ethereum and Base (`tests/fixtures/seaport/`), and the order hash and domain separator against the values Seaport itself
//! emitted and returned for them. The on-chain half of the checks (owner, order status, code hash, simulation) is
//! `nftnet.rs`'s; it feeds its answers in here as [`ChainFacts`].

use crate::evmtx::keccak;

/// Seaport 1.6: the same address on every chain (deterministic CREATE2), so this is not a constant of any one chain.
pub const SEAPORT_1_6: [u8; 20] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x68, 0xf1, 0x16, 0xa8, 0x94, 0x98, 0x4e, 0x2d, 0xb1,
    0x12, 0x3e, 0xb3, 0x95];

/// `fulfillBasicOrder((…))`, and the gas-efficient alias Seaport also routes to the same code. We accept either in; we send
/// only the first.
pub const FULFILL_BASIC_ORDER: [u8; 4] = [0xfb, 0x0f, 0x3e, 0xe1];
pub const FULFILL_BASIC_ORDER_EFFICIENT: [u8; 4] = [0x00, 0x00, 0x00, 0x00];
/// `getOrderStatus(bytes32)`, `getCounter(address)`, `information()`
pub const GET_ORDER_STATUS: [u8; 4] = [0x46, 0x42, 0x3a, 0xa7];
pub const GET_COUNTER: [u8; 4] = [0xf0, 0x7e, 0xc3, 0x73];
pub const INFORMATION: [u8; 4] = [0xf4, 0x7b, 0x77, 0x40];
/// ERC-721 `ownerOf(uint256)`, ERC-2981 `royaltyInfo(uint256,uint256)`
pub const OWNER_OF: [u8; 4] = [0x63, 0x52, 0x21, 0x1e];
pub const ROYALTY_INFO: [u8; 4] = [0x2a, 0x55, 0x20, 0x5a];

pub type Word = [u8; 32];

/// `BasicOrderParameters`, field for field. uint256s that are identities or hashed stay 32-byte words; amounts are u128
/// (an amount wider than that is refused at decode: it is not a price anyone pays).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicOrder {
    pub consideration_token: [u8; 20],
    pub consideration_identifier: Word,
    pub consideration_amount: u128,
    pub offerer: [u8; 20],
    pub zone: [u8; 20],
    pub offer_token: [u8; 20],
    pub offer_identifier: Word,
    pub offer_amount: u128,
    pub basic_order_type: u8,
    pub start_time: Word,
    pub end_time: Word,
    pub zone_hash: Word,
    pub salt: Word,
    pub offerer_conduit_key: Word,
    pub fulfiller_conduit_key: Word,
    pub total_original_additional_recipients: u64,
    pub additional_recipients: Vec<(u128, [u8; 20])>,
    pub signature: Vec<u8>,
}

fn w_at(b: &[u8], off: usize) -> Result<Word, String> {
    b.get(off..off + 32).map(|s| s.try_into().expect("32 bytes")).ok_or_else(|| "the order is shorter than its encoding says".into())
}
fn addr_of(w: &Word, what: &str) -> Result<[u8; 20], String> {
    if w[..12].iter().any(|x| *x != 0) {
        return Err(format!("{what} is not an address (dirty upper bytes)"));
    }
    Ok(w[12..].try_into().expect("20 bytes"))
}
/// A word as an integer of at most `bytes` bytes (≤ 16); anything wider is refused, never truncated.
fn narrow(w: &Word, bytes: usize, what: &str) -> Result<u128, String> {
    debug_assert!(bytes <= 16);
    if w[..32 - bytes].iter().any(|x| *x != 0) {
        return Err(format!("{what} is out of range"));
    }
    Ok(w[16..].iter().fold(0u128, |a, &x| (a << 8) | x as u128))
}
/// A uint256 time as seconds, saturating: `endTime = type(uint256).max` means "never", which is `u64::MAX` here.
pub fn secs(w: &Word) -> u64 {
    if w[..24].iter().any(|x| *x != 0) { u64::MAX } else { u64::from_be_bytes(w[24..].try_into().expect("8 bytes")) }
}
pub fn word_u128(v: u128) -> Word {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&v.to_be_bytes());
    w
}
pub fn word_addr(a: &[u8; 20]) -> Word {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a);
    w
}

/// Decode `fulfillBasicOrder` calldata (either selector). Offsets must be the canonical ones — Seaport itself refuses any
/// other layout for a basic order — and anything after the encoding (a marketplace's tracking tag) is ignored, because we
/// never send these bytes: [`encode`] builds our own.
pub fn decode(calldata: &[u8]) -> Result<BasicOrder, String> {
    let sel: [u8; 4] = calldata.get(..4).ok_or("no selector")?.try_into().expect("4 bytes");
    if sel != FULFILL_BASIC_ORDER && sel != FULFILL_BASIC_ORDER_EFFICIENT {
        return Err(format!("selector 0x{} is not fulfillBasicOrder", hex::encode(sel)));
    }
    let b = &calldata[4..];
    let w = |i: usize| w_at(b, 32 + 32 * i);
    if narrow(&w_at(b, 0)?, 8, "offset")? != 0x20 {
        return Err("the order is not at the canonical offset".into());
    }
    let recipients_off = narrow(&w(16)?, 8, "recipients offset")? as usize;
    let signature_off = narrow(&w(17)?, 8, "signature offset")? as usize;
    if recipients_off != 0x240 {
        return Err("additionalRecipients is not at the canonical offset".into());
    }
    let n = narrow(&w_at(b, 32 + recipients_off)?, 2, "recipient count")? as usize;
    if signature_off != 0x260 + 64 * n {
        return Err("the signature is not at the canonical offset".into());
    }
    let mut additional_recipients = Vec::with_capacity(n);
    for k in 0..n {
        let base = 32 + recipients_off + 32 + 64 * k;
        additional_recipients.push((narrow(&w_at(b, base)?, 16, "a recipient's amount")?, addr_of(&w_at(b, base + 32)?, "a recipient")?));
    }
    let sig_len = narrow(&w_at(b, 32 + signature_off)?, 2, "signature length")? as usize;
    let sig_start = 32 + signature_off + 32;
    let signature = b.get(sig_start..sig_start + sig_len).ok_or("the signature runs past the end")?.to_vec();
    let bot = narrow(&w(8)?, 1, "basicOrderType")? as u8;
    Ok(BasicOrder {
        consideration_token: addr_of(&w(0)?, "considerationToken")?,
        consideration_identifier: w(1)?,
        consideration_amount: narrow(&w(2)?, 16, "considerationAmount")?,
        offerer: addr_of(&w(3)?, "offerer")?,
        zone: addr_of(&w(4)?, "zone")?,
        offer_token: addr_of(&w(5)?, "offerToken")?,
        offer_identifier: w(6)?,
        offer_amount: narrow(&w(7)?, 16, "offerAmount")?,
        basic_order_type: bot,
        start_time: w(9)?,
        end_time: w(10)?,
        zone_hash: w(11)?,
        salt: w(12)?,
        offerer_conduit_key: w(13)?,
        fulfiller_conduit_key: w(14)?,
        total_original_additional_recipients: narrow(&w(15)?, 8, "totalOriginalAdditionalRecipients")? as u64,
        additional_recipients,
        signature,
    })
}

/// Our own `fulfillBasicOrder` calldata for `o`: canonical offsets, the plain selector, the fulfiller conduit zeroed (we pay
/// in ETH and approve nothing), and nothing appended.
pub fn encode(o: &BasicOrder) -> Vec<u8> {
    let mut out = FULFILL_BASIC_ORDER.to_vec();
    let n = o.additional_recipients.len();
    let heads: [Word; 18] = [
        word_addr(&o.consideration_token), o.consideration_identifier, word_u128(o.consideration_amount), word_addr(&o.offerer),
        word_addr(&o.zone), word_addr(&o.offer_token), o.offer_identifier, word_u128(o.offer_amount),
        word_u128(o.basic_order_type as u128), o.start_time, o.end_time, o.zone_hash, o.salt, o.offerer_conduit_key, [0u8; 32],
        word_u128(o.total_original_additional_recipients as u128), word_u128(0x240), word_u128((0x260 + 64 * n) as u128),
    ];
    out.extend_from_slice(&word_u128(0x20));
    heads.iter().for_each(|h| out.extend_from_slice(h));
    out.extend_from_slice(&word_u128(n as u128));
    for (amount, to) in &o.additional_recipients {
        out.extend_from_slice(&word_u128(*amount));
        out.extend_from_slice(&word_addr(to));
    }
    out.extend_from_slice(&word_u128(o.signature.len() as u128));
    let mut sig = o.signature.clone();
    sig.resize(o.signature.len().div_ceil(32) * 32, 0);
    out.extend(sig);
    out
}

impl BasicOrder {
    /// What the buyer pays: the offerer's amount plus every additional recipient's, or `None` on overflow.
    pub fn total(&self) -> Option<u128> {
        self.additional_recipients.iter().try_fold(self.consideration_amount, |a, (x, _)| a.checked_add(*x))
    }

    /// The order as `OrderComponents`, for hashing. Only route 0 (ETH → ERC-721) is expressed; [`verify`] refuses the rest
    /// before this matters.
    pub fn components(&self, counter: Word) -> OrderComponents {
        let mut consideration = vec![ConsiderationItem { item_type: 0, token: [0; 20], identifier: [0; 32],
            start: word_u128(self.consideration_amount), end: word_u128(self.consideration_amount), recipient: self.offerer }];
        let original = (self.total_original_additional_recipients as usize).min(self.additional_recipients.len());
        consideration.extend(self.additional_recipients[..original].iter().map(|(a, r)| ConsiderationItem { item_type: 0,
            token: [0; 20], identifier: [0; 32], start: word_u128(*a), end: word_u128(*a), recipient: *r }));
        OrderComponents {
            offerer: self.offerer,
            zone: self.zone,
            offer: vec![OfferItem { item_type: 2, token: self.offer_token, identifier: self.offer_identifier,
                start: word_u128(self.offer_amount), end: word_u128(self.offer_amount) }],
            consideration,
            order_type: self.basic_order_type % 4,
            start_time: self.start_time,
            end_time: self.end_time,
            zone_hash: self.zone_hash,
            salt: self.salt,
            conduit_key: self.offerer_conduit_key,
            counter,
        }
    }
}

// ---- EIP-712: the order hash Seaport computes, and the digest the offerer signed ---------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferItem { pub item_type: u8, pub token: [u8; 20], pub identifier: Word, pub start: Word, pub end: Word }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsiderationItem { pub item_type: u8, pub token: [u8; 20], pub identifier: Word, pub start: Word, pub end: Word, pub recipient: [u8; 20] }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderComponents {
    pub offerer: [u8; 20],
    pub zone: [u8; 20],
    pub offer: Vec<OfferItem>,
    pub consideration: Vec<ConsiderationItem>,
    pub order_type: u8,
    pub start_time: Word,
    pub end_time: Word,
    pub zone_hash: Word,
    pub salt: Word,
    pub conduit_key: Word,
    pub counter: Word,
}

const OFFER_ITEM: &str = "OfferItem(uint8 itemType,address token,uint256 identifierOrCriteria,uint256 startAmount,uint256 endAmount)";
const CONSIDERATION_ITEM: &str =
    "ConsiderationItem(uint8 itemType,address token,uint256 identifierOrCriteria,uint256 startAmount,uint256 endAmount,address recipient)";
const ORDER_COMPONENTS: &str = "OrderComponents(address offerer,address zone,OfferItem[] offer,ConsiderationItem[] consideration,uint8 orderType,uint256 startTime,uint256 endTime,bytes32 zoneHash,uint256 salt,bytes32 conduitKey,uint256 counter)";

fn hash_words(ws: &[Word]) -> Word {
    keccak(&ws.concat())
}

impl OrderComponents {
    /// Seaport's order hash (EIP-712 `hashStruct(OrderComponents)`).
    pub fn hash(&self) -> Word {
        let oth = keccak(OFFER_ITEM.as_bytes());
        let cth = keccak(CONSIDERATION_ITEM.as_bytes());
        let offer: Vec<Word> = self.offer.iter()
            .map(|i| hash_words(&[oth, word_u128(i.item_type as u128), word_addr(&i.token), i.identifier, i.start, i.end])).collect();
        let cons: Vec<Word> = self.consideration.iter().map(|i| hash_words(&[cth, word_u128(i.item_type as u128), word_addr(&i.token),
            i.identifier, i.start, i.end, word_addr(&i.recipient)])).collect();
        let typehash = keccak(format!("{ORDER_COMPONENTS}{CONSIDERATION_ITEM}{OFFER_ITEM}").as_bytes());
        hash_words(&[typehash, word_addr(&self.offerer), word_addr(&self.zone), hash_words(&offer), hash_words(&cons),
            word_u128(self.order_type as u128), self.start_time, self.end_time, self.zone_hash, self.salt, self.conduit_key,
            self.counter])
    }
}

/// Seaport 1.6's EIP-712 domain separator on `chain_id` — what `information()` returns there. Checked against the chain at
/// the pin, so an order hash computed here means what Seaport means.
pub fn domain_separator(chain_id: u64, seaport: &[u8; 20]) -> Word {
    let t = keccak(b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    hash_words(&[t, keccak(b"Seaport"), keccak(b"1.6"), word_u128(chain_id as u128), word_addr(seaport)])
}

pub fn digest(domain: &Word, order_hash: &Word) -> Word {
    keccak(&[&[0x19u8, 0x01][..], domain, order_hash].concat())
}

/// The EOA that signed `digest`, for a 65-byte (`r‖s‖v`) or 64-byte EIP-2098 signature; `None` for anything else (a bulk-order
/// proof or an ERC-1271 wallet: those are left to the simulation) or for a signature that recovers nothing.
pub fn recover(digest: &Word, sig: &[u8]) -> Option<[u8; 20]> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    let (r, s, v): ([u8; 32], [u8; 32], u8) = match sig.len() {
        65 => (sig[..32].try_into().ok()?, sig[32..64].try_into().ok()?, match sig[64] { 27 | 0 => 0, 28 | 1 => 1, _ => return None }),
        64 => {
            let mut vs: [u8; 32] = sig[32..].try_into().ok()?;
            let v = vs[0] >> 7;
            vs[0] &= 0x7f;
            (sig[..32].try_into().ok()?, vs, v)
        }
        _ => return None,
    };
    let sig = Signature::from_scalars(r, s).ok()?;
    if sig.normalize_s().is_some() {
        return None; // Seaport refuses high-s too
    }
    let vk = VerifyingKey::recover_from_prehash(digest, &sig, RecoveryId::from_byte(v)?).ok()?;
    let p = vk.to_encoded_point(false);
    Some(keccak(&p.as_bytes()[1..])[12..].try_into().expect("20 bytes"))
}

// ---- the verifier ------------------------------------------------------------------------------------------------------------

/// What was recorded before any order existed: the one token we buy, and the most we pay for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub collection: [u8; 20],
    pub token_id: Word,
    pub cap_wei: u128,
}

/// The chain's rules for an order, from its configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    pub fee_recipients: Vec<[u8; 20]>,
    pub max_fee_bps: u32,
    pub min_order_life_s: u64,
}

/// What the chain said at the pin, fed in by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainFacts {
    pub ours: [u8; 20],
    pub pin_timestamp: u64,
    /// The collection's ERC-2981 receiver at the pin, if it has one.
    pub royalty_receiver: Option<[u8; 20]>,
}

/// An order that passed: what to send, and what it pays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub total_wei: u128,
    pub calldata: Vec<u8>,
}

/// What an order sells and pays, whatever its encoding: the fields §6's rules 3–7 judge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terms {
    pub offerer: [u8; 20],
    pub offer_token: [u8; 20],
    pub offer_identifier: Word,
    pub offer_amount: u128,
    /// What the offerer receives.
    pub to_offerer: u128,
    /// Everyone else paid, in order.
    pub extras: Vec<(u128, [u8; 20])>,
    /// How many of `extras` the offerer signed for (the rest would be tips).
    pub signed_extras: u64,
    pub start_time: Word,
    pub end_time: Word,
}

/// Rules 3–7, shared by both encodings. Returns the total.
pub fn verify_terms(o: &Terms, t: &Target, rules: &Rules, facts: &ChainFacts) -> Result<u128, String> {
    // rule 3: exactly the listed token
    if o.offer_token != t.collection {
        return Err(format!("refused (wrong collection): the order sells 0x{}", hex::encode(o.offer_token)));
    }
    if o.offer_identifier != t.token_id {
        return Err("refused (wrong token): the order sells another token id".into());
    }
    if o.offer_amount != 1 {
        return Err(format!("refused (amount): an ERC-721 order for {} of it", o.offer_amount));
    }
    // rule 4: the offerer is a real third party; the NFT comes to us (msg.sender for a basic order, our recipient otherwise)
    if o.offerer == facts.ours || o.offerer == [0; 20] {
        return Err("refused (offerer): the offerer is us or nobody".into());
    }
    // rule 6: no money to strangers, no tips, and the fees bounded
    if o.signed_extras != o.extras.len() as u64 {
        return Err("refused (tips): the order carries recipients beyond the offerer's own".into());
    }
    let mut fees: u128 = 0;
    for (amount, to) in &o.extras {
        let known = rules.fee_recipients.contains(to) || facts.royalty_receiver.as_ref() == Some(to);
        if !known || *to == [0; 20] {
            return Err(format!("refused (unknown recipient): 0x{} is neither a marketplace fee recipient nor the royalty receiver",
                hex::encode(to)));
        }
        fees = fees.checked_add(*amount).ok_or("refused (overflow): the fees overflow")?;
    }
    // rule 5: total within the recorded cap
    let total = o.to_offerer.checked_add(fees).ok_or("refused (overflow): the total overflows")?;
    if total > t.cap_wei {
        return Err(format!("refused (over cap): the order costs {total} wei; the recorded cap is {}", t.cap_wei));
    }
    if total == 0 {
        return Err("refused (price): a free order is not a listing".into());
    }
    if fees.saturating_mul(10_000) > total.saturating_mul(rules.max_fee_bps as u128) {
        return Err(format!("refused (fees): {fees} of {total} wei goes to others, over {} bps", rules.max_fee_bps));
    }
    // rule 7: live at the pin, and for long enough after it
    let (start, end) = (secs(&o.start_time), secs(&o.end_time));
    if start > facts.pin_timestamp {
        return Err("refused (not started): the order starts after the pinned block".into());
    }
    if end <= facts.pin_timestamp.saturating_add(rules.min_order_life_s) {
        return Err(format!("refused (expired): the order ends at {end}, within {} s of the pinned block's {}",
            rules.min_order_life_s, facts.pin_timestamp));
    }
    Ok(total)
}

/// Every rule of `docs/v4/eth-nft-canister.md` §6 that needs no chain read beyond `facts`, for a basic order. Each refusal
/// names its rule.
pub fn verify(o: &BasicOrder, t: &Target, rules: &Rules, facts: &ChainFacts) -> Result<Checked, String> {
    // rule 2: ETH for one ERC-721, on an open order (0 = full open, 1 = partial open; 2 and 3 are restricted: a zone decides)
    match o.basic_order_type {
        0 | 1 => {}
        2 | 3 => return Err(format!("refused (restricted order, type {}): its zone must authorise the fill, and a basic order cannot \
            carry the zone's data", o.basic_order_type)),
        n => return Err(format!("refused (order type {n}): only ETH for an ERC-721 is bought")),
    }
    if o.consideration_token != [0; 20] || o.consideration_identifier != [0; 32] {
        return Err("refused (payment): the price is not native ETH".into());
    }
    let terms = Terms { offerer: o.offerer, offer_token: o.offer_token, offer_identifier: o.offer_identifier,
        offer_amount: o.offer_amount, to_offerer: o.consideration_amount, extras: o.additional_recipients.clone(),
        signed_extras: o.total_original_additional_recipients, start_time: o.start_time, end_time: o.end_time };
    Ok(Checked { total_wei: verify_terms(&terms, t, rules, facts)?, calldata: encode(o) })
}

/// Rule 9 for a basic order. See [`offerer_signed`].
pub fn signed_by_offerer(o: &BasicOrder, chain_id: u64, seaport: &[u8; 20], counter: Word) -> Result<bool, String> {
    offerer_signed(&o.components(counter).hash(), &o.offerer, &o.signature, chain_id, seaport)
}

/// Rule 9: the offerer signed `order_hash`, for a plain EOA signature (64 or 65 bytes) or a Seaport bulk-order signature (the
/// same, then a 3-byte index and a proof of 1–24 levels). `Ok(false)` when the signature is of neither kind (an ERC-1271
/// contract wallet, or none): the simulation is then the check.
pub fn offerer_signed(order_hash: &Word, offerer: &[u8; 20], sig: &[u8], chain_id: u64, seaport: &[u8; 20]) -> Result<bool, String> {
    let domain = domain_separator(chain_id, seaport);
    let (d, s) = match sig.len() {
        64 | 65 => (digest(&domain, order_hash), sig),
        _ => match bulk_root(order_hash, sig) {
            Some((root, height, sig_len)) => (digest(&domain, &hash_words(&[bulk_typehash(height), root])), &sig[..sig_len]),
            None => return Ok(false),
        },
    };
    match recover(&d, s) {
        Some(a) if a == *offerer => Ok(true),
        Some(a) => Err(format!("refused (signature): it was signed by 0x{}, not the offerer", hex::encode(a))),
        None => Err("refused (signature): it recovers no signer".into()),
    }
}

/// `BulkOrder(OrderComponents[2]…[2] tree)` for a tree of `height` levels, with its referenced types in EIP-712 order.
fn bulk_typehash(height: usize) -> Word {
    keccak(format!("BulkOrder(OrderComponents{} tree){CONSIDERATION_ITEM}{OFFER_ITEM}{ORDER_COMPONENTS}", "[2]".repeat(height)).as_bytes())
}

/// A bulk-order signature's Merkle root over `order_hash`, its height, and the length of the ECDSA part — Seaport's own
/// `_computeBulkOrderProof`: the key's low bit says whether the node is the right child.
fn bulk_root(order_hash: &Word, sig: &[u8]) -> Option<(Word, usize, usize)> {
    let sig_len = [64usize, 65].into_iter().find(|l| sig.len() > l + 3 && (sig.len() - l - 3) % 32 == 0)?;
    let height = (sig.len() - sig_len - 3) / 32;
    if !(1..=24).contains(&height) {
        return None;
    }
    let mut key = u32::from_be_bytes([0, sig[sig_len], sig[sig_len + 1], sig[sig_len + 2]]);
    let mut node = *order_hash;
    for i in 0..height {
        let p: Word = sig[sig_len + 3 + 32 * i..sig_len + 35 + 32 * i].try_into().ok()?;
        node = if key & 1 == 1 { hash_words(&[p, node]) } else { hash_words(&[node, p]) };
        key >>= 1;
    }
    Some((node, height, sig_len))
}

// ---- a listing the canister signs (docs/v6/canister-floor-followups.md §3.5) ---------------------------------------------------

/// A plain full-open listing: `offerer` offers one ERC-721 (`collection`, `token_id`) for `ask_wei` of native ETH to itself,
/// plus `fee` (a recipient and its share in bps of the ask) when given; zone zero, conduit zero, `order_type` 0 (FULL_OPEN).
/// The same `OrderComponents` a taker's `fulfillBasicOrder` route 0 hashes, so [`offerer_signed`] verifies it.
#[allow(clippy::too_many_arguments)]
pub fn listing_order(offerer: &[u8; 20], collection: &[u8; 20], token_id: &Word, ask_wei: u128, fee: Option<([u8; 20], u32)>, start: u64,
    end: u64, salt: Word, counter: Word) -> OrderComponents {
    let mut consideration = vec![ConsiderationItem { item_type: 0, token: [0; 20], identifier: [0; 32], start: word_u128(ask_wei),
        end: word_u128(ask_wei), recipient: *offerer }];
    if let Some((to, bps)) = fee {
        let amount = ask_wei.saturating_mul(bps as u128) / 10_000;
        if amount > 0 {
            consideration.push(ConsiderationItem { item_type: 0, token: [0; 20], identifier: [0; 32], start: word_u128(amount), end: word_u128(amount),
                recipient: to });
        }
    }
    OrderComponents {
        offerer: *offerer,
        zone: [0; 20],
        offer: vec![OfferItem { item_type: 2, token: *collection, identifier: *token_id, start: word_u128(1), end: word_u128(1) }],
        consideration,
        order_type: 0,
        start_time: word_u128(start as u128),
        end_time: word_u128(end as u128),
        zone_hash: [0; 32],
        salt,
        conduit_key: [0; 32],
        counter,
    }
}

/// The order as a venue's API takes it (OpenSea's `POST …/seaport/listings` shape: `parameters` plus `signature`), with
/// `totalOriginalConsiderationItems` and the order hash beside.
pub fn listing_json(o: &OrderComponents, hash: &Word, signature_hex: &str) -> String {
    let a = |x: &[u8; 20]| format!("0x{}", hex::encode(x));
    let w = |x: &Word| format!("0x{}", hex::encode(x));
    let n = |x: &Word| u128::from_be_bytes(x[16..].try_into().expect("16 bytes")).to_string();
    let offer: Vec<serde_json::Value> = o.offer.iter().map(|i| serde_json::json!({ "itemType": i.item_type, "token": a(&i.token),
        "identifierOrCriteria": n(&i.identifier), "startAmount": n(&i.start), "endAmount": n(&i.end) })).collect();
    let cons: Vec<serde_json::Value> = o.consideration.iter().map(|i| serde_json::json!({ "itemType": i.item_type, "token": a(&i.token),
        "identifierOrCriteria": n(&i.identifier), "startAmount": n(&i.start), "endAmount": n(&i.end), "recipient": a(&i.recipient) })).collect();
    serde_json::json!({
        "parameters": {
            "offerer": a(&o.offerer), "zone": a(&o.zone), "offer": offer, "consideration": cons, "orderType": o.order_type,
            "startTime": n(&o.start_time), "endTime": n(&o.end_time), "zoneHash": w(&o.zone_hash), "salt": n(&o.salt),
            "conduitKey": w(&o.conduit_key), "counter": n(&o.counter), "totalOriginalConsiderationItems": o.consideration.len(),
        },
        "signature": signature_hex,
        "orderHash": w(hash),
    }).to_string()
}

// ---- restricted orders through OpenSea's SignedZone: fulfillAdvancedOrder (docs/v4/eth-nft-canister.md §12) ----------------

/// `fulfillAdvancedOrder(((…),uint120,uint120,bytes,bytes),(…)[],bytes32,address)`
pub const FULFILL_ADVANCED_ORDER: [u8; 4] = [0xe7, 0xac, 0xab, 0x24];

/// `fulfillAdvancedOrder`'s arguments, decoded. The order itself is [`OrderComponents`] with `counter` left zero (the
/// parameters carry `totalOriginalConsiderationItems` instead; the counter is read at the pin).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvancedOrder {
    pub order: OrderComponents,
    pub total_original_consideration_items: u64,
    pub numerator: u128,
    pub denominator: u128,
    pub signature: Vec<u8>,
    pub extra_data: Vec<u8>,
    pub criteria_resolvers: usize,
    pub fulfiller_conduit_key: Word,
    pub recipient: [u8; 20],
}

fn at_off(b: &[u8], base: usize, i: usize, what: &str) -> Result<usize, String> {
    let o = narrow(&w_at(b, base + 32 * i)?, 4, what)? as usize;
    base.checked_add(o).filter(|x| *x <= b.len()).ok_or_else(|| format!("{what} points past the end"))
}
fn bytes_at(b: &[u8], at: usize, what: &str) -> Result<Vec<u8>, String> {
    let n = narrow(&w_at(b, at)?, 4, what)? as usize;
    b.get(at + 32..at + 32 + n).map(|x| x.to_vec()).ok_or_else(|| format!("{what} runs past the end"))
}

/// Decode `fulfillAdvancedOrder` calldata. Offsets are followed, not assumed, and every read is bounded; anything after the
/// encoding (a marketplace tag) is ignored, because [`encode_advanced`] builds what is sent.
pub fn decode_advanced(calldata: &[u8]) -> Result<AdvancedOrder, String> {
    if calldata.get(..4) != Some(&FULFILL_ADVANCED_ORDER[..]) {
        return Err("not fulfillAdvancedOrder".into());
    }
    let b = &calldata[4..];
    let ao = at_off(b, 0, 0, "advancedOrder")?;
    let cr = at_off(b, 0, 1, "criteriaResolvers")?;
    let criteria_resolvers = narrow(&w_at(b, cr)?, 4, "criteriaResolvers length")? as usize;
    let p = at_off(b, ao, 0, "parameters")?;
    let pw = |i: usize| w_at(b, p + 32 * i);
    let offer_at = at_off(b, p, 2, "offer")?;
    let cons_at = at_off(b, p, 3, "consideration")?;
    let n_offer = narrow(&w_at(b, offer_at)?, 2, "offer length")? as usize;
    let n_cons = narrow(&w_at(b, cons_at)?, 2, "consideration length")? as usize;
    let mut offer = Vec::with_capacity(n_offer);
    for k in 0..n_offer {
        let q = offer_at + 32 + 160 * k;
        offer.push(OfferItem { item_type: narrow(&w_at(b, q)?, 1, "itemType")? as u8, token: addr_of(&w_at(b, q + 32)?, "token")?,
            identifier: w_at(b, q + 64)?, start: w_at(b, q + 96)?, end: w_at(b, q + 128)? });
    }
    let mut consideration = Vec::with_capacity(n_cons);
    for k in 0..n_cons {
        let q = cons_at + 32 + 192 * k;
        consideration.push(ConsiderationItem { item_type: narrow(&w_at(b, q)?, 1, "itemType")? as u8,
            token: addr_of(&w_at(b, q + 32)?, "token")?, identifier: w_at(b, q + 64)?, start: w_at(b, q + 96)?,
            end: w_at(b, q + 128)?, recipient: addr_of(&w_at(b, q + 160)?, "recipient")? });
    }
    Ok(AdvancedOrder {
        order: OrderComponents {
            offerer: addr_of(&pw(0)?, "offerer")?, zone: addr_of(&pw(1)?, "zone")?, offer, consideration,
            order_type: narrow(&pw(4)?, 1, "orderType")? as u8, start_time: pw(5)?, end_time: pw(6)?, zone_hash: pw(7)?,
            salt: pw(8)?, conduit_key: pw(9)?, counter: [0; 32],
        },
        total_original_consideration_items: narrow(&pw(10)?, 8, "totalOriginalConsiderationItems")? as u64,
        numerator: narrow(&w_at(b, ao + 32)?, 15, "numerator")?,
        denominator: narrow(&w_at(b, ao + 64)?, 15, "denominator")?,
        signature: bytes_at(b, at_off(b, ao, 3, "signature")?, "signature")?,
        extra_data: bytes_at(b, at_off(b, ao, 4, "extraData")?, "extraData")?,
        criteria_resolvers,
        fulfiller_conduit_key: w_at(b, 64)?,
        recipient: addr_of(&w_at(b, 96)?, "recipient")?,
    })
}

fn padded(v: &[u8]) -> Vec<u8> {
    let mut out = word_u128(v.len() as u128).to_vec();
    let mut d = v.to_vec();
    d.resize(v.len().div_ceil(32) * 32, 0);
    out.extend(d);
    out
}

/// Our own `fulfillAdvancedOrder` calldata: the order, its signature and the zone's `extraData` as decoded; numerator and
/// denominator 1; **no criteria resolvers, the fulfiller conduit key zero, and the recipient `recipient`** (ours).
pub fn encode_advanced(o: &AdvancedOrder, recipient: &[u8; 20]) -> Vec<u8> {
    let c = &o.order;
    // OrderParameters: 11 head words, then offer[], then consideration[]
    let offer_len = 32 + 160 * c.offer.len();
    let mut params = Vec::new();
    for w in [word_addr(&c.offerer), word_addr(&c.zone), word_u128(11 * 32), word_u128((11 * 32 + offer_len) as u128),
        word_u128(c.order_type as u128), c.start_time, c.end_time, c.zone_hash, c.salt, c.conduit_key,
        word_u128(o.total_original_consideration_items as u128)] {
        params.extend_from_slice(&w);
    }
    params.extend_from_slice(&word_u128(c.offer.len() as u128));
    for i in &c.offer {
        for w in [word_u128(i.item_type as u128), word_addr(&i.token), i.identifier, i.start, i.end] { params.extend_from_slice(&w) }
    }
    params.extend_from_slice(&word_u128(c.consideration.len() as u128));
    for i in &c.consideration {
        for w in [word_u128(i.item_type as u128), word_addr(&i.token), i.identifier, i.start, i.end, word_addr(&i.recipient)] {
            params.extend_from_slice(&w)
        }
    }
    // AdvancedOrder: parameters, numerator, denominator, signature, extraData
    let sig = padded(&o.signature);
    let mut adv = Vec::new();
    let p_off = 5 * 32;
    let s_off = p_off + params.len();
    let e_off = s_off + sig.len();
    for w in [word_u128(p_off as u128), word_u128(1), word_u128(1), word_u128(s_off as u128), word_u128(e_off as u128)] {
        adv.extend_from_slice(&w);
    }
    adv.extend(params);
    adv.extend(sig);
    adv.extend(padded(&o.extra_data));
    let mut out = FULFILL_ADVANCED_ORDER.to_vec();
    for w in [word_u128(4 * 32), word_u128((4 * 32 + adv.len()) as u128), [0u8; 32], word_addr(recipient)] {
        out.extend_from_slice(&w);
    }
    out.extend(adv);
    out.extend_from_slice(&word_u128(0)); // criteriaResolvers: empty
    out
}

/// One SIP-7 context segment of OpenSea's SignedZone (substandards 1, 7, 8, 9: its `getSeaportMetadata()` lists these).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Substandard {
    pub id: u8,
    pub identifier: Word,
    pub registry: Option<[u8; 20]>,
    pub operator: Option<[u8; 20]>,
}

/// SIP-6 v0 + SIP-7 `extraData`: expected fulfiller, expiration, the zone signer's EIP-2098 signature, and the context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneData {
    pub fulfiller: [u8; 20],
    pub expiration: u64,
    pub signature: [u8; 64],
    pub context: Vec<u8>,
    pub substandards: Vec<Substandard>,
}

/// Parse a SignedZone `extraData`. Anything this does not recognise is refused as substandard: an unknown version byte, a
/// short body, an unknown substandard, or a segment cut short.
pub fn decode_zone_data(x: &[u8]) -> Result<ZoneData, String> {
    let bad = |why: &str| Err(format!("refused (zone data): {why}"));
    if x.len() < 93 {
        return bad(&format!("{} bytes; SIP-7 needs at least 93", x.len()));
    }
    if x[0] != 0 {
        return bad(&format!("SIP-6 version {}; only 0 is accepted", x[0]));
    }
    let context = x[93..].to_vec();
    let mut substandards = Vec::new();
    let mut i = 0;
    while i < context.len() {
        let id = if context[i] == 0 { 1 } else { context[i] };
        let (need, reg, op) = match id { 1 => (32, false, false), 7 => (72, true, true), 8 | 9 => (52, true, false),
            n => return bad(&format!("substandard {n} is not one this canister accepts")) };
        let seg = context.get(i + 1..i + 1 + need).ok_or("refused (zone data): a substandard segment is cut short")?;
        substandards.push(Substandard { id, identifier: seg[..32].try_into().expect("32"),
            registry: reg.then(|| seg[32..52].try_into().expect("20")), operator: op.then(|| seg[52..72].try_into().expect("20")) });
        i += 1 + need;
    }
    Ok(ZoneData { fulfiller: x[1..21].try_into().expect("20"), expiration: u64::from_be_bytes(x[21..29].try_into().expect("8")),
        signature: x[29..93].try_into().expect("64"), context, substandards })
}

/// The digest a SignedZone signer signs (SIP-7 `SignedOrder`), under the zone's own domain separator.
pub fn zone_digest(zone_domain: &Word, z: &ZoneData, order_hash: &Word) -> Word {
    let th = keccak(b"SignedOrder(address fulfiller,uint64 expiration,bytes32 orderHash,bytes context)");
    let s = hash_words(&[th, word_addr(&z.fulfiller), word_u128(z.expiration as u128), *order_hash, keccak(&z.context)]);
    digest(zone_domain, &s)
}

/// The chain's SignedZone rules.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ZoneRules {
    /// Allowed zones (their code hash is checked at the pin by the caller).
    pub zones: Vec<[u8; 20]>,
    /// Transfer validators a substandard 7/8/9 may name.
    pub registries: Vec<[u8; 20]>,
}

/// §12.3: a restricted order through an allowlisted SignedZone, bought whole, to us. Returns our own calldata and the total.
/// `now` is the later of the pinned block's timestamp and IC time: a zone signature must outlive both by the minimum life.
pub fn verify_advanced(o: &AdvancedOrder, t: &Target, rules: &Rules, zr: &ZoneRules, facts: &ChainFacts, now: u64)
    -> Result<(Checked, ZoneData), String> {
    let c = &o.order;
    if !matches!(c.order_type, 2 | 3) {
        return Err(format!("refused (order type {}): fulfillAdvancedOrder is accepted only for a restricted order through an \
            allow-listed zone; an open listing is bought as a basic order", c.order_type));
    }
    if !zr.zones.contains(&c.zone) {
        return Err(format!("refused (zone): 0x{} is not an allow-listed SignedZone on this chain", hex::encode(c.zone)));
    }
    if o.numerator != 1 || o.denominator != 1 {
        return Err(format!("refused (fraction): {}/{} of the order; only the whole order is bought", o.numerator, o.denominator));
    }
    if o.criteria_resolvers != 0 {
        return Err("refused (criteria): criteria resolvers are not accepted".into());
    }
    let [offer] = &c.offer[..] else { return Err(format!("refused (offer): {} offer items; exactly one ERC-721", c.offer.len())) };
    if offer.item_type != 2 || offer.start != offer.end {
        return Err("refused (offer): the offer is not one ERC-721 at a fixed amount".into());
    }
    let Some((first, rest)) = c.consideration.split_first() else { return Err("refused (payment): no consideration".into()) };
    let mut amounts = Vec::with_capacity(c.consideration.len());
    for i in &c.consideration {
        if i.item_type != 0 || i.token != [0; 20] || i.identifier != [0; 32] {
            return Err("refused (payment): the price is not native ETH".into());
        }
        if i.start != i.end {
            return Err("refused (payment): an amount that changes over time".into());
        }
        amounts.push(narrow(&i.start, 16, "an amount")?);
    }
    if first.recipient != c.offerer {
        return Err("refused (payment): the first consideration item is not paid to the offerer".into());
    }
    let terms = Terms {
        offerer: c.offerer, offer_token: offer.token, offer_identifier: offer.identifier, offer_amount: narrow(&offer.start, 16, "amount")?,
        to_offerer: amounts[0], extras: rest.iter().zip(&amounts[1..]).map(|(i, a)| (*a, i.recipient)).collect(),
        signed_extras: o.total_original_consideration_items.saturating_sub(1), start_time: c.start_time, end_time: c.end_time,
    };
    let total = verify_terms(&terms, t, rules, facts)?;
    // the zone's data: for us, not expiring before we can land, and only substandards we understand
    let z = decode_zone_data(&o.extra_data)?;
    if z.fulfiller != facts.ours {
        return Err(format!("refused (zone data): the zone signed for fulfiller 0x{}, not us", hex::encode(z.fulfiller)));
    }
    if z.expiration <= now.max(facts.pin_timestamp).saturating_add(rules.min_order_life_s) {
        return Err(format!("refused (zone data): the zone signature expires at {}, within {} s of now ({})", z.expiration,
            rules.min_order_life_s, now.max(facts.pin_timestamp)));
    }
    for s in &z.substandards {
        if s.identifier != first.identifier {
            return Err(format!("refused (zone data): substandard {} names identifier 0x{}, not the first received item's", s.id,
                hex::encode(s.identifier)));
        }
        if let Some(r) = s.registry {
            if !zr.registries.contains(&r) {
                return Err(format!("refused (zone data): substandard {} names registry 0x{}, not allow-listed", s.id, hex::encode(r)));
            }
        }
    }
    Ok((Checked { total_wei: total, calldata: encode_advanced(o, &facts.ours) }, z))
}

/// `information()` → (version, domain separator). The ABI is `(string, bytes32, address)`.
pub fn decode_information(ret: &[u8]) -> Result<(String, Word), String> {
    let off = narrow(&w_at(ret, 0)?, 8, "version offset")? as usize;
    let domain = w_at(ret, 32)?;
    let len = narrow(&w_at(ret, off)?, 2, "version length")? as usize;
    let v = ret.get(off + 32..off + 32 + len).ok_or("version runs past the end")?;
    Ok((String::from_utf8_lossy(v).into_owned(), domain))
}

/// `getOrderStatus` → (isValidated, isCancelled, totalFilled, totalSize).
pub fn decode_order_status(ret: &[u8]) -> Result<(bool, bool, u128, u128), String> {
    Ok((narrow(&w_at(ret, 0)?, 1, "isValidated")? == 1, narrow(&w_at(ret, 32)?, 1, "isCancelled")? == 1,
        narrow(&w_at(ret, 64)?, 16, "totalFilled")?, narrow(&w_at(ret, 96)?, 16, "totalSize")?))
}

/// An address returned by a view (`ownerOf`, `royaltyInfo`'s first word).
pub fn decode_address(ret: &[u8]) -> Result<[u8; 20], String> {
    addr_of(&w_at(ret, 0)?, "the returned address")
}

pub fn call1(sel: [u8; 4], arg: Word) -> Vec<u8> {
    [&sel[..], &arg].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ETH_FILL: &str = include_str!("../tests/fixtures/seaport/eth-fill-26037608.json");
    const BASE_FILL: &str = include_str!("../tests/fixtures/seaport/base-fill-51672614.json");
    const ETH_VALIDATED: &str = include_str!("../tests/fixtures/seaport/eth-validated-26027018.json");
    const OPENSEA_FEE: &str = "0000a26b00c1f0df003000390027140000faa719";
    const US: [u8; 20] = [0x5b; 20];

    struct Fill { chain: u64, input: Vec<u8>, value: u128, order_hash: Word, offerer: [u8; 20], block_ts: u64 }

    fn fill(s: &str, block_ts: u64) -> Fill {
        let v: serde_json::Value = serde_json::from_str(s).unwrap();
        let h = |k: &str| hex::decode(v[k].as_str().unwrap().trim_start_matches("0x")).unwrap();
        Fill { chain: v["chain_id"].as_u64().unwrap(), input: h("input"), order_hash: h("order_hash").try_into().unwrap(),
            value: u128::from_str_radix(v["value"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap(),
            offerer: h("offerer").try_into().unwrap(), block_ts }
    }
    // the timestamps of the blocks just before each real fill (26,037,607 and 51,672,613): the listing was live there
    fn eth() -> Fill { fill(ETH_FILL, 1_790_135_411) }
    fn base() -> Fill { fill(BASE_FILL, 1_790_134_573) }

    fn a(h: &str) -> [u8; 20] { hex::decode(h).unwrap().try_into().unwrap() }
    fn target(o: &BasicOrder, cap: u128) -> Target { Target { collection: o.offer_token, token_id: o.offer_identifier, cap_wei: cap } }
    fn rules() -> Rules { Rules { fee_recipients: vec![a(OPENSEA_FEE)], max_fee_bps: 1_500, min_order_life_s: 60 } }
    fn facts(f: &Fill) -> ChainFacts { ChainFacts { ours: US, pin_timestamp: f.block_ts, royalty_receiver: None } }

    #[test]
    fn a_listing_we_build_hashes_as_the_takers_basic_order_does_and_our_signature_recovers_as_the_offerer() {
        use k256::ecdsa::{signature::hazmat::PrehashSigner, SigningKey};
        let sk = SigningKey::from_bytes(&[7u8; 32].into()).unwrap();
        let p = sk.verifying_key().to_encoded_point(false);
        let me: [u8; 20] = keccak(&p.as_bytes()[1..])[12..].try_into().unwrap();
        let coll = a("bc4ca0eda7647a8ab7c2061c2e118a18a936f13d");
        let token = word_u128(9820);
        let fee_to = a(OPENSEA_FEE);
        let o = listing_order(&me, &coll, &token, 5 * 1_000_000_000_000_000_000, Some((fee_to, 50)), 1_800_000_000, 1_800_086_400, [3; 32], [0; 32]);
        assert_eq!(o.consideration.len(), 2);
        assert_eq!(o.consideration[1].start, word_u128(25 * 1_000_000_000_000_000));
        // the taker's basic order for it hashes to the same order hash
        let basic = BasicOrder { consideration_token: [0; 20], consideration_identifier: [0; 32], consideration_amount: 5 * 1_000_000_000_000_000_000,
            offerer: me, zone: [0; 20], offer_token: coll, offer_identifier: token, offer_amount: 1, basic_order_type: 0,
            start_time: word_u128(1_800_000_000), end_time: word_u128(1_800_086_400), zone_hash: [0; 32], salt: [3; 32], offerer_conduit_key: [0; 32],
            fulfiller_conduit_key: [0; 32], total_original_additional_recipients: 1, additional_recipients: vec![(25 * 1_000_000_000_000_000, fee_to)],
            signature: vec![] };
        assert_eq!(basic.components([0; 32]).hash(), o.hash());
        // signed by the key: recovers as the offerer on chain 1; a tampered ask does not
        let d = digest(&domain_separator(1, &SEAPORT_1_6), &o.hash());
        let (sig, rid): (k256::ecdsa::Signature, k256::ecdsa::RecoveryId) = sk.sign_prehash(&d).unwrap();
        let mut bytes = sig.to_bytes().to_vec();
        bytes.push(27 + rid.to_byte());
        assert_eq!(offerer_signed(&o.hash(), &me, &bytes, 1, &SEAPORT_1_6), Ok(true));
        let dear = listing_order(&me, &coll, &token, 6 * 1_000_000_000_000_000_000, Some((fee_to, 50)), 1_800_000_000, 1_800_086_400, [3; 32], [0; 32]);
        assert!(offerer_signed(&dear.hash(), &me, &bytes, 1, &SEAPORT_1_6).is_err());
        assert!(offerer_signed(&o.hash(), &me, &bytes, 8453, &SEAPORT_1_6).is_err(), "another chain's domain");
        let j: serde_json::Value = serde_json::from_str(&listing_json(&o, &o.hash(), "0xab")).unwrap();
        assert_eq!(j["parameters"]["totalOriginalConsiderationItems"], 2);
        assert_eq!(j["parameters"]["offer"][0]["identifierOrCriteria"], "9820");
        // no fee recipient: one consideration item
        assert_eq!(listing_order(&me, &coll, &token, 1, None, 0, 1, [0; 32], [0; 32]).consideration.len(), 1);
    }

    #[test]
    fn real_fills_decode_and_their_order_hash_is_the_one_seaport_emitted() {
        for f in [eth(), base()] {
            let o = decode(&f.input).unwrap();
            assert_eq!(o.offerer, f.offerer);
            assert_eq!(o.total(), Some(f.value), "the real fulfiller paid exactly the total");
            // counter 0 at both blocks (read with getCounter at the fork block)
            assert_eq!(o.components([0; 32]).hash(), f.order_hash, "chain {}", f.chain);
            // and the offerer really signed it: EIP-712 recovery over Seaport's domain on THIS chain
            assert_eq!(signed_by_offerer(&o, f.chain, &SEAPORT_1_6, [0; 32]), Ok(true), "chain {}", f.chain);
            // signed for chain 1, the same order is not the offerer's on chain 8453: the domain carries the chain id
            let other = if f.chain == 1 { 8453 } else { 1 };
            assert!(signed_by_offerer(&o, other, &SEAPORT_1_6, [0; 32]).unwrap_err().contains("not the offerer"));
        }
    }

    #[test]
    fn the_domain_separator_is_what_information_returned_on_each_chain() {
        // `cast call 0x…68F116a894984e2DB1123eB395 "information()"` on 2026-09-23
        assert_eq!(hex::encode(domain_separator(1, &SEAPORT_1_6)), "fce34bc6e1752c1117e5063116d25cad2fa2bdcf15ff2d2e275eece7dc31ba64");
        assert_eq!(hex::encode(domain_separator(8453, &SEAPORT_1_6)), "1965c52d779fea3b2c530bc1bbedc135c9671ade2fa4748307d92dbcdb47b2e0");
    }

    #[test]
    fn a_real_validated_restricted_order_hashes_to_its_event_hash() {
        // OrderValidated(bytes32 orderHash, OrderParameters) — a general order (three consideration items, OpenSea's zone)
        let v: serde_json::Value = serde_json::from_str(ETH_VALIDATED).unwrap();
        let d = hex::decode(v["data"].as_str().unwrap().trim_start_matches("0x")).unwrap();
        let w = |i: usize| -> Word { d[32 * i..32 * i + 32].try_into().unwrap() };
        let p = narrow(&w(1), 8, "").unwrap() as usize / 32; // the tuple's offset
        let pw = |i: usize| w(p + i);
        let offer_at = p + narrow(&pw(2), 8, "").unwrap() as usize / 32;
        let cons_at = p + narrow(&pw(3), 8, "").unwrap() as usize / 32;
        let offer = (0..narrow(&w(offer_at), 8, "").unwrap() as usize).map(|k| {
            let b = offer_at + 1 + 5 * k;
            OfferItem { item_type: w(b)[31], token: addr_of(&w(b + 1), "").unwrap(), identifier: w(b + 2), start: w(b + 3), end: w(b + 4) }
        }).collect();
        let consideration = (0..narrow(&w(cons_at), 8, "").unwrap() as usize).map(|k| {
            let b = cons_at + 1 + 6 * k;
            ConsiderationItem { item_type: w(b)[31], token: addr_of(&w(b + 1), "").unwrap(), identifier: w(b + 2), start: w(b + 3),
                end: w(b + 4), recipient: addr_of(&w(b + 5), "").unwrap() }
        }).collect();
        let c = OrderComponents { offerer: addr_of(&pw(0), "").unwrap(), zone: addr_of(&pw(1), "").unwrap(), offer, consideration,
            order_type: pw(4)[31], start_time: pw(5), end_time: pw(6), zone_hash: pw(7), salt: pw(8), conduit_key: pw(9),
            counter: word_u128(v["counter"].as_u64().unwrap() as u128) };
        assert_eq!(c.consideration.len(), 3);
        assert_eq!(c.hash(), w(0), "our EIP-712 hash is Seaport's");
    }

    #[test]
    fn the_real_order_passes_every_rule_and_our_calldata_is_our_own() {
        for f in [eth(), base()] {
            let o = decode(&f.input).unwrap();
            let got = verify(&o, &target(&o, f.value), &rules(), &facts(&f)).unwrap();
            assert_eq!(got.total_wei, f.value);
            // ours: the plain selector, the fulfiller conduit zeroed, no tracking tag — and it decodes to the same order
            assert_eq!(got.calldata[..4], FULFILL_BASIC_ORDER);
            assert_ne!(got.calldata[4..], f.input[4..]);
            let back = decode(&got.calldata).unwrap();
            assert_eq!(back, BasicOrder { fulfiller_conduit_key: [0; 32], ..o.clone() });
            assert_eq!(encode(&back), got.calldata);
            assert_eq!(got.calldata.len(), 4 + 32 * 20 + 64 * o.additional_recipients.len() + 32 + 96, "a 65-byte signature pads to 96");
            assert_eq!(f.input.len(), got.calldata.len() + 4, "the real fill carried a 4-byte tag we drop");
        }
    }

    #[test]
    fn every_tampered_order_is_refused_naming_its_rule() {
        let f = eth();
        let o = decode(&f.input).unwrap();
        let t = target(&o, f.value);
        let r = rules();
        let x = facts(&f);
        let refuse = |o: &BasicOrder, t: &Target, r: &Rules, x: &ChainFacts, why: &str| {
            let e = verify(o, t, r, x).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
        };
        let m = |f: &dyn Fn(&mut BasicOrder)| { let mut c = o.clone(); f(&mut c); c };
        let stranger = a("000000000000000000000000000000000000beef");
        refuse(&m(&|c| c.basic_order_type = 2), &t, &r, &x, "restricted");
        refuse(&m(&|c| c.basic_order_type = 4), &t, &r, &x, "only ETH for an ERC-721");
        refuse(&m(&|c| c.basic_order_type = 8), &t, &r, &x, "only ETH for an ERC-721");
        refuse(&m(&|c| c.consideration_token = stranger), &t, &r, &x, "not native ETH");
        refuse(&m(&|c| c.offer_token = stranger), &t, &r, &x, "wrong collection");
        refuse(&m(&|c| c.offer_identifier = word_u128(9_821)), &t, &r, &x, "wrong token");
        refuse(&m(&|c| c.offer_amount = 2), &t, &r, &x, "amount");
        refuse(&m(&|c| c.offerer = US), &t, &r, &x, "offerer");
        refuse(&m(&|c| c.additional_recipients[0].1 = stranger), &t, &r, &x, "unknown recipient");
        refuse(&m(&|c| c.additional_recipients.push((1, a(OPENSEA_FEE)))), &t, &r, &x, "tips");
        refuse(&m(&|c| { c.additional_recipients.push((1, stranger)); c.total_original_additional_recipients += 1; }), &t, &r, &x, "unknown recipient");
        refuse(&m(&|c| c.consideration_amount += 1), &t, &r, &x, "over cap");
        refuse(&o, &Target { cap_wei: f.value - 1, ..t.clone() }, &r, &x, "over cap");
        refuse(&m(&|c| c.additional_recipients[0].0 = u128::MAX), &t, &r, &x, "overflow");
        refuse(&m(&|c| { c.additional_recipients[0].0 = c.consideration_amount; }), &Target { cap_wei: u128::MAX, ..t.clone() }, &r, &x, "fees");
        refuse(&o, &t, &r, &ChainFacts { pin_timestamp: secs(&o.end_time) - 30, ..x.clone() }, "expired");
        refuse(&o, &t, &r, &ChainFacts { pin_timestamp: secs(&o.start_time) - 1, ..x.clone() }, "not started");
        // the royalty receiver the chain reports is a known recipient; the same address without that report is not
        let royalty = m(&|c| c.additional_recipients[0].1 = stranger);
        assert!(verify(&royalty, &t, &r, &ChainFacts { royalty_receiver: Some(stranger), ..x.clone() }).is_ok());
    }

    #[test]
    fn a_tampered_price_breaks_the_signature_even_under_the_cap() {
        // a relayer lowering the price (to stay under a cap) produces an order the offerer never signed
        let f = eth();
        let mut o = decode(&f.input).unwrap();
        o.consideration_amount -= 1;
        assert!(signed_by_offerer(&o, 1, &SEAPORT_1_6, [0; 32]).is_err());
        o.signature.truncate(10);
        assert_eq!(signed_by_offerer(&o, 1, &SEAPORT_1_6, [0; 32]), Ok(false), "not checkable here: left to the simulation");
    }

    #[test]
    fn malformed_calldata_is_refused() {
        let f = eth();
        assert!(decode(&[0xde, 0xad, 0xbe, 0xef]).unwrap_err().contains("not fulfillBasicOrder"));
        assert!(decode(&f.input[..200]).is_err());
        let mut bad = f.input.clone();
        bad[4 + 31] = 0x40; // the order's offset
        assert!(decode(&bad).unwrap_err().contains("canonical"));
        let mut dirty = f.input.clone();
        dirty[4 + 32 + 32 * 3] = 1; // a byte above the offerer's address
        assert!(decode(&dirty).unwrap_err().contains("dirty"));
    }

    #[test]
    fn views_decode() {
        // information() on Ethereum, as returned 2026-09-23
        let info = hex::decode(concat!(
            "0000000000000000000000000000000000000000000000000000000000000060",
            "fce34bc6e1752c1117e5063116d25cad2fa2bdcf15ff2d2e275eece7dc31ba64",
            "00000000000000000000000000000000f9490004c11cef243f5400493c00ad63",
            "0000000000000000000000000000000000000000000000000000000000000003",
            "312e360000000000000000000000000000000000000000000000000000000000")).unwrap();
        let (v, d) = decode_information(&info).unwrap();
        assert_eq!((v.as_str(), d), ("1.6", domain_separator(1, &SEAPORT_1_6)));
        let mut st = vec![0u8; 128];
        st[31] = 1; st[127] = 1; st[95] = 1;
        assert_eq!(decode_order_status(&st).unwrap(), (true, false, 1, 1));
        assert_eq!(call1(OWNER_OF, word_u128(9_820)).len(), 36);
        assert_eq!(secs(&[0xff; 32]), u64::MAX);
    }

    // ---- restricted orders through OpenSea's SignedZone (§12) -------------------------------------------------------------

    const ZONE_FILL: &str = include_str!("../tests/fixtures/seaport/base-zone-fill-51673875.json");
    const ZONE_FILL_2981: &str = include_str!("../tests/fixtures/seaport/base-zone-fill-51673570.json");

    struct ZoneFill { input: Vec<u8>, order_hash: Word, fulfiller: [u8; 20], fork_ts: u64, zone: [u8; 20], domain: Word,
        signer: [u8; 20], validator: [u8; 20], value: u128 }
    fn zone_fill() -> ZoneFill { zone_fill_of(ZONE_FILL) }
    fn zone_fill_of(src: &str) -> ZoneFill {
        let v: serde_json::Value = serde_json::from_str(src).unwrap();
        let h = |k: &str| hex::decode(v[k].as_str().unwrap().trim_start_matches("0x")).unwrap();
        ZoneFill { input: h("input"), order_hash: h("order_hash").try_into().unwrap(), fulfiller: h("fulfiller").try_into().unwrap(),
            fork_ts: v["fork_block_timestamp"].as_u64().unwrap(), zone: h("zone").try_into().unwrap(),
            domain: h("zone_domain_separator").try_into().unwrap(), signer: h("zone_active_signer").try_into().unwrap(),
            validator: h("transfer_validator").try_into().unwrap(),
            value: u128::from_str_radix(v["value"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap() }
    }
    fn zrules(z: &ZoneFill) -> ZoneRules { ZoneRules { zones: vec![z.zone], registries: vec![z.validator] } }
    /// The same order with the zone's data re-made for `fulfiller` and `expiration` (its signature is then not the zone
    /// signer's — the canister does not check it; the simulation at the pin does, and the fork run shows it).
    fn remade(o: &AdvancedOrder, fulfiller: [u8; 20], expiration: u64) -> AdvancedOrder {
        let mut x = o.extra_data.clone();
        x[1..21].copy_from_slice(&fulfiller);
        x[21..29].copy_from_slice(&expiration.to_be_bytes());
        AdvancedOrder { extra_data: x, ..o.clone() }
    }

    #[test]
    fn a_real_signedzone_fill_decodes_and_every_signature_in_it_is_what_we_think() {
        let z = zone_fill();
        let o = decode_advanced(&z.input).unwrap();
        let c = &o.order;
        assert_eq!((c.order_type, c.zone, o.numerator, o.denominator, o.criteria_resolvers), (2, z.zone, 1, 1, 0));
        assert_eq!(o.recipient, z.fulfiller);
        assert_eq!(c.consideration.iter().map(|i| narrow(&i.start, 16, "").unwrap()).sum::<u128>(), z.value, "the fulfiller paid the total");
        // our EIP-712 order hash is the one Seaport emitted
        assert_eq!(OrderComponents { counter: [0; 32], ..c.clone() }.hash(), z.order_hash);
        // the offerer's BULK signature (64 bytes, a 3-byte index, a 1-level proof) recovers to the offerer
        assert_eq!(o.signature.len(), 64 + 3 + 32);
        assert_eq!(offerer_signed(&z.order_hash, &c.offerer, &o.signature, 8453, &SEAPORT_1_6), Ok(true));
        assert!(offerer_signed(&z.order_hash, &c.offerer, &o.signature, 1, &SEAPORT_1_6).unwrap_err().contains("not the offerer"));
        // the zone's data: SIP-6 v0, the real fulfiller, ~5 minutes of life, substandard 8 naming the transfer validator
        let d = decode_zone_data(&o.extra_data).unwrap();
        assert_eq!((d.fulfiller, d.substandards.len()), (z.fulfiller, 1));
        assert_eq!(d.substandards[0], Substandard { id: 8, identifier: [0; 32], registry: Some(z.validator), operator: None });
        assert_eq!(d.expiration - 1_790_137_097, 281, "281 s after the block that mined it");
        // and the zone signer's signature recovers, under SIP-7's SignedOrder and the zone's domain, to the zone's active signer
        assert_eq!(recover(&zone_digest(&z.domain, &d, &z.order_hash), &d.signature), Some(z.signer));
    }

    #[test]
    fn our_advanced_calldata_is_the_real_one_with_our_recipient_and_no_conduit() {
        let z = zone_fill();
        let o = decode_advanced(&z.input).unwrap();
        // re-encoding for the real fulfiller reproduces the real calldata except the conduit key we zero and the tag we drop
        let mut want = z.input[..z.input.len() - 4].to_vec();
        want[4 + 64..4 + 96].fill(0);
        assert_eq!(encode_advanced(&o, &z.fulfiller), want);
        let ours = decode_advanced(&encode_advanced(&o, &US)).unwrap();
        assert_eq!(ours, AdvancedOrder { recipient: US, fulfiller_conduit_key: [0; 32], ..o });
    }

    #[test]
    fn a_signedzone_order_made_out_to_us_passes_and_every_tamper_is_refused() {
        let z = zone_fill();
        let real = decode_advanced(&z.input).unwrap();
        let o = remade(&real, US, z.fork_ts + 290);
        let offer = &o.order.offer[0];
        let t = Target { collection: offer.token, token_id: offer.identifier, cap_wei: z.value };
        let royalty = o.order.consideration[2].recipient;
        let x = ChainFacts { ours: US, pin_timestamp: z.fork_ts, royalty_receiver: Some(royalty) };
        let r = rules();
        let zr = zrules(&z);
        let now = z.fork_ts + 10;
        let (ok, zd) = verify_advanced(&o, &t, &r, &zr, &x, now).unwrap();
        assert_eq!((ok.total_wei, zd.fulfiller), (z.value, US));
        let sent = decode_advanced(&ok.calldata).unwrap();
        assert_eq!((sent.recipient, sent.fulfiller_conduit_key, sent.criteria_resolvers), (US, [0; 32], 0), "the NFT comes to us");
        let refuse = |o: &AdvancedOrder, t: &Target, zr: &ZoneRules, x: &ChainFacts, now: u64, why: &str| {
            let e = verify_advanced(o, t, &r, zr, x, now).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
        };
        let m = |f: &dyn Fn(&mut AdvancedOrder)| { let mut c = o.clone(); f(&mut c); c };
        let stranger = a("000000000000000000000000000000000000beef");
        // the zone's data
        refuse(&real, &t, &zr, &x, now, "not us");
        refuse(&o, &t, &zr, &x, z.fork_ts + 290 - 60, "expires");
        refuse(&o, &t, &zr, &ChainFacts { pin_timestamp: z.fork_ts + 240, ..x.clone() }, now, "expires");
        refuse(&m(&|c| c.extra_data[0] = 1), &t, &zr, &x, now, "SIP-6 version");
        refuse(&m(&|c| c.extra_data.truncate(90)), &t, &zr, &x, now, "at least 93");
        refuse(&m(&|c| c.extra_data[93] = 5), &t, &zr, &x, now, "substandard 5");
        refuse(&m(&|c| c.extra_data.truncate(120)), &t, &zr, &x, now, "cut short");
        refuse(&m(&|c| c.extra_data[94 + 31] = 1), &t, &zr, &x, now, "identifier");
        refuse(&o, &t, &ZoneRules { registries: vec![], ..zr.clone() }, &x, now, "registry");
        // the order's shape
        refuse(&o, &t, &ZoneRules { zones: vec![], ..zr.clone() }, &x, now, "allow-listed SignedZone");
        refuse(&m(&|c| c.order.order_type = 0), &t, &zr, &x, now, "restricted order");
        refuse(&m(&|c| c.numerator = 2), &t, &zr, &x, now, "fraction");
        refuse(&m(&|c| { c.numerator = 1; c.denominator = 2 }), &t, &zr, &x, now, "fraction");
        refuse(&m(&|c| c.criteria_resolvers = 1), &t, &zr, &x, now, "criteria");
        refuse(&m(&|c| { let i = c.order.offer[0].clone(); c.order.offer.push(i) }), &t, &zr, &x, now, "offer items");
        refuse(&m(&|c| c.order.offer[0].item_type = 3), &t, &zr, &x, now, "one ERC-721");
        refuse(&m(&|c| c.order.consideration[1].item_type = 1), &t, &zr, &x, now, "native ETH");
        refuse(&m(&|c| c.order.consideration[1].end = word_u128(1)), &t, &zr, &x, now, "changes over time");
        refuse(&m(&|c| c.order.consideration[0].recipient = stranger), &t, &zr, &x, now, "not paid to the offerer");
        // and every §6 rule still applies
        refuse(&m(&|c| c.order.consideration[1].recipient = stranger), &t, &zr, &x, now, "unknown recipient");
        refuse(&o, &t, &zr, &ChainFacts { royalty_receiver: None, ..x.clone() }, now, "unknown recipient");
        refuse(&m(&|c| c.total_original_consideration_items = 2), &t, &zr, &x, now, "tips");
        refuse(&o, &Target { cap_wei: z.value - 1, ..t.clone() }, &zr, &x, now, "over cap");
        refuse(&o, &Target { token_id: word_u128(4_712), ..t.clone() }, &zr, &x, now, "wrong token");
        refuse(&m(&|c| c.order.offerer = US), &t, &zr, &x, now, "offerer");
    }

    #[test]
    fn a_second_real_fill_whose_creator_fee_is_the_erc2981_receiver_passes_and_the_first_whose_is_not_is_refused() {
        // 0xb22b9f9e (block 51673570): 7.8% to 0xca3f…f3aa, which the collection's royaltyInfo names; the fork run bought this one
        let z = zone_fill_of(ZONE_FILL_2981);
        let real = decode_advanced(&z.input).unwrap();
        assert_eq!(OrderComponents { counter: [0; 32], ..real.order.clone() }.hash(), z.order_hash);
        assert_eq!(offerer_signed(&z.order_hash, &real.order.offerer, &real.signature, 8453, &SEAPORT_1_6), Ok(true));
        let d = decode_zone_data(&real.extra_data).unwrap();
        assert_eq!(recover(&zone_digest(&z.domain, &d, &z.order_hash), &d.signature), Some(z.signer), "the same zone signer");
        assert_eq!(d.substandards[0].registry, Some(z.validator), "another transfer validator than the first fill's");
        let o = remade(&real, US, z.fork_ts + 600);
        let offer = &o.order.offer[0];
        let t = Target { collection: offer.token, token_id: offer.identifier, cap_wei: z.value };
        let royalty = a("ca3f48726774fe6b6f437cc4d0577333f685f3aa");
        let x = ChainFacts { ours: US, pin_timestamp: z.fork_ts, royalty_receiver: Some(royalty) };
        assert_eq!(verify_advanced(&o, &t, &rules(), &zrules(&z), &x, z.fork_ts).unwrap().0.total_wei, z.value);
        // 0xc812f2c3's collection answers royaltyInfo with zero: its 10% creator payout is OpenSea's off-chain setting, so rule 6
        // has nothing on chain to check it against and refuses — the fork run shows the same refusal
        let first = zone_fill();
        let f = remade(&decode_advanced(&first.input).unwrap(), US, first.fork_ts + 600);
        let ft = Target { collection: f.order.offer[0].token, token_id: f.order.offer[0].identifier, cap_wei: first.value };
        let e = verify_advanced(&f, &ft, &rules(), &zrules(&first), &ChainFacts { ours: US, pin_timestamp: first.fork_ts, royalty_receiver: None },
            first.fork_ts).unwrap_err();
        assert!(e.contains("unknown recipient") && e.contains("3918a9b4"), "{e}");
    }
}
