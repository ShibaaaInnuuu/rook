//! The books' test harness: rook_core's LiteSVM `Env` (solana/programs/rook_core/tests/suite/common.rs) with the account
//! plumbing deleted. The canister's [`Books`] decide; a small simulated chain executes what each operation returns (lamports at
//! each module address `M[i]` and at `K_tre`, wX balances and supply per collection, wallets) so the tests check the books
//! against "the chain" exactly as rook_core's `check_books` / `check_ledger` checked accounts.
#![allow(dead_code)]

use std::collections::BTreeMap;

use etch_autobuyer::books::ledger::MintItemArgs;
use etch_autobuyer::books::math::{keccak, round_at, DRAND_GENESIS, DRAND_PERIOD};
use etch_autobuyer::books::*;

pub const T0: i64 = 1_760_000_000;
pub const SOL: u64 = 1_000_000_000;
/// A floor of 10 ETH ≈ 200 SOL per whole unit.
pub const FLOOR: u64 = 200 * SOL;
pub const REVENUE_TARGET: u64 = 20 * SOL;
pub const ID: u16 = 0;

pub const USER: [u8; 32] = [0x11; 32];
pub const HOLDER: [u8; 32] = [0x22; 32];
pub const CUSTODIAN: [u8; 32] = [0x33; 32];
/// The raffle's prize token account at `K_tre`.
pub const PRIZE: [u8; 32] = [0x44; 32];
pub const CUSTODY_EVM: [u8; 20] = [0x11; 20];

pub fn test_params() -> Params {
    etch_autobuyer::books::default_params(REVENUE_TARGET)
}

pub fn tx_hash(n: u64) -> [u8; 32] {
    let mut k = [0xAAu8; 32];
    k[24..].copy_from_slice(&n.to_be_bytes());
    k
}

pub fn token_id(n: u64) -> [u8; 32] {
    let mut k = [0u8; 32];
    k[24..].copy_from_slice(&n.to_be_bytes());
    k
}

pub fn wallet(n: u64) -> [u8; 32] {
    let mut k = [0x77u8; 32];
    k[..8].copy_from_slice(&n.to_le_bytes());
    k
}

pub fn eth_args(token: u64, tx: [u8; 32]) -> MintItemArgs {
    let key = token_id(token);
    MintItemArgs { key, source: keccak(&[&key, &tx]), tx, vout: 0, deposit: false, inscription_id: String::new(), proof: vec![] }
}

pub fn eth_kind() -> ListingKind {
    ListingKind::EthNft { chain_id: 1, collection: [0x5a; 20], custody: CUSTODY_EVM }
}

#[track_caller]
pub fn ok<T: std::fmt::Debug>(r: Res<T>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("expected success, got {e:?}"),
    }
}

#[track_caller]
pub fn expect_err<T: std::fmt::Debug>(r: Res<T>, want: BooksError) {
    match r {
        Ok(v) => panic!("expected {want:?}, got Ok({v:?})"),
        Err(e) => assert_eq!(e, want),
    }
}

pub struct Env {
    pub b: Books,
    /// The collection the helpers act on (rook_core `use_listing`).
    pub id: u16,
    pub now: i64,
    pub slot: u64,
    /// Lamports above rent at each module address `M[i]`.
    pub vault: BTreeMap<u16, u64>,
    /// Lamports above rent at `K_tre`.
    pub tre: u64,
    /// Lamports received (+) and spent (−) per wallet.
    pub sol: BTreeMap<[u8; 32], i128>,
    /// wX balances per (collection, owner) and the supply per collection.
    pub wx: BTreeMap<(u16, [u8; 32]), u64>,
    pub supply: BTreeMap<u16, u64>,
    /// drand rounds the canister has verified.
    pub drand: BTreeMap<u64, [u8; 32]>,
}

impl Env {
    pub fn new() -> Env {
        Env::with_params(test_params())
    }

    pub fn with_params(p: Params) -> Env {
        let mut env = Env::bare();
        ok(env.b.add_collection(ID, eth_kind(), [0xC0; 32], p));
        env
    }

    /// No collection yet.
    pub fn bare() -> Env {
        Env {
            b: Books::new(),
            id: ID,
            now: T0,
            slot: 1_000,
            vault: BTreeMap::new(),
            tre: 0,
            sol: BTreeMap::new(),
            wx: BTreeMap::new(),
            supply: BTreeMap::new(),
            drand: BTreeMap::new(),
        }
    }

    /// Another EthNft collection, made current.
    pub fn add_collection(&mut self, id: u16, p: Params) {
        let mut mint = [0xC0; 32];
        mint[0] = id as u8;
        ok(self.b.add_collection(id, ListingKind::EthNft { chain_id: 1, collection: [id as u8; 20], custody: CUSTODY_EVM }, mint, p));
        self.id = id;
    }

    pub fn use_listing(&mut self, id: u16) {
        self.id = id;
    }

    pub fn module(&self) -> Module {
        self.b.get(self.id).unwrap().module.clone()
    }

    pub fn module_of(&self, id: u16) -> Module {
        self.b.get(id).unwrap().module.clone()
    }

    pub fn ledger(&self) -> Ledger {
        self.b.get(self.id).unwrap().ledger.clone()
    }

    pub fn e(&self) -> u64 {
        self.ledger().sol_backed
    }

    pub fn raffle(&self) -> Raffle {
        self.b.get(self.id).unwrap().raffle.clone().expect("a raffle")
    }

    pub fn ticket(&self, id: u64) -> Ticket {
        self.ledger().tickets[&id].clone()
    }

    pub fn set_time(&mut self, t: i64) {
        self.now = t;
        self.slot += 1;
    }

    pub fn advance_time(&mut self, dt: i64) {
        let t = self.now + dt;
        self.set_time(t);
    }

    pub fn reach_round(&mut self, round: u64) {
        if round_at(self.now) < round {
            self.set_time(DRAND_GENESIS + (round as i64 - 1) * DRAND_PERIOD);
        }
    }

    /// The canister verified drand round `round` (refused for a round not live yet, as the BLS check of a future round cannot pass).
    pub fn prove(&mut self, round: u64, randomness: [u8; 32]) -> Res<()> {
        if round > round_at(self.now) {
            return Err(BooksError::RoundInFuture);
        }
        self.drand.insert(round, randomness);
        Ok(())
    }

    // ---- the chain -------------------------------------------------------------------------------------------------------

    pub fn vault_free(&self) -> u64 {
        *self.vault.get(&self.id).unwrap_or(&0)
    }

    pub fn vault_of(&self, id: u16) -> u64 {
        *self.vault.get(&id).unwrap_or(&0)
    }

    fn vault_in(&mut self, id: u16, l: u64) {
        *self.vault.entry(id).or_default() += l;
    }

    fn vault_out(&mut self, id: u16, l: u64) {
        let v = self.vault.entry(id).or_default();
        *v = v.checked_sub(l).expect("the module address cannot pay more than it holds");
    }

    pub fn lamports(&self, who: &[u8; 32]) -> i128 {
        *self.sol.get(who).unwrap_or(&0)
    }

    fn pay(&mut self, who: [u8; 32], l: i128) {
        *self.sol.entry(who).or_default() += l;
    }

    pub fn balance(&self, owner: &[u8; 32]) -> u64 {
        *self.wx.get(&(self.id, *owner)).unwrap_or(&0)
    }

    pub fn supply(&self) -> u64 {
        *self.supply.get(&self.id).unwrap_or(&0)
    }

    fn mint_to(&mut self, owner: [u8; 32], units: u64) {
        *self.wx.entry((self.id, owner)).or_default() += units;
        *self.supply.entry(self.id).or_default() += units;
    }

    fn burn_from(&mut self, owner: [u8; 32], units: u64) {
        let b = self.wx.entry((self.id, owner)).or_default();
        *b = b.checked_sub(units).expect("an SPL burn of more than the balance fails");
        *self.supply.entry(self.id).or_default() -= units;
    }

    pub fn transfer_wx(&mut self, from: [u8; 32], to: [u8; 32], units: u64) {
        let b = self.wx.entry((self.id, from)).or_default();
        *b = b.checked_sub(units).expect("transfer");
        *self.wx.entry((self.id, to)).or_default() += units;
    }

    // ---- invariants (rook_core check_books / check_ledger) ---------------------------------------------------------------

    #[track_caller]
    pub fn check_books(&self) {
        for (id, c) in &self.b.collections {
            ok(books_balance(&c.module, self.vault_of(*id)));
        }
        ok(treasury_balance(&self.b, self.tre));
    }

    #[track_caller]
    pub fn check_ledger(&self) {
        for (id, c) in &self.b.collections {
            ok(supply_matches(&c.ledger, *self.supply.get(id).unwrap_or(&0)));
            ok(no_free_profit(&c.module, self.now, c.ledger.sol_backed, c.kd(self.b.keeper_paused)));
        }
    }

    // ---- the floor -------------------------------------------------------------------------------------------------------

    pub fn post(&mut self, price: u64) -> Res<(u64, u64)> {
        self.post_with(price, 9_000, 20)
    }

    pub fn post_with(&mut self, price: u64, conf: u16, depth: u32) -> Res<(u64, u64)> {
        self.b.post_floor(self.id, price, conf, depth, self.now)
    }

    /// `mint_units`: `who` pays `lamports` into `M[i]` and receives wX.
    pub fn mint_units(&mut self, who: [u8; 32], lamports: u64, min_units: u64) -> Res<u64> {
        let m = self.b.mint(self.id, lamports, min_units, self.now)?;
        self.pay(who, -(lamports as i128));
        self.vault_in(self.id, lamports);
        self.mint_to(who, m.units);
        Ok(m.units)
    }

    /// `burn_units`: `who` burns `units`; `to` receives the lamports. Refused like the SPL burn when `who` holds less.
    pub fn burn_units(&mut self, who: [u8; 32], to: [u8; 32], units: u64, min_lamports: u64) -> Res<u64> {
        if self.balance(&who) < units {
            return Err(BooksError::BadInput);
        }
        let r = self.b.burn(self.id, units, min_lamports, self.now)?;
        self.burn_from(who, units);
        self.vault_out(self.id, r.paid);
        self.pay(to, r.paid as i128);
        Ok(r.paid)
    }

    /// A fresh floor is assumed; `lamports` of units minted to USER.
    pub fn user_with_units(&mut self, lamports: u64) -> u64 {
        ok(self.mint_units(USER, lamports, 0))
    }

    pub fn advance(&mut self, amount: u64) -> Res<u64> {
        let r = self.b.advance(self.id, amount, self.now)?;
        self.vault_out(self.id, amount);
        self.pay(CUSTODIAN, amount as i128);
        Ok(r)
    }

    pub fn settle(&mut self, cost: u64) -> Res<u64> {
        self.b.settle_advance(self.id, cost, self.now)
    }

    pub fn item_arrived(&mut self, cost: u64) -> Res<u64> {
        let r = self.b.item_arrived(self.id, cost, self.now)?;
        self.vault_out(self.id, cost);
        self.pay(CUSTODIAN, cost as i128);
        Ok(r)
    }

    pub fn return_advance(&mut self, lamports: u64) -> Res<(u64, u64)> {
        let r = self.b.return_advance(self.id, lamports)?;
        self.pay(CUSTODIAN, -(lamports as i128));
        self.vault_in(self.id, lamports);
        Ok(r)
    }

    pub fn snapshot(&mut self) -> Res<u64> {
        self.b.snapshot_cap(self.id, self.now)
    }

    pub fn fund_reserve(&mut self, lamports: u64) -> Res<()> {
        self.b.fund_reserve(self.id, lamports)?;
        self.vault_in(self.id, lamports);
        Ok(())
    }

    pub fn skim(&mut self) -> Res<u64> {
        let a = self.b.skim(self.id, self.now)?;
        self.vault_out(self.id, a);
        self.tre += a;
        Ok(a)
    }

    pub fn pause_poster(&mut self) {
        ok(self.b.set_poster_paused(self.id, true));
    }

    pub fn unpause_poster(&mut self) {
        ok(self.b.set_poster_paused(self.id, false));
    }

    /// SOL-backed units at the grad price's level, the way rook_core's tests used `convert`: `lamports` of public mints, an
    /// hour apart when the bucket is full, re-posting the floor each hour.
    pub fn seed_units(&mut self, who: [u8; 32], lamports: u64) {
        let mut left = lamports;
        while left > 0 {
            let room = self.module().mint_room(self.now).unwrap();
            let price = self.module().mint_price(self.now, self.e()).unwrap();
            let can = (room as u128 * price as u128 / ONE as u128) as u64;
            let pay = left.min(can.saturating_sub(SOL / 1_000));
            if pay < SOL {
                self.advance_time(3_600);
                ok(self.post(self.module().accepted));
                continue;
            }
            ok(self.mint_units(who, pay, 0));
            left -= pay;
        }
    }

    // ---- items and tickets -------------------------------------------------------------------------------------------------

    pub fn mint_item(&mut self, args: MintItemArgs, to: MintTo) -> Res<u32> {
        let r = self.b.mint_item(self.id, args, to, self.now)?;
        if let MintTo::Holder(h) = to {
            self.mint_to(h, ONE);
        }
        Ok(r)
    }

    pub fn wrap_nft(&mut self, token: u64, tx: [u8; 32], to: MintTo) -> Res<u32> {
        self.mint_item(eth_args(token, tx), to)
    }

    /// `redeem`: the books first, then the redeemer's ONE wX is burned on chain.
    pub fn redeem(&mut self, who: [u8; 32], d: Destination) -> Res<u64> {
        if self.balance(&who) < ONE {
            return Err(BooksError::BadInput);
        }
        let t = self.b.redeem(self.id, who, d, self.now)?;
        self.burn_from(who, ONE);
        Ok(t)
    }

    /// Prove the ticket's round (if not yet; moving the clock to it if needed) and resolve it.
    pub fn resolve(&mut self, id: u64) -> Res<Resolution> {
        let t = self.ticket(id);
        if !self.drand.contains_key(&t.round) {
            self.reach_round(t.round);
            let mut r = [7u8; 32];
            r[..8].copy_from_slice(&t.round.to_le_bytes());
            ok(self.prove(t.round, r));
        }
        let rnd = self.drand.get(&t.round).copied();
        self.b.resolve(self.id, id, rnd, self.now)
    }

    /// `resolve` with whatever the canister has verified for the ticket's round (`None` if nothing).
    pub fn resolve_as_is(&mut self, id: u64) -> Res<Resolution> {
        let rnd = self.b.get(self.id).unwrap().ledger.tickets.get(&id).and_then(|t| self.drand.get(&t.round).copied());
        self.b.resolve(self.id, id, rnd, self.now)
    }

    pub fn record_delivery(&mut self, id: u64, tx: [u8; 32]) -> Res<()> {
        self.b.record_delivery(self.id, id, tx)
    }

    pub fn note_sent(&mut self, id: u64, tx: [u8; 32]) -> Res<()> {
        self.b.note_sent(self.id, id, tx)
    }

    pub fn mark_undeliverable(&mut self, id: u64) -> Res<()> {
        self.b.mark_undeliverable(self.id, id)
    }

    /// `refund`: the canister re-mints ONE to the ticket's owner.
    pub fn refund(&mut self, id: u64) -> Res<[u8; 32]> {
        let proven = self.b.get(self.id).unwrap().ledger.tickets.get(&id).is_some_and(|t| self.drand.contains_key(&t.round));
        let owner = self.b.refund(self.id, id, proven, self.now)?;
        self.mint_to(owner, ONE);
        Ok(owner)
    }

    // ---- the treasury ------------------------------------------------------------------------------------------------------

    pub fn receive(&mut self, lamports: u64) -> Res<()> {
        self.b.receive(lamports)?;
        self.tre += lamports;
        Ok(())
    }

    pub fn allocate(&mut self) -> Res<Allocation> {
        let a = self.b.allocate()?;
        self.tre -= a.to_ops;
        if a.to_ops != 0 {
            let ops = self.b.treasury.ops.expect("an ops address");
            self.pay(ops, a.to_ops as i128);
        }
        Ok(a)
    }

    pub fn begin(&mut self) -> Res<()> {
        self.b.begin_distribution(self.now)
    }

    pub fn distribute(&mut self, index: u8) -> Res<Distribution> {
        let d = self.b.distribute(index)?;
        self.apply_distribution(&d);
        Ok(d)
    }

    pub fn apply_distribution(&mut self, d: &Distribution) {
        self.tre -= d.to_surplus + d.to_ops;
        self.vault_in(d.collection, d.to_surplus);
        if d.to_ops != 0 {
            let ops = self.b.treasury.ops.expect("an ops address");
            self.pay(ops, d.to_ops as i128);
        }
    }

    /// Income `lamports` → allocate → a round over the registered collections.
    pub fn fund_round(&mut self, lamports: u64) {
        ok(self.receive(lamports));
        ok(self.allocate());
        ok(self.begin());
    }

    pub fn cap_of(&self) -> u64 {
        self.module().accepted * self.b.treasury.reserve_cap_bps as u64 / 10_000
    }

    // ---- the raffle ----------------------------------------------------------------------------------------------------------

    /// Lamports booked straight into the raffle's pot (rook_core `top_up`).
    pub fn top_up(&mut self, lamports: u64) {
        ok(self.b.fund_raffle(self.id, lamports));
        self.tre += lamports;
    }

    pub fn fund_prize(&mut self) -> Res<PrizeFunded> {
        let p = self.b.fund_prize(self.id, self.now)?;
        self.tre -= p.price;
        self.vault_in(self.id, p.price);
        self.mint_to(PRIZE, p.units);
        Ok(p)
    }

    pub fn commit(&mut self, root: [u8; 32], total: u64, snapshot_slot: u64, entries: u32) -> Res<u64> {
        self.b.commit_raffle(self.id, root, total, snapshot_slot, entries, self.slot, self.now)
    }

    pub fn claim(&mut self, owner: [u8; 32], cum_start: u64, weight: u64, proof: &[[u8; 32]]) -> Res<u64> {
        let u = self.b.claim_raffle(self.id, owner, cum_start, weight, proof, self.now)?;
        self.transfer_wx(PRIZE, owner, u);
        Ok(u)
    }
}

/// xorshift64 (rook_core props.rs).
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}
