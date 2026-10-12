//! The canister's Solana identity (docs/v9/solana-migration.md §2.4 "Keys", spike Q3 §1; canister-only design §1.1, spike S3): ONE
//! threshold **Ed25519** key (`schnorr_public_key` / `sign_with_schnorr`, algorithm `ed25519`, ≈ 26.15 B cycles a signature), and every
//! Solana address of the canister one derivation path of it ([`KeyPath`]). The secp256k1 `"evm"` key stays for Ethereum and the BIP340
//! paths for Bitcoin; nobody holds any of them.
//!
//! The key name is the install's `key_name`, under the same guard as the other two keys (`canister::validate`: naming `key_1`
//! needs `accept_production_key_name`). dfx 0.32's local replica holds `schnorr:Ed25519:key_1` (Q3 §1), so the whole send line
//! runs locally under the production key's NAME, as the BIP340 key always has; the flag records intent, it cannot tell networks
//! apart.
//!
//! ## The derivation-path format (PERMANENT: changing it changes every address)
//!
//! Spike S3 pinned the multi-element form with fixed-width little-endian integers. Each element is one HKDF step of the IC's Ed25519
//! derivation (the canister id is the implicit first element), so every `[b"sol", ..]` key is a child of `K_main`:
//!
//! | [`KeyPath`] | Elements | Role |
//! |---|---|---|
//! | `Main` | `[b"sol"]` | `K_main`: fee payer and nonce authority of every send line; custodian (exists since phase A) |
//! | `Relay` | `[b"sol-relay"]` | the dedicated Relay deposit key (audit A5-03; a sibling of `sol`, never a fee payer of another key's money) |
//! | `Coll(i)` | `[b"sol", b"coll", i: u32 LE]` | `K_coll[i]`: module address `M[i]` AND mint authority of wX[i] |
//! | `Claimer` | `[b"sol", b"claimer"]` | `K_fc`: `fee_claimer` / `leftover_receiver` of every DBC config, owner of partner LP NFTs |
//! | `Coin(mint)` | `[b"sol", b"coin", mint: 32 bytes]` | `K_coin[c]`: owner of coin c's backing vault ATA |
//! | `Desk(i)` | `[b"sol", b"desk", i: u32 LE]` | `K_desk[i]`: collection i's wX/SOL pool position and arb inventory (the pool desk) |
//! | `Treasury` | `[b"sol", b"treasury"]` | `K_tre` |
//! | `Intent(n)` | `[b"sol", b"intent", n: u64 LE]` | `K_int[n]`: the deposit address of user intent n |
//! | `PositionNft(n)` | `[b"sol", b"nft", n: u64 LE]` | the mint of DAMM v2 position NFT n (S1 §5.3: it must sign its own creation; the migration's two) |
//! | `Config(i)` | `[b"sol", b"config", i: u32 LE]` | the address of collection i's DBC config account (W6: `create_config` needs the new account to sign) |
//!
//! Integer widths and the tags are fixed here and nowhere else: [`KeyPath::derivation`]. `tests/solkey_paths.rs` pins the exact
//! bytes and, with `ic-ed25519` 0.6, that every child derives from `K_main`'s key and chain code. The offline helper
//! (`cargo run --example sol_keys`) computes every address before the canister runs (`derive_mainnet_key(Key1, canister id, path)`),
//! for the ceremony's launch file.
//!
//! ## Signing
//!
//! Every signature goes through [`sign_with_keys`] (or [`sign_message`], its single-key form): refused, BEFORE any cycles are spent
//! on signing, when a signer of the compiled message is not a key of this canister the caller named, when the Relay key would sign
//! with anyone else (A5-03), when the fee payer is not `K_main` (fees never come out of a backing address, §1.1), or when the signed
//! transaction would exceed 1,232 bytes (Q3 §3.3). The design's change to the old rule ("this key is the ONLY signer") is exactly
//! this: every signer is one of ours, signed by its own path, in the compiled message's signer order (the fee payer first, then the
//! other signers as the compiler sorts them: by key bytes, not by role; S3). The signatures are asked IN PARALLEL (one signature's
//! latency for any number; S3 §4 row 6). A "request queue is full" reject (20 per key on the local replica, S3 row 9) is
//! [`QUEUE_FULL`]: nothing is saved and the caller retries at its next tick (the rejected calls are refunded).
use std::cell::RefCell;
use std::collections::BTreeMap;

use candid::CandidType;
use ic_cdk::management_canister::{schnorr_public_key, sign_with_schnorr, SchnorrAlgorithm, SchnorrKeyId, SchnorrPublicKeyArgs, SignWithSchnorrArgs};
use serde::Deserialize;

use crate::soltx::{self, Message, Pubkey};

/// The Solana key's derivation path (`K_main`, [`KeyPath::Main`]).
pub const DERIVATION: &[u8] = b"sol";
/// Audit A5-03: the DEDICATED Relay deposit key's derivation path. Relay's depository is upgradeable by a third party and is invoked with
/// its depositor as a signer: only this key, funded with exactly the deposit plus its fee just before it (and swept back after anything
/// but a successful deposit), is ever handed to it. It is never the fee payer of any other transaction, never a nonce authority and never
/// rook_core's custodian, so an upgraded depository can take at most the one deposit it is given.
pub const RELAY_DERIVATION: &[u8] = b"sol-relay";

/// The prefix of a signing failure that is the threshold key's request queue being full (S3: 20 requests per key on the local
/// replica; refunded): retry at the next tick, nothing was saved.
pub const QUEUE_FULL: &str = "signing queue full (retry next tick)";

/// A key of this canister, by role (canister-only design §1.1). The derivation encoding ([`KeyPath::derivation`]) is permanent.
#[derive(CandidType, Deserialize, serde::Serialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyPath {
    /// `K_main`, `[b"sol"]`.
    Main,
    /// The Relay deposit key, `[b"sol-relay"]`.
    Relay,
    /// `K_coll[i]`: module address `M[i]` and mint authority of wX[i].
    Coll(u32),
    /// `K_fc`: the fee claimer of every collection DBC config.
    Claimer,
    /// `K_coin[c]`, by the coin's mint.
    Coin(Pubkey),
    /// `K_desk[i]`: the pool desk of collection i.
    Desk(u32),
    /// `K_tre`.
    Treasury,
    /// `K_int[n]`: user intent n's deposit address.
    Intent(u64),
    /// A DAMM v2 position NFT mint (the two a migration creates).
    PositionNft(u64),
    /// W6: collection i's DBC config account (a keypair-style address that signs its own creation, once).
    Config(u32),
}

impl KeyPath {
    /// The derivation path's elements (PERMANENT; see the module header).
    pub fn derivation(&self) -> Vec<Vec<u8>> {
        let sol = DERIVATION.to_vec();
        match self {
            KeyPath::Main => vec![sol],
            KeyPath::Relay => vec![RELAY_DERIVATION.to_vec()],
            KeyPath::Coll(i) => vec![sol, b"coll".to_vec(), i.to_le_bytes().to_vec()],
            KeyPath::Claimer => vec![sol, b"claimer".to_vec()],
            KeyPath::Coin(mint) => vec![sol, b"coin".to_vec(), mint.to_vec()],
            KeyPath::Desk(i) => vec![sol, b"desk".to_vec(), i.to_le_bytes().to_vec()],
            KeyPath::Treasury => vec![sol, b"treasury".to_vec()],
            KeyPath::Intent(n) => vec![sol, b"intent".to_vec(), n.to_le_bytes().to_vec()],
            KeyPath::PositionNft(n) => vec![sol, b"nft".to_vec(), n.to_le_bytes().to_vec()],
            KeyPath::Config(i) => vec![sol, b"config".to_vec(), i.to_le_bytes().to_vec()],
        }
    }

    /// The text form the operator methods and the offline helper take: `main`, `relay`, `coll:<i>`, `claimer`, `coin:<mint base58>`,
    /// `desk:<i>`, `treasury`, `intent:<n>`, `nft:<n>`.
    pub fn parse(s: &str) -> Result<KeyPath, String> {
        let (tag, arg) = match s.trim().split_once(':') {
            Some((t, a)) => (t, Some(a)),
            None => (s.trim(), None),
        };
        let int = |what: &str| -> Result<u64, String> {
            arg.ok_or(format!("{tag} needs :<{what}>"))?.parse::<u64>().map_err(|e| format!("{tag}: {e}"))
        };
        let small = |what: &str| -> Result<u32, String> { u32::try_from(int(what)?).map_err(|_| format!("{tag}: over u32")) };
        let none = |k: KeyPath| if arg.is_some() { Err(format!("{tag} takes no argument")) } else { Ok(k) };
        match tag {
            "main" => none(KeyPath::Main),
            "relay" => none(KeyPath::Relay),
            "claimer" => none(KeyPath::Claimer),
            "treasury" => none(KeyPath::Treasury),
            "coll" => Ok(KeyPath::Coll(small("collection index")?)),
            "desk" => Ok(KeyPath::Desk(small("collection index")?)),
            "intent" => Ok(KeyPath::Intent(int("intent number")?)),
            "nft" => Ok(KeyPath::PositionNft(int("position number")?)),
            "config" => Ok(KeyPath::Config(small("collection index")?)),
            "coin" => Ok(KeyPath::Coin(soltx::pubkey(arg.ok_or("coin needs :<mint base58>")?)?)),
            _ => Err(format!("unknown key path {s:?} (main, relay, coll:<i>, claimer, coin:<mint>, desk:<i>, treasury, intent:<n>, nft:<n>, config:<i>)")),
        }
    }

    /// The text form [`KeyPath::parse`] reads.
    pub fn text(&self) -> String {
        match self {
            KeyPath::Main => "main".into(),
            KeyPath::Relay => "relay".into(),
            KeyPath::Coll(i) => format!("coll:{i}"),
            KeyPath::Claimer => "claimer".into(),
            KeyPath::Coin(m) => format!("coin:{}", soltx::b58_encode(m)),
            KeyPath::Desk(i) => format!("desk:{i}"),
            KeyPath::Treasury => "treasury".into(),
            KeyPath::Intent(n) => format!("intent:{n}"),
            KeyPath::PositionNft(n) => format!("nft:{n}"),
            KeyPath::Config(i) => format!("config:{i}"),
        }
    }
}

thread_local! {
    /// Every derived key asked so far (a key never changes for a given canister id, key name and path: S3 row 8), per wasm instance.
    static KEYS: RefCell<BTreeMap<KeyPath, Pubkey>> = const { RefCell::new(BTreeMap::new()) };
}

fn key_id() -> SchnorrKeyId {
    SchnorrKeyId { algorithm: SchnorrAlgorithm::Ed25519, name: crate::canister::cfg().key_name }
}

/// The public key of one of this canister's paths (`schnorr_public_key(ed25519)`, cached).
pub async fn derive_key(path: &KeyPath) -> Result<Pubkey, String> {
    if let Some(k) = KEYS.with(|k| k.borrow().get(path).copied()) {
        return Ok(k);
    }
    let r = schnorr_public_key(&SchnorrPublicKeyArgs { canister_id: None, derivation_path: path.derivation(), key_id: key_id() })
        .await
        .map_err(|e| format!("schnorr_public_key(ed25519, {}): {e:?}", path.text()))?;
    let k = pubkey_of(&r.public_key)?;
    KEYS.with(|c| c.borrow_mut().insert(*path, k));
    Ok(k)
}

/// The canister's Solana public key (32 bytes), `K_main`.
pub async fn sol_pubkey() -> Result<Pubkey, String> {
    derive_key(&KeyPath::Main).await
}

/// The dedicated Relay deposit key (audit A5-03), on [`RELAY_DERIVATION`].
pub async fn sol_relay_pubkey() -> Result<Pubkey, String> {
    derive_key(&KeyPath::Relay).await
}

/// An Ed25519 public key as the management canister returns it: exactly 32 bytes.
pub fn pubkey_of(raw: &[u8]) -> Result<Pubkey, String> {
    raw.try_into().map_err(|_| format!("an ed25519 public key of {} bytes, not 32", raw.len()))
}

/// The canister's Solana address (base58): fee payer, custodian, poster and nonce authority of every Solana line.
#[ic_cdk::update]
pub async fn sol_address() -> Result<String, String> {
    crate::canister::only_operator();
    Ok(soltx::b58_encode(&sol_pubkey().await?))
}

/// The address of any of the canister's paths (`KeyPath::parse`'s text form), for the operator and the ceremony, which compares it
/// with the offline derivation (`examples/sol_keys.rs`) before pinning it in the launch file.
#[ic_cdk::update]
pub async fn sol_address_of(path: String) -> Result<String, String> {
    crate::canister::only_operator();
    Ok(soltx::b58_encode(&derive_key(&KeyPath::parse(&path)?).await?))
}

/// Who signs a Solana message: the threshold key on the IC, a fake in the tests.
#[allow(async_fn_in_trait)]
pub trait SolSigner {
    fn me(&self) -> Pubkey;
    /// The Ed25519 signature of the message bytes by `K_main`. Called only through [`sign_with_keys`].
    async fn sign_bytes(&mut self, message: &[u8]) -> Result<[u8; 64], String>;
    /// The dedicated Relay deposit key (audit A5-03), if this signer holds one.
    fn relay(&self) -> Option<Pubkey> {
        None
    }
    /// The Ed25519 signature of the message bytes by the Relay deposit key. Called only through [`sign_with_keys`].
    async fn sign_relay_bytes(&mut self, _message: &[u8]) -> Result<[u8; 64], String> {
        Err("this signer holds no Relay deposit key".into())
    }
    /// The public key of `path` (no signature: free of signing cycles).
    async fn key(&mut self, path: &KeyPath) -> Result<Pubkey, String> {
        match path {
            KeyPath::Main => Ok(self.me()),
            KeyPath::Relay => self.relay().ok_or_else(|| "this signer holds no Relay deposit key".into()),
            p => Err(format!("this signer holds no derived key {}", p.text())),
        }
    }
    /// One signature of `message` per path, in the order given (the IC: all asked at once). Called only through [`sign_with_keys`].
    async fn sign_paths(&mut self, paths: &[KeyPath], message: &[u8]) -> Vec<Result<[u8; 64], String>> {
        let mut out = Vec::with_capacity(paths.len());
        for p in paths {
            out.push(match p {
                KeyPath::Main => self.sign_bytes(message).await,
                KeyPath::Relay => self.sign_relay_bytes(message).await,
                p => Err(format!("this signer holds no derived key {}", p.text())),
            });
        }
        out
    }
}

/// The guard every signature passes (pure, the single-key form kept for phase A's callers): this key is the message's one and only
/// signer, and the signed transaction fits.
pub fn signable(msg: &Message, me: &Pubkey) -> Result<(), String> {
    if msg.signers.as_slice() != [*me] {
        return Err(format!("refused: the message needs {} signature(s) ({}), and this canister signs alone as {}", msg.signers.len(),
            msg.signers.iter().map(|k| soltx::b58_encode(k)).collect::<Vec<_>>().join(", "), soltx::b58_encode(me)));
    }
    soltx::check_size(soltx::wire_size(msg))
}

/// The multi-key guard (pure): each signer of `msg`, in the compiled message's order, is one of `held` (the keys of this canister the
/// caller named, `K_main` among them); the fee payer is `K_main` (fees never come from a backing address), or the Relay key signing
/// ALONE (A5-03: it never signs with anyone else); and the signed transaction fits in 1,232 bytes. Returns the path of each signer, in
/// the message's signer order (fee payer first, the rest as the compiler sorted them).
pub fn signer_paths(msg: &Message, held: &[(KeyPath, Pubkey)]) -> Result<Vec<KeyPath>, String> {
    if msg.signers.is_empty() {
        return Err("refused: a message with no signer".into());
    }
    let mut paths = Vec::with_capacity(msg.signers.len());
    for k in &msg.signers {
        match held.iter().find(|(_, h)| h == k) {
            Some((p, _)) => paths.push(*p),
            None => {
                return Err(format!("refused: signer {} of the message is not a key of this canister named for it ({}); nothing was signed",
                    soltx::b58_encode(k), held.iter().map(|(p, _)| p.text()).collect::<Vec<_>>().join(", ")))
            }
        }
    }
    if paths.contains(&KeyPath::Relay) && paths.len() != 1 {
        return Err("refused: the Relay deposit key signs alone, never with another key (audit A5-03)".into());
    }
    if paths[0] != KeyPath::Main && paths[0] != KeyPath::Relay {
        return Err(format!("refused: the fee payer is {} ({}): every transaction's fee is paid by K_main", paths[0].text(),
            soltx::b58_encode(&msg.signers[0])));
    }
    soltx::check_size(soltx::wire_size(msg))?;
    Ok(paths)
}

/// The keys a message may be signed by: `K_main`, the Relay key, and `extra`'s, derived (no signing cycles).
pub async fn held_keys<S: SolSigner>(s: &mut S, extra: &[KeyPath]) -> Result<Vec<(KeyPath, Pubkey)>, String> {
    let mut held = vec![(KeyPath::Main, s.me())];
    if let Some(r) = s.relay() {
        held.push((KeyPath::Relay, r));
    }
    for p in extra {
        if !held.iter().any(|(q, _)| q == p) {
            let k = s.key(p).await?;
            held.push((*p, k));
        }
    }
    Ok(held)
}

/// A signing failure is the key's request queue being full (S3 row 9: "request queue for key … is full", reject code 4).
pub fn is_queue_full(e: &str) -> bool {
    e.starts_with(QUEUE_FULL) || (e.contains("queue") && e.contains("full"))
}

/// Sign a message the caller checked with [`signer_paths`] (its `paths`): every signature asked at once; ALL must come back, or
/// nothing is returned (a full queue is [`QUEUE_FULL`]: retry next tick). Returns the wire transaction and every signature in order.
pub async fn sign_paths_checked<S: SolSigner>(s: &mut S, msg: &Message, paths: &[KeyPath]) -> Result<(Vec<u8>, Vec<[u8; 64]>), String> {
    let got = s.sign_paths(paths, &msg.bytes).await;
    if got.len() != paths.len() {
        return Err(format!("{} signatures for {} signers", got.len(), paths.len()));
    }
    let mut sigs = Vec::with_capacity(got.len());
    let mut errs = Vec::new();
    for (p, r) in paths.iter().zip(got) {
        match r {
            Ok(x) => sigs.push(x),
            Err(e) => errs.push((p.text(), e)),
        }
    }
    if !errs.is_empty() {
        let full = errs.iter().any(|(_, e)| is_queue_full(e));
        let what = errs.iter().map(|(p, e)| format!("{p}: {e}")).collect::<Vec<_>>().join("; ");
        return Err(if full { format!("{QUEUE_FULL}: {what}; nothing was saved") } else { format!("signing failed ({what}); nothing was saved") });
    }
    Ok((soltx::assemble(msg, &sigs)?, sigs))
}

/// Sign a compiled message with every key it needs, each a key of this canister named in `extra` (or `K_main` / the Relay key): the
/// guard ([`signer_paths`]) first, then every signature at once. Returns the wire transaction and the first signature (the
/// transaction's id).
pub async fn sign_with_keys<S: SolSigner>(s: &mut S, msg: &Message, extra: &[KeyPath]) -> Result<(Vec<u8>, [u8; 64]), String> {
    let held = held_keys(s, extra).await?;
    let paths = signer_paths(msg, &held)?;
    let (tx, sigs) = sign_paths_checked(s, msg, &paths).await?;
    Ok((tx, sigs[0]))
}

/// `sign_sol`: a message whose one signer is the canister's key, or the dedicated Relay deposit key (audit A5-03): each signs alone,
/// never both. Returns the wire transaction and its signature.
pub async fn sign_message<S: SolSigner>(s: &mut S, msg: &Message) -> Result<(Vec<u8>, [u8; 64]), String> {
    if msg.signers.len() != 1 {
        let me = s.relay().filter(|r| msg.signers.first() == Some(r)).unwrap_or(s.me());
        signable(msg, &me)?;
    }
    sign_with_keys(s, msg, &[]).await
}

/// The threshold key: `K_main`, the Relay key and every derived path.
pub struct IcSigner {
    me: Pubkey,
    relay: Pubkey,
}

impl IcSigner {
    pub async fn new() -> Result<IcSigner, String> {
        Ok(IcSigner { me: sol_pubkey().await?, relay: sol_relay_pubkey().await? })
    }
}

async fn schnorr_sign(path: &KeyPath, message: &[u8]) -> Result<[u8; 64], String> {
    let arg = SignWithSchnorrArgs { message: message.to_vec(), derivation_path: path.derivation(), key_id: key_id(), aux: None };
    let r = sign_with_schnorr(&arg).await.map_err(|e| {
        let e = format!("sign_with_schnorr(ed25519, {}): {e:?}", path.text());
        if is_queue_full(&e) { format!("{QUEUE_FULL}: {e}") } else { e }
    })?;
    r.signature.as_slice().try_into().map_err(|_| format!("an ed25519 signature of {} bytes, not 64", r.signature.len()))
}

impl SolSigner for IcSigner {
    fn me(&self) -> Pubkey {
        self.me
    }
    async fn sign_bytes(&mut self, message: &[u8]) -> Result<[u8; 64], String> {
        schnorr_sign(&KeyPath::Main, message).await
    }
    fn relay(&self) -> Option<Pubkey> {
        Some(self.relay)
    }
    async fn sign_relay_bytes(&mut self, message: &[u8]) -> Result<[u8; 64], String> {
        schnorr_sign(&KeyPath::Relay, message).await
    }
    async fn key(&mut self, path: &KeyPath) -> Result<Pubkey, String> {
        derive_key(path).await
    }
    /// S3 §4 row 6: every signature at once (`join_all`): one signature's latency, the same cycles.
    async fn sign_paths(&mut self, paths: &[KeyPath], message: &[u8]) -> Vec<Result<[u8; 64], String>> {
        futures::future::join_all(paths.iter().map(|p| schnorr_sign(p, message))).await
    }
}
