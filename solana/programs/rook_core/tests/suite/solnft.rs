//! Solana NFT listings (docs/v9/solana-nfts-design.md): the NFTs sit in the canister's wallet (the custodian); `mint_item`
//! checks on chain that the item is a verified member held by the custodian, `record_delivery` that the ticket's Solana
//! destination holds it. Run against the real Token Metadata, Token Auth Rules and mpl-core programs and the Metaplex
//! Foundation / Compatibility rule sets, all dumped from mainnet into `solana/tests/fixtures`.

use super::common::*;
use anchor_lang::prelude::Pubkey;
use anchor_lang::Space;
use anchor_lang::solana_program::{
    instruction::{AccountMeta, Instruction},
    system_program,
};
use anchor_spl::token::spl_token;
use rook_core::error::RookError;
use rook_core::hashing::keccak;
use rook_core::instructions::MintItemArgs;
use rook_core::metadata::MPL_TOKEN_METADATA_ID as TM;
use rook_core::nft::MPL_CORE_ID as CORE;
use rook_core::seeds::*;
use rook_core::state::*;
use rook_core::units::ONE;
use solana_keypair::Keypair;
use solana_signer::Signer;

const AUTH: Pubkey = anchor_lang::pubkey!("auth9SigNpDKz4sJJ1DfCTuZrZNSAgh9sFD3rboVmgg");
const ATA: Pubkey = anchor_lang::pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
const SYSVAR_IX: Pubkey = anchor_lang::pubkey!("Sysvar1nstructions1111111111111111111111111");
const CB: Pubkey = anchor_lang::pubkey!("ComputeBudget111111111111111111111111111111");
/// The Metaplex Foundation Rule Set (latest revision: `Pass` for every operation) and the Compatibility Rule Set.
const FOUNDATION: Pubkey = anchor_lang::pubkey!("eBJLFYPxJmMGKuFwpDWkzxZeUrad92kZRC5BJLpzyT9");
const COMPAT: Pubkey = anchor_lang::pubkey!("AdH2Utn6Fus15ZhtenW4hZBQnvtLgM1YCW2MfVp7pYS5");

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").join(name)
}

fn b64(s: &str) -> Vec<u8> {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|c| *c != b'=') {
        buf = (buf << 6) | A.iter().position(|a| *a == c).unwrap() as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}

/// A rule set account from `solana account --output json` (data base64).
fn load_rule_set(env: &mut Env, key: Pubkey, file: &str) {
    let j = std::fs::read_to_string(fixture(file)).unwrap();
    let i = j.find("\"data\":[\"").unwrap() + 9;
    let data = b64(&j[i..i + j[i..].find('"').unwrap()]);
    let lamports = env.svm.minimum_balance_for_rent_exemption(data.len());
    env.svm
        .set_account(key, solana_account::Account { lamports, data, owner: AUTH, executable: false, rent_epoch: u64::MAX })
        .unwrap();
}

fn s(out: &mut Vec<u8>, v: &str) {
    out.extend_from_slice(&(v.len() as u32).to_le_bytes());
    out.extend_from_slice(v.as_bytes());
}

fn cu_limit() -> Instruction {
    let mut d = vec![2u8];
    d.extend_from_slice(&1_400_000u32.to_le_bytes());
    Instruction { program_id: CB, accounts: vec![], data: d }
}

fn tm_pda(seeds: &[&[u8]]) -> Pubkey {
    let mut s: Vec<&[u8]> = vec![b"metadata", TM.as_ref()];
    s.extend_from_slice(seeds);
    Pubkey::find_program_address(&s, &TM).0
}
fn metadata_of(mint: &Pubkey) -> Pubkey {
    tm_pda(&[mint.as_ref()])
}
fn edition_of(mint: &Pubkey) -> Pubkey {
    tm_pda(&[mint.as_ref(), b"edition"])
}
fn record_of(mint: &Pubkey, ta: &Pubkey) -> Pubkey {
    tm_pda(&[mint.as_ref(), b"token_record", ta.as_ref()])
}
fn ata(wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[wallet.as_ref(), spl_token::ID.as_ref(), mint.as_ref()], &ATA).0
}
fn w(k: Pubkey) -> AccountMeta {
    AccountMeta::new(k, false)
}
fn r(k: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(k, false)
}

const NON_FUNGIBLE: u8 = 0;
const PROGRAMMABLE: u8 = 4;

/// A Token Metadata NFT made by `ua` with Token Metadata's own `CreateV1` + `MintV1` (to `owner`'s ATA), and, with a
/// collection, verified into it by `VerifyCollectionV1` (when `verify`). Returns the mint.
fn tm_nft(env: &mut Env, ua: &Keypair, standard: u8, collection: Option<Pubkey>, verify: bool, rule_set: Option<Pubkey>, owner: &Pubkey, is_collection: bool) -> Pubkey {
    let mint = Keypair::new();
    let m = mint.pubkey();
    let mut d = vec![42u8, 0];
    s(&mut d, "Rook test");
    s(&mut d, "RT");
    s(&mut d, "https://example.invalid/x.json");
    d.extend_from_slice(&0u16.to_le_bytes());
    d.extend_from_slice(&[0, 0, 1, standard]); // creators None, primary sale false, mutable, standard
    match collection {
        Some(c) => {
            d.extend_from_slice(&[1, 0]);
            d.extend_from_slice(c.as_ref());
        }
        None => d.push(0),
    }
    d.push(0); // uses
    if is_collection {
        d.extend_from_slice(&[1, 0]);
        d.extend_from_slice(&0u64.to_le_bytes());
    } else {
        d.push(0);
    }
    match rule_set {
        Some(k) => {
            d.push(1);
            d.extend_from_slice(k.as_ref());
        }
        None => d.push(0),
    }
    d.push(0); // decimals
    d.extend_from_slice(&[1, 0]); // print supply Zero
    let u = ua.pubkey();
    let create = Instruction {
        program_id: TM,
        accounts: vec![
            w(metadata_of(&m)),
            w(edition_of(&m)),
            AccountMeta::new(m, true),
            AccountMeta::new_readonly(u, true),
            AccountMeta::new(u, true),
            AccountMeta::new_readonly(u, true),
            r(system_program::ID),
            r(SYSVAR_IX),
            r(spl_token::ID),
        ],
        data: d,
    };
    let ta = ata(owner, &m);
    let prog = standard == PROGRAMMABLE;
    let mut d = vec![43u8, 0];
    d.extend_from_slice(&1u64.to_le_bytes());
    d.push(0);
    let mint_ix = Instruction {
        program_id: TM,
        accounts: vec![
            w(ta),
            r(*owner),
            w(metadata_of(&m)),
            w(edition_of(&m)),
            if prog { w(record_of(&m, &ta)) } else { r(TM) },
            w(m),
            AccountMeta::new_readonly(u, true),
            r(TM),
            AccountMeta::new(u, true),
            r(system_program::ID),
            r(SYSVAR_IX),
            r(spl_token::ID),
            r(ATA),
            r(if rule_set.is_some() { AUTH } else { TM }),
            r(rule_set.unwrap_or(TM)),
        ],
        data: d,
    };
    ok(env.send(&[cu_limit(), create, mint_ix], &[ua, &mint]));
    if let (Some(c), true) = (collection, verify) {
        let v = Instruction {
            program_id: TM,
            accounts: vec![
                AccountMeta::new_readonly(u, true),
                r(TM),
                w(metadata_of(&m)),
                r(c),
                w(metadata_of(&c)),
                r(edition_of(&c)),
                r(system_program::ID),
                r(SYSVAR_IX),
            ],
            data: vec![52, 1],
        };
        ok(env.send(&[cu_limit(), v], &[ua]));
    }
    m
}

/// Token Metadata `TransferV1`, owner → `to` (both ATAs), the owner signing and paying.
fn tm_transfer(owner: &Pubkey, to: &Pubkey, mint: &Pubkey, prog: bool, rule_set: Option<Pubkey>) -> Instruction {
    let (src, dst) = (ata(owner, mint), ata(to, mint));
    let mut d = vec![49u8, 0];
    d.extend_from_slice(&1u64.to_le_bytes());
    d.push(0);
    Instruction {
        program_id: TM,
        accounts: vec![
            w(src),
            r(*owner),
            w(dst),
            r(*to),
            r(*mint),
            w(metadata_of(mint)),
            r(edition_of(mint)),
            if prog { w(record_of(mint, &src)) } else { r(TM) },
            if prog { w(record_of(mint, &dst)) } else { r(TM) },
            AccountMeta::new_readonly(*owner, true),
            AccountMeta::new(*owner, true),
            r(system_program::ID),
            r(SYSVAR_IX),
            r(spl_token::ID),
            r(ATA),
            r(if rule_set.is_some() { AUTH } else { TM }),
            r(rule_set.unwrap_or(TM)),
        ],
        data: d,
    }
}

/// A Core collection (update authority `ua`) and its plugins (raw Borsh `Vec<PluginAuthorityPair>` items).
fn core_collection(env: &mut Env, ua: &Keypair, plugins: &[Vec<u8>]) -> Pubkey {
    let c = Keypair::new();
    let mut d = vec![1u8];
    s(&mut d, "Rook Core");
    s(&mut d, "https://example.invalid/c.json");
    push_plugins(&mut d, plugins);
    let i = Instruction {
        program_id: CORE,
        accounts: vec![AccountMeta::new(c.pubkey(), true), r(CORE), AccountMeta::new(ua.pubkey(), true), r(system_program::ID)],
        data: d,
    };
    ok(env.send(&[i], &[ua, &c]));
    c.pubkey()
}

fn push_plugins(d: &mut Vec<u8>, plugins: &[Vec<u8>]) {
    if plugins.is_empty() {
        d.push(0);
    } else {
        d.push(1);
        d.extend_from_slice(&(plugins.len() as u32).to_le_bytes());
        for p in plugins {
            d.extend_from_slice(p);
        }
    }
}

/// A Core asset in `collection` (its update authority `ua` signs), owned by `owner`.
fn core_asset(env: &mut Env, ua: &Keypair, collection: Pubkey, owner: &Pubkey, plugins: &[Vec<u8>]) -> Pubkey {
    let a = Keypair::new();
    let mut d = vec![20u8, 0];
    s(&mut d, "Rook Core #1");
    s(&mut d, "https://example.invalid/a.json");
    push_plugins(&mut d, plugins);
    d.push(0); // external plugin adapters
    let i = Instruction {
        program_id: CORE,
        accounts: vec![
            AccountMeta::new(a.pubkey(), true),
            w(collection),
            AccountMeta::new_readonly(ua.pubkey(), true),
            AccountMeta::new(ua.pubkey(), true),
            r(*owner),
            r(CORE),
            r(system_program::ID),
            r(CORE),
        ],
        data: d,
    };
    ok(env.send(&[i], &[ua, &a]));
    a.pubkey()
}

fn core_transfer(asset: Pubkey, collection: Pubkey, owner: &Pubkey, to: &Pubkey) -> Instruction {
    Instruction {
        program_id: CORE,
        accounts: vec![
            w(asset),
            r(collection),
            AccountMeta::new(*owner, true),
            AccountMeta::new_readonly(*owner, true),
            r(*to),
            r(system_program::ID),
            r(CORE),
        ],
        data: vec![14, 0],
    }
}

/// A Solana NFT listing env: the registry, a `SolNft` listing of `collection` (the canister = `env.custodian` is custodian,
/// wallet and poster), its wrapper and module; the Metaplex programs and rule sets loaded.
struct Sol {
    env: Env,
    creator: Keypair,
    collection: Pubkey,
    standard: NftStandard,
}

fn programs(env: &mut Env) {
    let core = std::fs::read(fixture("mpl_core.so")).unwrap();
    env.svm.add_program(CORE, &core).unwrap();
    let auth = std::fs::read(fixture("mpl_token_auth_rules.so")).unwrap();
    env.svm.add_program(AUTH, &auth).unwrap();
    load_rule_set(env, FOUNDATION, "rule_set_metaplex_foundation.json");
    load_rule_set(env, COMPAT, "rule_set_metaplex_compatibility.json");
}

fn sol(standard: NftStandard, core_plugins: &[Vec<u8>]) -> Sol {
    let mut env = Env::bare();
    programs(&mut env);
    let creator = Keypair::new();
    env.svm.airdrop(&creator.pubkey(), 1_000 * SOL).unwrap();
    let collection = if standard == NftStandard::Core {
        core_collection(&mut env, &creator, core_plugins)
    } else {
        let c = creator.pubkey();
        tm_nft(&mut env, &creator, NON_FUNGIBLE, None, false, None, &c, true)
    };
    let c = env.custodian.pubkey();
    ok(env.list(ListingKind::SolNft { collection, standard }, "Mad Lads", "", c, c));
    env.init_wrapper();
    env.init_module(default_params());
    Sol { env, creator, collection, standard }
}

impl Sol {
    /// A member NFT owned by `owner` (Token Metadata: `prog` → pNFT on `rule_set`).
    fn nft(&mut self, owner: &Pubkey, prog: bool, rule_set: Option<Pubkey>) -> Pubkey {
        let (cr, c) = (self.creator.insecure_clone(), self.collection);
        if self.standard == NftStandard::Core {
            core_asset(&mut self.env, &cr, c, owner, &[])
        } else {
            let st = if prog { PROGRAMMABLE } else { NON_FUNGIBLE };
            tm_nft(&mut self.env, &cr, st, Some(c), true, rule_set, owner, false)
        }
    }

    /// `mint_item`'s remaining accounts for `asset` held by `holder`.
    fn custody_accounts(&self, asset: &Pubkey, holder: &Pubkey) -> Vec<AccountMeta> {
        if self.standard == NftStandard::Core {
            vec![r(*asset), r(self.collection)]
        } else {
            vec![r(*asset), r(metadata_of(asset)), r(edition_of(asset)), r(ata(holder, asset))]
        }
    }

    fn args(asset: &Pubkey, n: u64, deposit: bool) -> MintItemArgs {
        let key = asset.to_bytes();
        let tx = tx_hash(n);
        MintItemArgs { key, source: keccak(&[&key, &tx]), tx, vout: 0, deposit, inscription_id: String::new(), proof: vec![] }
    }

    /// The canister wraps `asset` (claimed held by `holder`'s wallet) into `dest`.
    fn wrap_with(&mut self, asset: &Pubkey, holder: &Pubkey, n: u64, dest: &Pubkey) -> Res {
        let c = self.env.custodian.insecure_clone();
        let mut i = self.env.mint_item_ix(&c.pubkey(), Self::args(asset, n, false), dest);
        i.accounts.extend(self.custody_accounts(asset, holder));
        self.env.send(&[cu_limit(), i], &[&c])
    }

    fn wrap(&mut self, asset: &Pubkey, n: u64, dest: &Pubkey) -> Res {
        let c = self.env.custodian.pubkey();
        self.wrap_with(asset, &c, n, dest)
    }

    /// The record_delivery instruction for ticket `id`, with the destination's holding account.
    fn record_delivery_ix(&self, id: u64, holding: Pubkey) -> Instruction {
        let t = self.env.ticket(id);
        let item_pk = pda(&[ITEM, self.env.wrapper.as_ref(), &t.item_id.to_le_bytes()]);
        let item: Item = self.env.get(&item_pk);
        let mut i = ix(
            rook_core::instruction::RecordDelivery { tx: tx_hash(77) },
            rook_core::accounts::RecordDelivery {
                custodian: self.env.custodian.pubkey(),
                listing: self.env.listing,
                wrapper: self.env.wrapper,
                ticket: self.env.ticket_pda(id),
                item: item_pk,
                held: pda(&[HELD, self.env.wrapper.as_ref(), &item.key]),
            },
        );
        i.accounts.push(r(holding));
        i
    }

    /// The asset of ticket `id`'s resolved item.
    fn ticket_asset(&self, id: u64) -> Pubkey {
        let t = self.env.ticket(id);
        let item: Item = self.env.get(&pda(&[ITEM, self.env.wrapper.as_ref(), &t.item_id.to_le_bytes()]));
        Pubkey::new_from_array(item.key)
    }
}

fn holder(env: &mut Env) -> (Keypair, Pubkey) {
    let h = Keypair::new();
    env.svm.airdrop(&h.pubkey(), 100 * SOL).unwrap();
    let acc = env.token_account(&h.pubkey());
    (h, acc)
}

fn token_amount(env: &Env, ta: &Pubkey) -> u64 {
    env.svm.get_account(ta).map(|a| u64::from_le_bytes(a.data[64..72].try_into().unwrap())).unwrap_or(0)
}

fn core_owner_of(env: &Env, asset: &Pubkey) -> Pubkey {
    Pubkey::new_from_array(env.svm.get_account(asset).unwrap().data[1..33].try_into().unwrap())
}

/// Redeem one unit of `h` to its own wallet, resolve it, and return the ticket id.
fn redeem_resolve(sx: &mut Sol, h: &Keypair, acc: &Pubkey) -> u64 {
    let id = sx.env.wrapper().ticket_count;
    ok(sx.env.redeem(h, acc, Destination::Sol(h.pubkey())));
    ok(sx.env.resolve(id));
    id
}

#[test]
fn listing_a_solana_collection() {
    let mut env = Env::bare();
    let c = env.custodian.pubkey();
    let k = ListingKind::SolNft { collection: Pubkey::new_unique(), standard: NftStandard::Programmable };
    // no custody address off chain: the custody wallet is the custodian
    expect_err(env.list(k, "Mad Lads", "0x1111111111111111111111111111111111111111", c, c), RookError::Empty);
    expect_err(env.list(ListingKind::SolNft { collection: Pubkey::default(), standard: NftStandard::Legacy }, "X", "", c, c), RookError::BadInput);
    expect_err(env.list(k, "Mad Lads", "", c, Pubkey::default()), RookError::BadInput);
    // the other kinds still need their custody address
    expect_err(env.list(ListingKind::EthNft { chain_id: 1, collection: [1; 20] }, "M", "", c, c), RookError::Empty);
    ok(env.list(k, "Mad Lads", "", c, c));
    let l: Listing = env.get(&env.listing);
    assert_eq!(l.kind, k);
    assert_eq!(env.svm.get_account(&env.listing).unwrap().data.len(), 8 + Listing::INIT_SPACE);
    println!("Listing account: {} bytes", 8 + Listing::INIT_SPACE);
    // the poster is the canister, as for an EthNft listing
    env.init_wrapper();
    env.init_module(default_params());
    ok(env.post(FLOOR));
}

/// Legacy NFT: the canister's wallet holds it → `mint_item` → a holder redeems → the canister transfers it wallet to wallet
/// and records the delivery in the same transaction, checked on chain.
#[test]
fn legacy_mint_redeem_deliver() {
    let mut sx = sol(NftStandard::Legacy, &[]);
    let c = sx.env.custodian.pubkey();
    let asset = sx.nft(&c, false, None);
    let (h, acc) = holder(&mut sx.env);
    let res = ok(sx.wrap(&asset, 1, &acc));
    println!("mint_item (Legacy) CU: {}", res.compute_units_consumed);
    assert_eq!(sx.env.balance(&acc), ONE);
    expect_err(sx.wrap(&asset, 2, &acc), RookError::AlreadyHeld);
    sx.env.check_ledger();
    let id = redeem_resolve(&mut sx, &h, &acc);
    assert_eq!(sx.ticket_asset(id), asset);
    let dst = ata(&h.pubkey(), &asset);
    // recorded before the transfer: refused (the destination does not hold it)
    let early = sx.record_delivery_ix(id, ata(&c, &asset));
    let cu = sx.env.custodian.insecure_clone();
    expect_err(sx.env.send(&[early], &[&cu]), RookError::NotInCustody);
    let create = Instruction {
        program_id: ATA,
        accounts: vec![AccountMeta::new(c, true), w(dst), r(h.pubkey()), r(asset), r(system_program::ID), r(spl_token::ID)],
        data: vec![1],
    };
    let t = spl_token::instruction::transfer(&spl_token::ID, &ata(&c, &asset), &dst, &c, &[], 1).unwrap();
    let rec = sx.record_delivery_ix(id, dst);
    let m = ok(sx.env.send(&[create, t, rec], &[&cu]));
    println!("transfer + record_delivery (Legacy) CU: {}", m.compute_units_consumed);
    assert_eq!(token_amount(&sx.env, &dst), 1);
    assert_eq!(sx.env.ticket(id).stage, Stage::Delivered);
    assert_eq!(sx.env.wrapper().held, 0);
    sx.env.check_ledger();
}

/// pNFTs on the Metaplex Foundation rule set (its latest revision passes every operation), the Compatibility rule set and
/// none: custody checked, then a wallet-to-wallet Token Metadata `TransferV1` + `record_delivery` in one transaction.
#[test]
fn programmable_mint_redeem_deliver() {
    for rs in [Some(FOUNDATION), Some(COMPAT), None] {
        let mut sx = sol(NftStandard::Programmable, &[]);
        let c = sx.env.custodian.pubkey();
        let asset = sx.nft(&c, true, rs);
        // a legacy member of the same collection is accepted by a Token Metadata listing too
        let legacy = sx.nft(&c, false, None);
        let (h, acc) = holder(&mut sx.env);
        let res = ok(sx.wrap(&asset, 1, &acc));
        println!("mint_item (pNFT, {:?}) CU: {}", rs, res.compute_units_consumed);
        ok(sx.wrap(&legacy, 2, &acc));
        sx.env.check_ledger();
        let id = redeem_resolve(&mut sx, &h, &acc);
        let got = sx.ticket_asset(id);
        let prog = got == asset;
        let cu = sx.env.custodian.insecure_clone();
        let t = tm_transfer(&c, &h.pubkey(), &got, prog, if prog { rs } else { None });
        let rec = sx.record_delivery_ix(id, ata(&h.pubkey(), &got));
        let m = ok(sx.env.send(&[cu_limit(), t, rec], &[&cu]));
        println!("TM TransferV1 + record_delivery (prog {prog}, {:?}) CU: {}", rs, m.compute_units_consumed);
        assert_eq!(token_amount(&sx.env, &ata(&h.pubkey(), &got)), 1);
        assert_eq!(sx.env.ticket(id).stage, Stage::Delivered);
        sx.env.check_ledger();
    }
}

/// Core: an asset of the listing's collection held by the canister; delivered by mpl-core `TransferV1` + `record_delivery`.
#[test]
fn core_mint_redeem_deliver() {
    // a royalty deny list naming rook_core does not matter: the program moves nothing
    let mut deny = vec![0u8]; // Plugin::Royalties
    deny.extend_from_slice(&500u16.to_le_bytes());
    deny.extend_from_slice(&1u32.to_le_bytes()); // one creator at 100 %
    deny.extend_from_slice(Pubkey::new_unique().as_ref());
    deny.push(100);
    deny.push(2); // RuleSet::ProgramDenyList
    deny.extend_from_slice(&1u32.to_le_bytes());
    deny.extend_from_slice(rook_core::id().as_ref());
    deny.push(0); // authority: default
    let mut sx = sol(NftStandard::Core, &[deny]);
    let c = sx.env.custodian.pubkey();
    let asset = sx.nft(&c, false, None);
    let (h, acc) = holder(&mut sx.env);
    let res = ok(sx.wrap(&asset, 1, &acc));
    println!("mint_item (Core) CU: {}", res.compute_units_consumed);
    sx.env.check_ledger();
    let id = redeem_resolve(&mut sx, &h, &acc);
    let cu = sx.env.custodian.insecure_clone();
    expect_err(sx.env.send(&[sx.record_delivery_ix(id, asset)], &[&cu]), RookError::NotInCustody);
    let t = core_transfer(asset, sx.collection, &c, &h.pubkey());
    let rec = sx.record_delivery_ix(id, asset);
    let m = ok(sx.env.send(&[t, rec], &[&cu]));
    println!("Core TransferV1 + record_delivery CU: {}", m.compute_units_consumed);
    assert_eq!(core_owner_of(&sx.env, &asset), h.pubkey());
    assert_eq!(sx.env.ticket(id).stage, Stage::Delivered);
    sx.env.check_ledger();
}

/// Not a member, not in custody, not an NFT: refused with the reason.
#[test]
fn non_members_and_items_not_in_custody_are_refused() {
    let mut sx = sol(NftStandard::Legacy, &[]);
    let c = sx.env.custodian.pubkey();
    let (h, acc) = holder(&mut sx.env);
    let cr = sx.creator.insecure_clone();
    let coll = sx.collection;
    // unverified collection
    let unverified = tm_nft(&mut sx.env, &cr, NON_FUNGIBLE, Some(coll), false, None, &c, false);
    expect_err(sx.wrap(&unverified, 1, &acc), RookError::NotInCollection);
    // another collection
    let other_c = tm_nft(&mut sx.env, &cr, NON_FUNGIBLE, None, false, None, &cr.pubkey(), true);
    let other = tm_nft(&mut sx.env, &cr, NON_FUNGIBLE, Some(other_c), true, None, &c, false);
    expect_err(sx.wrap(&other, 2, &acc), RookError::NotInCollection);
    // the collection NFT itself is not a member
    let cr_pk = cr.pubkey();
    expect_err(sx.wrap_with(&coll, &cr_pk, 3, &acc), RookError::NotInCollection);
    // a member held by someone else's wallet
    let hp = h.pubkey();
    let elsewhere = sx.nft(&hp, false, None);
    expect_err(sx.wrap_with(&elsewhere, &hp, 4, &acc), RookError::NotInCustody);
    expect_err(sx.wrap(&elsewhere, 4, &acc), RookError::NotInCustody);
    // the key must be the asset passed
    let good = sx.nft(&c, false, None);
    let cu = sx.env.custodian.insecure_clone();
    let mut i = sx.env.mint_item_ix(&c, Sol::args(&elsewhere, 5, false), &acc);
    i.accounts.extend(sx.custody_accounts(&good, &c));
    expect_err(sx.env.send(&[i], &[&cu]), RookError::BadInput);
    // accounts missing
    let i = sx.env.mint_item_ix(&c, Sol::args(&good, 6, false), &acc);
    expect_err(sx.env.send(&[i], &[&cu]), RookError::MissingAccount);
    // a wrong master edition (another mint's)
    let mut i = sx.env.mint_item_ix(&c, Sol::args(&good, 7, false), &acc);
    i.accounts.extend(vec![r(good), r(metadata_of(&good)), r(edition_of(&other)), r(ata(&c, &good))]);
    expect_err(sx.env.send(&[i], &[&cu]), RookError::NotAnNft);
    // a fungible token is not an NFT: an SPL mint with supply 2
    let f = Keypair::new();
    let rent = sx.env.svm.minimum_balance_for_rent_exemption(82);
    let mk = anchor_lang::solana_program::system_instruction::create_account(&c, &f.pubkey(), rent, 82, &spl_token::ID);
    let init = spl_token::instruction::initialize_mint2(&spl_token::ID, &f.pubkey(), &c, None, 0).unwrap();
    let ca = Instruction {
        program_id: ATA,
        accounts: vec![AccountMeta::new(c, true), w(ata(&c, &f.pubkey())), r(c), r(f.pubkey()), r(system_program::ID), r(spl_token::ID)],
        data: vec![1],
    };
    let mt = spl_token::instruction::mint_to(&spl_token::ID, &f.pubkey(), &ata(&c, &f.pubkey()), &c, &[], 2).unwrap();
    ok(sx.env.send(&[mk, init, ca, mt], &[&cu, &f]));
    let mut i = sx.env.mint_item_ix(&c, Sol::args(&f.pubkey(), 8, false), &acc);
    i.accounts.extend(vec![r(f.pubkey()), r(metadata_of(&good)), r(edition_of(&good)), r(ata(&c, &f.pubkey()))]);
    expect_err(sx.env.send(&[i], &[&cu]), RookError::NotAnNft);
    ok(sx.wrap(&good, 9, &acc));
    assert_eq!(sx.env.wrapper().held, 1);
    sx.env.check_ledger();
}

/// cNFTs have no account (a Bubblegum asset id is an empty address); Core's compressed `HashedAssetV1` is refused; a Core
/// asset is not a Token Metadata member and the reverse.
#[test]
fn compressed_and_wrong_standard_are_refused() {
    let mut sx = sol(NftStandard::Core, &[]);
    let c = sx.env.custodian.pubkey();
    let (_h, acc) = holder(&mut sx.env);
    let cnft = Pubkey::new_unique();
    expect_err(sx.wrap(&cnft, 1, &acc), RookError::NotAnNft);
    let hashed = Pubkey::new_unique();
    let mut d = vec![2u8]; // Key::HashedAssetV1
    d.extend_from_slice(&[0u8; 40]);
    let lamports = sx.env.svm.minimum_balance_for_rent_exemption(d.len());
    sx.env.svm.set_account(hashed, solana_account::Account { lamports, data: d, owner: CORE, executable: false, rent_epoch: 0 }).unwrap();
    expect_err(sx.wrap(&hashed, 2, &acc), RookError::NotAnNft);
    // a Token Metadata NFT on a Core listing
    let cr = sx.creator.insecure_clone();
    let tm_c = tm_nft(&mut sx.env, &cr, NON_FUNGIBLE, None, false, None, &cr.pubkey(), true);
    let tm = tm_nft(&mut sx.env, &cr, NON_FUNGIBLE, Some(tm_c), true, None, &c, false);
    expect_err(sx.wrap(&tm, 3, &acc), RookError::NotAnNft);

    let mut st = sol(NftStandard::Legacy, &[]);
    let c2 = st.env.custodian.pubkey();
    let (_h2, acc2) = holder(&mut st.env);
    let cr2 = st.creator.insecure_clone();
    let cc = core_collection(&mut st.env, &cr2, &[]);
    let ca = core_asset(&mut st.env, &cr2, cc, &c2, &[]);
    let cu = st.env.custodian.insecure_clone();
    let mut i = st.env.mint_item_ix(&c2, Sol::args(&ca, 4, false), &acc2);
    i.accounts.extend(vec![r(ca), r(metadata_of(&ca)), r(edition_of(&ca)), r(ata(&c2, &ca))]);
    expect_err(st.env.send(&[i], &[&cu]), RookError::NotAnNft);
}

/// Core plugins that could freeze, take or burn an item in custody, or veto its transfer, are refused.
#[test]
fn core_permanent_delegates_are_refused() {
    let permanent_freeze = vec![5u8, 0, 0]; // PermanentFreezeDelegate { frozen: false }, default authority
    let permanent_transfer = vec![7u8, 0]; // PermanentTransferDelegate
    let mut sx = sol(NftStandard::Core, &[]);
    let (_h, acc) = holder(&mut sx.env);
    let c = sx.env.custodian.pubkey();
    let cr = sx.creator.insecure_clone();
    let coll = sx.collection;
    let a = core_asset(&mut sx.env, &cr, coll, &c, &[permanent_freeze]);
    expect_err(sx.wrap(&a, 1, &acc), RookError::UnsupportedNft);
    // an owner-managed transfer delegate held by someone else
    let mut td = vec![3u8, 1, 3]; // TransferDelegate, Some(Address)
    td.extend_from_slice(Pubkey::new_unique().as_ref());
    let b = core_asset(&mut sx.env, &cr, coll, &c, &[td]);
    expect_err(sx.wrap(&b, 2, &acc), RookError::UnsupportedNft);
    let ok_asset = core_asset(&mut sx.env, &cr, coll, &c, &[vec![6u8, 0, 0, 0, 0, 0]]); // Attributes { [] }
    ok(sx.wrap(&ok_asset, 3, &acc));
    // a collection created with a permanent delegate refuses every member
    let mut s2 = sol(NftStandard::Core, &[permanent_transfer]);
    let (_h2, acc2) = holder(&mut s2.env);
    let c2 = s2.env.custodian.pubkey();
    let a2 = s2.nft(&c2, false, None);
    expect_err(s2.wrap(&a2, 1, &acc2), RookError::UnsupportedNft);
}

/// Redemption destinations of a Solana listing: a Solana wallet, never the custody wallet; no burn address, no other chain.
#[test]
fn redeem_destinations() {
    let mut sx = sol(NftStandard::Legacy, &[]);
    let c = sx.env.custodian.pubkey();
    let asset = sx.nft(&c, false, None);
    let (h, acc) = holder(&mut sx.env);
    ok(sx.wrap(&asset, 1, &acc));
    expect_err(sx.env.redeem(&h, &acc, Destination::Sol(c)), RookError::CustodyDestination);
    expect_err(sx.env.redeem(&h, &acc, Destination::Sol(Pubkey::default())), RookError::BadDestination);
    expect_err(sx.env.redeem(&h, &acc, Destination::Burn), RookError::BadDestination);
    expect_err(sx.env.redeem(&h, &acc, Destination::Eth([4; 20])), RookError::BadDestination);
    ok(sx.env.redeem(&h, &acc, Destination::Sol(h.pubkey())));
    // and an EthNft listing takes no Solana destination
    let mut env = Env::new();
    let (h2, acc2) = holder(&mut env);
    ok(env.wrap_nft(1, tx_hash(1), &acc2));
    expect_err(env.redeem(&h2, &acc2, Destination::Sol(h2.pubkey())), RookError::BadDestination);
}

/// The canister's purchase path against an advance, unchanged: `advance` → buy on Tensor / Magic Eden into its wallet →
/// `mint_item` into the module's account (custody checked on chain) → `settle_advance`.
#[test]
fn purchase_path_against_an_advance() {
    let mut sx = sol(NftStandard::Programmable, &[]);
    let u = sx.env.user.insecure_clone();
    ok(sx.env.post(FLOOR));
    let uacc = sx.env.token_account(&u.pubkey());
    ok(sx.env.mint_units(&u, &uacc, 1_000 * SOL, 0));
    let m = sx.env.module();
    let e = sx.env.wrapper().eth_backed;
    let max = m.max_item_cost(sx.env.now, e, false).unwrap();
    assert!(max > 0);
    ok(sx.env.advance(max));
    let c = sx.env.custodian.pubkey();
    // the marketplace sale lands the pNFT in the canister's wallet (here: minted there)
    let asset = sx.nft(&c, true, Some(FOUNDATION));
    let mwx = sx.env.module_wx;
    ok(sx.wrap(&asset, 1, &mwx));
    ok(sx.env.settle(max - SOL));
    let w = sx.env.wrapper();
    assert_eq!((w.held, w.eth_backed), (1, e - ONE));
    assert_eq!(sx.env.module().transit_units, 0);
    sx.env.check_books();
    sx.env.check_ledger();
    // the ticket path still runs for that unit: the user's whole wX redeems the bought pNFT
    let id = redeem_resolve(&mut sx, &u, &uacc);
    assert_eq!(sx.ticket_asset(id), asset);
    let cu = sx.env.custodian.insecure_clone();
    let t = tm_transfer(&c, &u.pubkey(), &asset, true, Some(FOUNDATION));
    let rec = sx.record_delivery_ix(id, ata(&u.pubkey(), &asset));
    ok(sx.env.send(&[cu_limit(), t, rec], &[&cu]));
    sx.env.check_books();
    sx.env.check_ledger();
}

// ---- the treasury at 64 collections ----------------------------------------------------------------------------------

fn set_volume(env: &mut Env, module: &Pubkey, v: u64) {
    use anchor_lang::AccountSerialize;
    let mut m: Module = env.get(module);
    let day = (env.now / 86_400) as u64;
    m.day_of[(day % 7) as usize] = day;
    m.day_volume[(day % 7) as usize] = v;
    let mut acc = env.svm.get_account(module).unwrap();
    let mut d = Vec::new();
    m.try_serialize(&mut d).unwrap();
    acc.data[..d.len()].copy_from_slice(&d);
    env.svm.set_account(*module, acc).unwrap();
}

fn weigh(env: &mut Env, start: u8, modules: &[Pubkey]) -> Res {
    let mut i = ix(rook_core::instruction::WeighDistribution { start }, rook_core::accounts::BeginDistribution { treasury: env.treasury });
    i.accounts.extend(modules.iter().map(|m| r(*m)));
    let u = env.user.insecure_clone();
    env.send(&[i], &[&u])
}

#[test]
fn treasury_distributes_to_64_collections_in_pages() {
    use rook_core::state::MAX_COLLECTIONS;
    assert_eq!(MAX_COLLECTIONS, 64);
    let mut env = Env::bare();
    let c = env.custodian.pubkey();
    let o = env.owner.insecure_clone();
    let mut modules = vec![];
    for n in 0..65u32 {
        ok(env.list(ListingKind::SolNft { collection: Pubkey::new_unique(), standard: NftStandard::Core }, &format!("C{n}"), "", c, c));
        env.init_wrapper();
        env.init_module(default_params());
        let add = ix(
            rook_core::instruction::AddCollection {},
            rook_core::accounts::AddCollection { owner: o.pubkey(), registry: env.registry, treasury: env.treasury, module: env.module },
        );
        let res = env.send(&[add], &[&o]);
        if n < 64 {
            ok(res);
            modules.push(env.module);
        } else {
            expect_err(res, RookError::Full);
        }
    }
    let t: Treasury = env.get(&env.treasury);
    assert_eq!(t.collections.len(), 64);
    let size = env.svm.get_account(&env.treasury).unwrap().data.len();
    assert_eq!(size, 8 + Treasury::INIT_SPACE);
    println!("Treasury account: {} bytes, rent {} lamports", size, env.svm.minimum_balance_for_rent_exemption(size));
    // weights 1..=64 (the last collection's index 63 tests the top bit of `round_paid`)
    for (i, m) in modules.iter().enumerate() {
        set_volume(&mut env, m, (i as u64 + 1) * SOL);
    }
    let u = env.user.insecure_clone();
    let pot = 64 * 65 / 2 * SOL / 1_000;
    let i = ix(
        rook_core::instruction::Receive { lamports: pot },
        rook_core::accounts::Receive { payer: u.pubkey(), treasury: env.treasury, system_program: system_program::ID },
    );
    ok(env.send(&[i], &[&u]));
    let i = ix(rook_core::instruction::Allocate {}, rook_core::accounts::Allocate { treasury: env.treasury, ops: None });
    ok(env.send(&[i], &[&u]));
    let reserves = env.get::<Treasury>(&env.treasury).reserves_pot;
    // pages of 16 (a legacy transaction carries about 30 keys; an address lookup table carries 61)
    expect_err(weigh(&mut env, 16, &modules[16..32]), RookError::NothingPending);
    let r0 = ok(weigh(&mut env, 0, &modules[..16]));
    println!("weigh_distribution, 16 modules: {} CU", r0.compute_units_consumed);
    assert_eq!(env.get::<Treasury>(&env.treasury).weigh_next, 16);
    expect_err(weigh(&mut env, 32, &modules[32..48]), RookError::OutOfOrder);
    expect_err(weigh(&mut env, 0, &modules[..16]), RookError::RoundOpen);
    let mut i = ix(rook_core::instruction::BeginDistribution {}, rook_core::accounts::BeginDistribution { treasury: env.treasury });
    i.accounts.extend(modules[..16].iter().map(|m| r(*m)));
    expect_err(env.send(&[i], &[&u]), RookError::BadInput);
    // nothing is owed while weighing
    let dist = |env: &mut Env, idx: usize| {
        let listing = env.get::<Module>(&modules[idx]).listing;
        let i = ix(
            rook_core::instruction::Distribute { index: idx as u8 },
            rook_core::accounts::Distribute {
                treasury: env.treasury,
                module: modules[idx],
                module_vault: pda(&[MODULE_VAULT, modules[idx].as_ref()]),
                ops: None,
                raffle: pda(&[RAFFLE, listing.as_ref()]),
            },
        );
        let u = env.user.insecure_clone();
        env.send(&[i], &[&u])
    };
    expect_err(dist(&mut env, 0), RookError::NothingToDo);
    ok(weigh(&mut env, 16, &modules[16..32]));
    ok(weigh(&mut env, 32, &modules[32..48]));
    ok(weigh(&mut env, 48, &modules[48..64]));
    let t: Treasury = env.get(&env.treasury);
    assert_eq!((t.weigh_next, t.round_total, t.round_pot, t.round_paid), (0, 64 * 65 / 2 * SOL, reserves, 0));
    for idx in 0..64 {
        let before = env.get::<Module>(&modules[idx]).surplus;
        ok(dist(&mut env, idx));
        let got = env.get::<Module>(&modules[idx]).surplus - before;
        assert_eq!(got, reserves * (idx as u64 + 1) / (64 * 65 / 2), "index {idx}");
    }
    expect_err(dist(&mut env, 63), RookError::NothingToDo);
    let t: Treasury = env.get(&env.treasury);
    assert_eq!(t.round_paid, u64::MAX);
    assert!(!t.round_open().unwrap());
}
