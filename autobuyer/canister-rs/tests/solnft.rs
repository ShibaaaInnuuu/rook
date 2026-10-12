//! Solana NFTs held by the canister (docs/v9/solana-nfts-canister.md): the decoders, membership and custody, the asks and the
//! judge, the delivery judge and the floor, on real mainnet accounts (fixtures recorded by
//! `autobuyer/solnft-svm/record_fixtures.py`; the same purchases and deliveries are EXECUTED in LiteSVM by `autobuyer/solnft-svm`).
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

use base64::Engine;
use etch_autobuyer::solnftbuy::{self as buy, Ask, AskDetail, Candidate, Expect, Venue};
use etch_autobuyer::solnftfloor::{self as floor, FloorParams};
use etch_autobuyer::solnftsend as send;
use etch_autobuyer::solnft::{self, Accounts, Edition, Member, NftError, Standard, TokenState};
use etch_autobuyer::solrpc::{self, Account, Kind, Refused, Req, SolRpc};
use etch_autobuyer::soltx::{self, key, Pubkey};
use serde_json::{json, Value};

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

struct Fx {
    unix_time: i64,
    slot: u64,
    facts: Value,
    a: Accounts,
}

impl Fx {
    fn k(&self, f: &str) -> Pubkey {
        soltx::pubkey(self.facts[f].as_str().unwrap_or_else(|| panic!("fact {f}"))).unwrap()
    }
    fn data(&mut self, k: &Pubkey) -> &mut Vec<u8> {
        &mut self.a.get_mut(k).unwrap().as_mut().unwrap().data
    }
}

fn fx(name: &str) -> Fx {
    let p = format!("{}/tests/fixtures/solnft/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let j: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    let mut a = Accounts::new();
    for x in j["accounts"].as_array().unwrap() {
        a.insert(soltx::pubkey(x["pubkey"].as_str().unwrap()).unwrap(), Some(Account {
            lamports: x["lamports"].as_u64().unwrap(),
            owner: soltx::pubkey(x["owner"].as_str().unwrap()).unwrap(),
            executable: x["executable"].as_bool().unwrap(),
            data: base64::engine::general_purpose::STANDARD.decode(x["data"].as_str().unwrap()).unwrap(),
        }));
    }
    for x in j["absent"].as_array().unwrap() {
        a.insert(soltx::pubkey(x.as_str().unwrap()).unwrap(), None);
    }
    Fx { unix_time: j["unix_time"].as_i64().unwrap(), slot: j["slot"].as_u64().unwrap(), facts: j["facts"].clone(), a }
}

const SOL: u64 = 1_000_000_000;
const MAD_LADS: &str = "J1S9H3QjnRtBbbuD4HjPV6RpRhwuk4zKbxsnCHuTgh9w";
const ME: Pubkey = [0x11; 32];

fn member(f: &Fx, std: Standard) -> Member {
    solnft::is_member(&f.k("asset"), &f.k("collection"), std, &f.a).unwrap()
}

// ---- decoders and membership ------------------------------------------------------------------------------------------------------------

#[test]
fn decodes_a_real_pnft() {
    let f = fx("tensor_pnft_mad_lads");
    let mint = f.k("asset");
    let md = solnft::decode_metadata(&f.a[&solnft::metadata_address(&mint)].as_ref().unwrap().data).unwrap();
    assert_eq!(md.mint, mint);
    assert!(md.name.starts_with("Mad Lads #"), "{}", md.name);
    assert_eq!(md.seller_fee_bps, 420);
    assert_eq!(md.token_standard, Some(solnft::TS_PROGRAMMABLE_NON_FUNGIBLE));
    assert_eq!(md.collection, Some((true, key(MAD_LADS))));
    assert_eq!(md.programmable, Some(Some(key(solnft::FOUNDATION_RULE_SET))));
    assert!(!md.is_collection);
    let ed = solnft::decode_edition(&f.a[&solnft::edition_address(&mint)].as_ref().unwrap().data).unwrap();
    assert!(matches!(ed, Edition::Master { supply: 0, max_supply: Some(0) }), "{ed:?}");
    let list_ta = soltx::associated_token_address(&f.k("list_state"), &mint);
    let rec = solnft::decode_token_record(&f.a[&solnft::token_record_address(&mint, &list_ta)].as_ref().unwrap().data).unwrap();
    assert_eq!(rec.state, TokenState::Unlocked);
    let ta = solnft::decode_token_account(&f.a[&list_ta].as_ref().unwrap().data).unwrap();
    assert_eq!((ta.mint, ta.owner, ta.amount, ta.state), (mint, f.k("list_state"), 1, 2), "a pNFT's token account is frozen");
}

#[test]
fn members_of_every_standard() {
    let m = member(&fx("tensor_pnft_mad_lads"), Standard::Programmable);
    assert_eq!((m.standard, m.royalty_bps, m.rule_set), (Standard::Programmable, 420, Some(key(solnft::FOUNDATION_RULE_SET))));
    assert!(!m.creators.is_empty());
    // a legacy member (Famous Fox Federation: no token standard at all), under a Legacy listing
    let m = member(&fx("tensor_legacy_fff"), Standard::Legacy);
    assert_eq!((m.standard, m.rule_set), (Standard::Legacy, None));
    // a Token Metadata listing takes either kind: the pNFT under a "Legacy" listing is still recognised as programmable
    let m = member(&fx("tensor_pnft_mad_lads"), Standard::Legacy);
    assert_eq!(m.standard, Standard::Programmable);
    // the Compatibility rule set (SMB Gen3)
    let m = member(&fx("mmm_pnft_smb_gen3"), Standard::Programmable);
    assert_eq!(m.rule_set, Some(key(solnft::COMPATIBILITY_RULE_SET)));
    // Core: OTC Desks (a Royalties plugin only), escrowed by its Tensor listing
    let f = fx("tensor_core_otc");
    let m = member(&f, Standard::Core);
    assert_eq!((m.standard, m.royalty_bps, m.core_owner), (Standard::Core, 500, Some(f.k("list_state"))));
}

#[test]
fn not_members() {
    let mut f = fx("tensor_pnft_mad_lads");
    let (mint, coll) = (f.k("asset"), f.k("collection"));
    let other = key("BUjZjAS2vbbb65g7Z1Ca9ZRVYoJscURG5L3AkVvHP9ac");
    assert!(matches!(solnft::is_member(&mint, &other, Standard::Programmable, &f.a), Err(NftError::NotInCollection(_))));
    // a Core listing does not take a Token Metadata item
    assert!(matches!(solnft::is_member(&mint, &coll, Standard::Core, &f.a), Err(NftError::NotAnNft(_))));
    // unverified collection: flip the `verified` byte (it sits right before the collection key)
    let md_key = solnft::metadata_address(&mint);
    let d = f.data(&md_key);
    let at = d.windows(32).position(|w| w == key(MAD_LADS)).unwrap() - 1;
    d[at] = 0;
    let e = solnft::is_member(&mint, &coll, Standard::Programmable, &f.a).unwrap_err();
    assert!(matches!(&e, NftError::NotInCollection(s) if s.contains("not verified")), "{e}");
    f.data(&md_key)[at] = 1;
    // a print edition: the edition account says EditionV1
    let ed = solnft::edition_address(&mint);
    f.data(&ed)[0] = 1;
    f.data(&ed).resize(41, 0);
    assert!(matches!(solnft::is_member(&mint, &coll, Standard::Programmable, &f.a), Err(NftError::NotAnNft(s)) if s.contains("print")));
    let mut f = fx("tensor_pnft_mad_lads");
    // Token-2022, and a supply of 2
    f.a.get_mut(&mint).unwrap().as_mut().unwrap().owner = key(solnft::TOKEN_2022);
    assert!(matches!(solnft::is_member(&mint, &coll, Standard::Programmable, &f.a), Err(NftError::NotAnNft(s)) if s.contains("Token-2022")));
    let mut f = fx("tensor_pnft_mad_lads");
    f.data(&mint)[36] = 2;
    assert!(matches!(solnft::is_member(&mint, &coll, Standard::Programmable, &f.a), Err(NftError::NotAnNft(s)) if s.contains("supply 2")));
    // the accounts were not read at all
    assert!(matches!(solnft::is_member(&mint, &coll, Standard::Programmable, &Accounts::new()), Err(NftError::Missing(_))));
}

#[test]
fn refusals_on_real_accounts() {
    let f = fx("core_refusals");
    // Retardio Cousins Classic: its collection carries a PermanentFreezeDelegate
    let e = solnft::is_member(&f.k("retardio_asset"), &f.k("retardio_collection"), Standard::Core, &f.a).unwrap_err();
    assert!(matches!(&e, NftError::Unsupported(s) if s.contains("permanent") && s.contains("collection")), "{e}");
    // Critters #600: a TransferDelegate plugin, its authority the owner (no delegate): accepted; handed to the update
    // authority (a delegate the owner does not control): refused
    let (ca, cc) = (f.k("critters_asset"), f.k("critters_collection"));
    solnft::is_member(&ca, &cc, Standard::Core, &f.a).unwrap();
    let mut g = fx("core_refusals");
    let at = transfer_delegate_authority(&g.a[&ca].as_ref().unwrap().data);
    assert_eq!(g.data(&ca)[at], 1, "Owner");
    g.data(&ca)[at] = 2;
    let e = solnft::is_member(&ca, &cc, Standard::Core, &g.a).unwrap_err();
    assert!(matches!(&e, NftError::Unsupported(s) if s.contains("delegate")), "{e}");
    // USDC: a fungible
    let e = solnft::is_member(&f.k("usdc"), &key(MAD_LADS), Standard::Legacy, &f.a).unwrap_err();
    assert!(matches!(e, NftError::NotAnNft(_)), "{e}");
    // a compressed NFT (Tensorians): there is no account
    let e = solnft::is_member(&f.k("cnft_tensorians"), &key(MAD_LADS), Standard::Legacy, &f.a).unwrap_err();
    assert!(matches!(&e, NftError::Missing(s) if s.contains("does not exist")), "{e}");
    let e = solnft::is_member(&f.k("cnft_tensorians"), &key(MAD_LADS), Standard::Core, &f.a).unwrap_err();
    assert!(matches!(e, NftError::Missing(_)));
    // a HashedAssetV1 (Core, compressed): refused as not an NFT
    let mut g = fx("tensor_core_otc");
    let asset = g.k("asset");
    g.data(&asset)[0] = 2;
    assert!(matches!(solnft::is_member(&asset, &g.k("collection"), Standard::Core, &g.a), Err(NftError::NotAnNft(s)) if s.contains("Hashed")));
}

/// The byte of the asset's plugin registry that holds its TransferDelegate's authority tag.
fn transfer_delegate_authority(d: &[u8]) -> usize {
    let mut r = soltx::Reader { data: d, at: 33 };
    if r.u8().unwrap() != 0 {
        r.take(32).unwrap();
    }
    r.string(1000).unwrap();
    r.string(1000).unwrap();
    if r.bool().unwrap() {
        r.u64().unwrap();
    }
    assert_eq!(r.u8().unwrap(), 3);
    let mut r = soltx::Reader { data: d, at: r.u64().unwrap() as usize + 1 };
    for _ in 0..r.u32().unwrap() {
        let (t, at) = (r.u8().unwrap(), r.at);
        if r.u8().unwrap() == 3 {
            r.take(32).unwrap();
        }
        r.u64().unwrap();
        if t == solnft::PT_TRANSFER_DELEGATE {
            return at;
        }
    }
    panic!("no TransferDelegate")
}

// ---- custody ------------------------------------------------------------------------------------------------------------------------------

#[test]
fn custody() {
    // escrowed items are in their escrow's custody (the same rules as the canister's own wallet)
    let f = fx("tensor_pnft_mad_lads");
    let m = member(&f, Standard::Programmable);
    let c = solnft::in_custody(&m, &f.k("list_state"), &f.a).unwrap();
    assert_eq!(c.account, soltx::associated_token_address(&f.k("list_state"), &m.asset));
    assert!(matches!(solnft::in_custody(&m, &ME, &f.a), Err(NftError::NotInCustody(_))));
    let f = fx("tensor_legacy_fff");
    let m = member(&f, Standard::Legacy);
    solnft::in_custody(&m, &f.k("list_state"), &f.a).unwrap();
    let f = fx("tensor_core_otc");
    let m = member(&f, Standard::Core);
    solnft::in_custody(&m, &f.k("list_state"), &f.a).unwrap();
    assert!(matches!(solnft::in_custody(&m, &ME, &f.a), Err(NftError::NotInCustody(_))));
    // an M2 listing escrows the item at M2's program-as-signer, not in the seller's wallet
    let f = fx("m2_pnft_mad_lads_wallet");
    let m = member(&f, Standard::Programmable);
    solnft::in_custody(&m, &buy::m2_program_as_signer(), &f.a).unwrap();
    assert!(solnft::in_custody(&m, &f.k("seller"), &f.a).is_err());
}

#[test]
fn custody_refuses_delegates_and_locks() {
    let base = fx("tensor_pnft_mad_lads");
    let m = member(&base, Standard::Programmable);
    let holder = base.k("list_state");
    let ta = soltx::associated_token_address(&holder, &m.asset);
    let rec = solnft::token_record_address(&m.asset, &ta);
    // a pNFT's token record Locked, or with a delegate
    let mut f = fx("tensor_pnft_mad_lads");
    f.data(&rec)[2] = 1;
    assert!(matches!(solnft::in_custody(&m, &holder, &f.a), Err(NftError::NotInCustody(s)) if s.contains("Locked")));
    let mut f = fx("tensor_pnft_mad_lads");
    let d = f.data(&rec);
    let mut x = d[..3].to_vec();
    x.extend([0u8, 1]);
    x.extend([0x77; 32]);
    x.resize(d.len(), 0);
    *d = x;
    assert!(matches!(solnft::in_custody(&m, &holder, &f.a), Err(NftError::NotInCustody(s)) if s.contains("delegate")));
    // a legacy token account with a delegate, or frozen
    let base = fx("tensor_legacy_fff");
    let m = member(&base, Standard::Legacy);
    let holder = base.k("list_state");
    let ta = soltx::associated_token_address(&holder, &m.asset);
    let mut f = fx("tensor_legacy_fff");
    f.data(&ta)[72] = 1;
    f.data(&ta)[76..108].copy_from_slice(&[0x77; 32]);
    assert!(matches!(solnft::in_custody(&m, &holder, &f.a), Err(NftError::NotInCustody(s)) if s.contains("delegate")));
    let mut f = fx("tensor_legacy_fff");
    f.data(&ta)[108] = 2;
    assert!(matches!(solnft::in_custody(&m, &holder, &f.a), Err(NftError::NotInCustody(s)) if s.contains("frozen")));
}

// ---- the two-provider read ------------------------------------------------------------------------------------------------------------

/// A provider answering `getMultipleAccounts` from a fixture as a real one would (then through `normalise`); `lie` changes one
/// account's lamports in its answers.
struct Fake {
    name: &'static str,
    a: Accounts,
    slot: u64,
    lie: Option<Pubkey>,
}

impl SolRpc for Fake {
    fn name(&self) -> String {
        self.name.into()
    }
    async fn ask(&mut self, r: &Req) -> Result<Value, String> {
        assert_eq!(r.method, "getMultipleAccounts");
        assert_eq!(r.params[1]["commitment"], "finalized");
        let vals: Vec<Value> = r.params[0].as_array().unwrap().iter().map(|k| {
            let k = soltx::pubkey(k.as_str().unwrap()).unwrap();
            match self.a.get(&k).cloned().flatten() {
                None => Value::Null,
                Some(x) => json!({ "lamports": x.lamports + (self.lie == Some(k)) as u64, "owner": soltx::b58_encode(&x.owner),
                    "executable": x.executable, "rentEpoch": 18446744073709551615u64, "space": x.data.len(),
                    "data": [base64::engine::general_purpose::STANDARD.encode(&x.data), "base64"] }),
            }
        }).collect();
        let body = json!({ "jsonrpc": "2.0", "id": 1, "result": { "context": { "slot": self.slot }, "value": vals } }).to_string();
        Ok(solrpc::normalise(200, body.as_bytes(), &solrpc::context(Kind::Accounts, r.bucket)))
    }
}

fn pair(f: &Fx, lie: Option<Pubkey>) -> Vec<Fake> {
    vec![Fake { name: "a", a: f.a.clone(), slot: f.slot, lie: None }, Fake { name: "b", a: f.a.clone(), slot: f.slot + 3, lie }]
}

#[test]
fn check_reads_at_finalized_through_two_providers() {
    let f = fx("m2_pnft_mad_lads_wallet");
    let (asset, coll, seller) = (f.k("asset"), f.k("collection"), buy::m2_program_as_signer());
    let (m, c) = block_on(solnft::check(&mut pair(&f, None), &asset, &coll, Standard::Programmable, &seller)).unwrap();
    assert_eq!((m.asset, c.holder), (asset, seller));
    // one provider answers differently: refused, never guessed
    let e = block_on(solnft::check(&mut pair(&f, Some(asset)), &asset, &coll, Standard::Programmable, &seller)).unwrap_err();
    assert!(e.contains("disagree"), "{e}");
    let e = block_on(solnft::read(&mut pair(&f, None)[..1], &[asset])).unwrap_err();
    assert!(matches!(e, Refused::Failed(s) if s.contains("two at least")));
    // not the canister's: the check says so
    let e = block_on(solnft::check(&mut pair(&f, None), &asset, &coll, Standard::Programmable, &ME)).unwrap_err();
    assert!(e.contains("not in custody"), "{e}");
}

// ---- asks and the judge -------------------------------------------------------------------------------------------------------------------

fn ask_of(f: &Fx, venue: Venue, listing: &str, std: Standard) -> (Ask, Member) {
    let m = member(f, std);
    let c = Candidate { venue, listing: f.k(listing), asset: f.k("asset") };
    (buy::verify_ask(&c, &m, &f.a, f.unix_time).unwrap(), m)
}

#[test]
fn asks_price_everything_the_buyer_pays() {
    // Tensor pNFT: 10 SOL + 2 % taker fee + 4.2 % royalties (enforced)
    let (x, m) = ask_of(&fx("tensor_pnft_mad_lads"), Venue::Tensor, "list_state", Standard::Programmable);
    assert_eq!((x.price, x.total, x.keyless()), (10 * SOL, 10_620_000_000, true));
    assert_eq!(x.rent(&m), buy::ATA_RENT + buy::TOKEN_RECORD_RENT);
    // Tensor legacy: no royalties (optional, not paid)
    let (x, _) = ask_of(&fx("tensor_legacy_fff"), Venue::Tensor, "list_state", Standard::Legacy);
    assert_eq!((x.price, x.total), (10 * SOL, 10_200_000_000));
    // Tensor Core: Royalties 5 % enforced
    let (x, m) = ask_of(&fx("tensor_core_otc"), Venue::Tensor, "list_state", Standard::Core);
    assert_eq!(x.total, x.price + x.price / 50 + x.price / 20);
    assert_eq!(x.rent(&m), 0);
    // mmm: a step over spot, Magic Eden's notary must co-sign
    let f = fx("mmm_pnft_smb_gen3");
    let (x, _) = ask_of(&f, Venue::Mmm, "sell_state", Standard::Programmable);
    let AskDetail::Mmm { state: p, .. } = &x.detail else { panic!() };
    assert_eq!(x.price, p.spot_price + p.spot_price * p.curve_delta / 10_000);
    assert_eq!(x.cosigner, Some(key(buy::ME_NOTARY)));
    assert!(!x.keyless());
    assert!(x.total > x.price);
    // M2: priced, but notary-gated
    let (x, _) = ask_of(&fx("m2_pnft_mad_lads_wallet"), Venue::M2, "seller_trade_state", Standard::Programmable);
    assert_eq!(x.price, 8_352_000_000);
    assert!(!x.keyless());
    let ah = fx("m2_pnft_mad_lads_wallet");
    let (notary, required) = buy::m2_requires_notary(&ah.a[&key(buy::M2_AUCTION_HOUSE)].as_ref().unwrap().data).unwrap();
    assert_eq!((notary, required), (key(buy::ME_NOTARY), true));
}

#[test]
fn stale_or_foreign_asks_are_refused() {
    let f = fx("tensor_pnft_mad_lads");
    let m = member(&f, Standard::Programmable);
    let c = Candidate { venue: Venue::Tensor, listing: f.k("list_state"), asset: f.k("asset") };
    let AskDetail::Tensor(l) = buy::verify_ask(&c, &m, &f.a, f.unix_time).unwrap().detail else { panic!() };
    assert!(buy::verify_ask(&c, &m, &f.a, l.expiry + 1).unwrap_err().contains("expiry"));
    // the index named the wrong venue / the wrong asset / an escrow that no longer holds it
    assert!(buy::verify_ask(&Candidate { venue: Venue::Mmm, ..c.clone() }, &m, &f.a, f.unix_time).is_err());
    let mut g = fx("tensor_pnft_mad_lads");
    let lta = soltx::associated_token_address(&c.listing, &c.asset);
    g.data(&lta)[64] = 0;
    assert!(buy::verify_ask(&c, &m, &g.a, f.unix_time).unwrap_err().contains("escrow holds 0"));
    // a listing that requires a cosigner is priced but not keyless, and the canister will not build it
    let mut g = fx("tensor_pnft_mad_lads");
    // this ListState: currency None, private taker None, maker broker Some: the cosigner sits at 157..189
    g.data(&c.listing)[157..189].copy_from_slice(&[0x55; 32]);
    let x = buy::verify_ask(&c, &m, &g.a, f.unix_time).unwrap();
    assert_eq!(x.cosigner, Some([0x55; 32]));
    assert!(buy::tensor_buy(&ME, &x, &m).unwrap_err().contains("co-signature"));
}

#[test]
fn best_ask_takes_the_cheapest_keyless_under_the_limit() {
    let t = ask_of(&fx("tensor_pnft_mad_lads"), Venue::Tensor, "list_state", Standard::Programmable);
    let mmm = ask_of(&fx("mmm_pnft_smb_gen3"), Venue::Mmm, "sell_state", Standard::Programmable);
    let m2 = ask_of(&fx("m2_pnft_mad_lads_wallet"), Venue::M2, "seller_trade_state", Standard::Programmable);
    let all = vec![t.clone(), mmm.clone(), m2.clone()];
    let big = 100 * SOL;
    assert_eq!(buy::best_ask(&all, big, &[]).unwrap().0.venue, Venue::Tensor, "only Tensor is keyless");
    assert_eq!(buy::best_ask(&all, big, &[Venue::Mmm]).unwrap().0.venue, Venue::Mmm, "cheaper once mmm is co-signed");
    assert_eq!(buy::best_ask(&all, big, &[Venue::Mmm, Venue::M2]).unwrap().0.venue, Venue::Mmm, "M2 is never chosen");
    let limit = t.0.total + t.0.rent(&t.1);
    assert!(buy::best_ask(&all, limit, &[]).is_some());
    assert!(buy::best_ask(&all, limit - 1, &[]).is_none(), "the rent counts");
}

#[test]
fn the_judge() {
    let f = fx("tensor_pnft_mad_lads");
    let (x, m) = ask_of(&f, Venue::Tensor, "list_state", Standard::Programmable);
    let coll = f.k("collection");
    let limit = x.total + x.rent(&m);
    let e = Expect { payer: ME, ask: &x, member: &m, collection: coll, limit };
    let ixs = buy::buy_instructions(&ME, &x, &m).unwrap();
    buy::judge(&ixs, &e).unwrap();
    // J1: anything else in the transaction
    let mut more = ixs.clone();
    more.push(soltx::transfer(ME, [0x99; 32], 1));
    assert!(buy::judge(&more, &e).unwrap_err().starts_with("J1"));
    let mut two = ixs.clone();
    two.push(ixs[1].clone());
    assert!(buy::judge(&two, &e).unwrap_err().starts_with("J1"));
    // J2: a taker broker of ours slipped in, the creators reordered, another rule set, a Core instruction for a pNFT
    let mut b = ixs.clone();
    b[1].accounts[8] = soltx::meta([0x99; 32], false, true);
    assert!(buy::judge(&b, &e).unwrap_err().starts_with("J2"));
    let mut b = ixs.clone();
    b[1].accounts[19].key = key(solnft::COMPATIBILITY_RULE_SET);
    assert!(buy::judge(&b, &e).unwrap_err().starts_with("J2"));
    let mut b = ixs.clone();
    b[1].data[..8].copy_from_slice(&soltx::anchor_disc("global", "buy_core"));
    assert!(buy::judge(&b, &e).unwrap_err().starts_with("J2"));
    // J3: a raised maximum, trailing data, a limit under the price
    let mut b = ixs.clone();
    b[1].data[8..16].copy_from_slice(&(x.price * 2).to_le_bytes());
    assert!(buy::judge(&b, &e).unwrap_err().starts_with("J3"));
    let mut b = ixs.clone();
    b[1].data.push(0);
    assert!(buy::judge(&b, &e).unwrap_err().starts_with("J3"));
    assert!(buy::judge(&ixs, &Expect { limit: limit - 1, ..e.clone() }).unwrap_err().starts_with("J3"));
    // J4: the member of another collection
    assert!(buy::judge(&ixs, &Expect { collection: [0x42; 32], ..e.clone() }).unwrap_err().starts_with("J4"));
    // a lower maximum is fine for the judge (the program then refuses it if under the listing)
    let mut b = ixs.clone();
    b[1].data[8..16].copy_from_slice(&(x.price - 1).to_le_bytes());
    buy::judge(&b, &e).unwrap();
}

#[test]
fn the_judge_on_mmm() {
    let f = fx("mmm_legacy_tff");
    let (x, m) = ask_of(&f, Venue::Mmm, "sell_state", Standard::Legacy);
    let e = Expect { payer: ME, ask: &x, member: &m, collection: f.k("collection"), limit: x.total + x.rent(&m) };
    let ixs = buy::buy_instructions(&ME, &x, &m).unwrap();
    buy::judge(&ixs, &e).unwrap();
    // the cosigner is a signer of the canonical instruction: Magic Eden's notary
    assert!(ixs[1].accounts[2].signer && ixs[1].accounts[2].key == key(buy::ME_NOTARY));
    // a taker fee within 5 % is allowed only as far as the limit allows; over 5 % never
    let mut b = ixs.clone();
    let n = b[1].data.len();
    b[1].data[n - 2..].copy_from_slice(&600u16.to_le_bytes());
    assert!(buy::judge(&b, &e).unwrap_err().contains("fees out of range"));
    let mut b = ixs.clone();
    let n = b[1].data.len();
    b[1].data[n - 4..n - 2].copy_from_slice(&(-100i16).to_le_bytes());
    buy::judge(&b, &e).unwrap();
    // a legacy buyside royalty: refused
    let mut b = ixs.clone();
    b[1].data[24..26].copy_from_slice(&100u16.to_le_bytes());
    assert!(buy::judge(&b, &e).unwrap_err().contains("royalty"));
}

#[test]
fn wire_size_fits() {
    for (name, venue, listing, std) in [("tensor_pnft_mad_lads", Venue::Tensor, "list_state", Standard::Programmable),
        ("tensor_legacy_fff", Venue::Tensor, "list_state", Standard::Legacy), ("tensor_core_otc", Venue::Tensor, "list_state", Standard::Core),
        ("mmm_pnft_smb_gen3", Venue::Mmm, "sell_state", Standard::Programmable), ("mmm_legacy_tff", Venue::Mmm, "sell_state", Standard::Legacy)] {
        let (x, m) = ask_of(&fx(name), venue, listing, std);
        let parts = soltx::compile_parts(&buy::buy_instructions(&ME, &x, &m).unwrap(), ME, [1; 32]).unwrap();
        let size = soltx::wire_size(&parts.message());
        assert!(size <= soltx::PACKET_DATA_SIZE, "{name}: {size}");
        let parts = soltx::compile_parts(&send::send_instructions(&ME, &key(buy::ME_NOTARY), &m).unwrap(), ME, [1; 32]).unwrap();
        assert!(soltx::wire_size(&parts.message()) <= soltx::PACKET_DATA_SIZE);
    }
}

// ---- delivery -----------------------------------------------------------------------------------------------------------------------------

#[test]
fn delivery_instructions_and_their_judge() {
    let to = key(buy::ME_NOTARY); // any wallet (on the curve)
    for (name, std, n) in [("tensor_pnft_mad_lads", Standard::Programmable, 2), ("tensor_legacy_fff", Standard::Legacy, 4),
        ("tensor_core_otc", Standard::Core, 2)] {
        let m = member(&fx(name), std);
        let ixs = send::send_instructions(&ME, &to, &m).unwrap();
        assert_eq!(ixs.len(), n, "{name}");
        send::judge_send(&ixs, &ME, &to, &m).unwrap();
        let mut b = ixs.clone();
        b.push(soltx::transfer(ME, to, 1));
        assert!(send::judge_send(&b, &ME, &to, &m).is_err());
        assert!(send::judge_send(&ixs, &ME, &[0x22; 32], &m).is_err());
    }
    let m = member(&fx("tensor_core_otc"), Standard::Core);
    assert!(send::send_instructions(&ME, &ME, &m).is_err(), "to the canister itself");
    assert!(send::send_instructions(&ME, &[0; 32], &m).is_err());
    let pda = buy::list_state_address(&m.asset);
    assert!(send::send_instructions(&ME, &pda, &m).unwrap_err().contains("curve"), "a program's PDA is no wallet");
    // the pNFT transfer carries the item's rule set and the auth rules program
    let m = member(&fx("tensor_pnft_mad_lads"), Standard::Programmable);
    let ix = &send::send_instructions(&ME, &to, &m).unwrap()[1];
    assert_eq!(ix.program, key(solnft::TOKEN_METADATA));
    assert_eq!(ix.accounts[15].key, key(solnft::TOKEN_AUTH_RULES));
    assert_eq!(ix.accounts[16].key, key(solnft::FOUNDATION_RULE_SET));
}

// ---- the floor -------------------------------------------------------------------------------------------------------------------------------

#[test]
fn floor_from_real_listings_at_finalized() {
    let f = fx("floor_mad_lads");
    let cands = floor::candidates_from_me(&f.facts["me_listings"]);
    assert!(cands.len() >= 8, "{}", cands.len());
    let reference = floor::reference_from_me_stats(&f.facts["me_stats"]).unwrap();
    assert_eq!(reference, 8_352_000_000);
    let coll = key(MAD_LADS);
    let (fl, asks) = block_on(floor::read_floor(&mut pair(&f, None), &cands, &coll, Standard::Programmable, f.unix_time, reference,
        &FloorParams::default())).unwrap();
    assert_eq!(asks.len(), cands.len(), "every listing verified on chain");
    // one ask per seller: 9YCN4w… lists three; the 2nd cheapest seller's ask
    assert_eq!(fl.cheapest, 8_352_000_000);
    assert_eq!(fl.price, 8_353_000_000);
    assert!(fl.sellers >= 5 && fl.depth >= 4, "{fl:?}");
    assert_eq!(fl.keyless_total, None, "every one of these is M2: notary-gated");
    // the reference disagrees by more than 20 %: refused
    let e = floor::floor_from(&asks, 6 * SOL, &FloorParams::default()).unwrap_err();
    assert!(e.contains("refused"), "{e}");
    // a provider that disagrees: no floor
    let lie = cands[0].listing;
    assert!(block_on(floor::read_floor(&mut pair(&f, Some(lie)), &cands, &coll, Standard::Programmable, f.unix_time, reference,
        &FloorParams::default())).unwrap_err().contains("disagree"));
}

#[test]
fn floor_rules() {
    let t = ask_of(&fx("tensor_pnft_mad_lads"), Venue::Tensor, "list_state", Standard::Programmable);
    let mk = |seller: u8, asset: u8, price: u64| {
        let mut a = t.clone();
        a.0.seller = [seller; 32];
        a.0.asset = [asset; 32];
        a.0.price = price;
        a.0.cosigner = Some(key(buy::ME_NOTARY));
        a.0.venue = Venue::M2;
        a
    };
    let p = FloorParams::default();
    // one seller listing many is one voice
    let sybil = vec![mk(1, 1, 5 * SOL), mk(1, 2, 5 * SOL), mk(1, 3, 5 * SOL), mk(2, 4, 9 * SOL)];
    assert!(floor::floor_from(&sybil, 9 * SOL, &p).unwrap_err().contains("2 distinct sellers"));
    // one asset on two venues counts once (the cheaper)
    let dup = vec![mk(1, 1, 8 * SOL), mk(2, 1, 7 * SOL), mk(3, 3, 9 * SOL), mk(4, 4, 10 * SOL)];
    let fl = floor::floor_from(&dup, 9 * SOL, &p).unwrap();
    assert_eq!((fl.sellers, fl.cheapest, fl.price), (3, 7 * SOL, 9 * SOL));
    // the keyless ask is reported apart (all in)
    let mut with_t = dup.clone();
    with_t.push(t.clone());
    let fl = floor::floor_from(&with_t, 9 * SOL, &p).unwrap();
    assert_eq!((fl.keyless_total, fl.keyless_venue), (Some(t.0.total), Some(Venue::Tensor)));
    assert!(floor::floor_from(&dup, 0, &p).is_err());
    assert!(floor::reference_from_me_stats(&json!({"floorPrice": 0})).is_err());
    // candidates: unknown sources and junk are skipped, the order is canonical
    let v = json!([{"listingSource": "TENSOR_AMM_V2", "pdaAddress": MAD_LADS, "tokenMint": MAD_LADS},
        {"listingSource": "M2", "pdaAddress": "nope", "tokenMint": MAD_LADS},
        {"listingSource": "MMM", "pdaAddress": MAD_LADS, "tokenMint": buy::ME_NOTARY}]);
    assert_eq!(floor::candidates_from_me(&v).len(), 1);
}
