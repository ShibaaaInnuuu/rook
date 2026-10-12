//! The LiteSVM harness: one environment with the registry, an EthNft listing (the canister = a fake custodian keypair, which
//! is also its poster), its wrapper and floor module, the treasury and the wake desk. Builders for every instruction.
#![allow(dead_code)]

use anchor_lang::{
    prelude::{Clock, Pubkey},
    solana_program::{instruction::Instruction, system_instruction, system_program},
    AccountDeserialize, InstructionData, ToAccountMetas,
};
use anchor_spl::token::spl_token;
use litesvm::{types::TransactionMetadata, LiteSVM};
use rook_core::{
    error::RookError,
    instructions::{ListArgs, MintItemArgs},
    seeds::*,
    state::*,
    params::Params,
    units::ONE,
};
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_signer::Signer;
use solana_transaction::versioned::VersionedTransaction;

pub const T0: i64 = 1_760_000_000;
pub const SOL: u64 = 1_000_000_000;
/// A floor of 10 ETH ≈ 200 SOL per whole unit.
pub const FLOOR: u64 = 200 * SOL;
/// The tests' launch values (decision D-c: rook_core holds no SOL/ETH rate; production takes them from the launch file,
/// `ops/solana/launch-params.mjs`). These are the old placeholders at 20 SOL per ETH: `revenueTarget` 1 ETH, the wake bond
/// 0.002 ETH within [0.0005, 1] ETH.
pub const REVENUE_TARGET: u64 = 20 * SOL;
pub const WAKE_BOND: u64 = 40_000_000;
pub const MIN_BOND: u64 = 10_000_000;
pub const MAX_BOND: u64 = 20 * SOL;

/// `StackChecks.defaultParams` at the tests' revenue target.
pub fn default_params() -> Params {
    rook_core::params::default_params(REVENUE_TARGET)
}

pub fn desk_params() -> DeskParams {
    DeskParams { bond: WAKE_BOND, min_bond: MIN_BOND, max_bond: MAX_BOND, window: 7 * 86_400 }
}

/// rook_core as LiteSVM 0.10 can load it. `anchor build` (1.2.1) emits SBPF v3 by default, which LiteSVM 0.10 refuses
/// (InvalidAccountData), so when target/deploy/rook_core.so is v3 the tests run the same source built for SBPF v2:
///   cargo build-sbf --manifest-path programs/rook_core/Cargo.toml --arch v2 --sbf-out-dir target/deploy-sbpfv2
/// Metaplex Token Metadata (`init_wrapper` creates the wX mint's metadata), the mainnet program dumped for spike Q1.
pub fn mpl_token_metadata_so() -> Vec<u8> {
    let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/mpl_token_metadata.so");
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The wX mint's Metaplex metadata PDA `["metadata", MPL, mint]`.
pub fn metadata_pda(mint: &Pubkey) -> Pubkey {
    use rook_core::metadata::{METADATA_SEED, MPL_TOKEN_METADATA_ID};
    Pubkey::find_program_address(&[METADATA_SEED, MPL_TOKEN_METADATA_ID.as_ref(), mint.as_ref()], &MPL_TOKEN_METADATA_ID).0
}

/// A fresh SVM with rook_core and Metaplex Token Metadata.
pub fn new_svm() -> LiteSVM {
    let mut svm = LiteSVM::new();
    svm.add_program(rook_core::id(), &rook_core_so()).unwrap();
    svm.add_program(rook_core::metadata::MPL_TOKEN_METADATA_ID, &mpl_token_metadata_so()).unwrap();
    svm
}

pub fn rook_core_so() -> Vec<u8> {
    let target = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("..");
    if let Ok(b) = std::fs::read(target.join("deploy/rook_core.so")) {
        if b.len() > 52 && b[48] != 3 {
            return b;
        }
    }
    std::fs::read(target.join("deploy-sbpfv2/rook_core.so")).expect(
        "build the SBPF v2 test binary first: cargo build-sbf --manifest-path programs/rook_core/Cargo.toml --arch v2 --sbf-out-dir target/deploy-sbpfv2",
    )
}

pub fn pda(seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &rook_core::id()).0
}

pub fn token_id(n: u64) -> [u8; 32] {
    let mut k = [0u8; 32];
    k[24..].copy_from_slice(&n.to_be_bytes());
    k
}

pub fn tx_hash(n: u64) -> [u8; 32] {
    let mut k = [0xAAu8; 32];
    k[24..].copy_from_slice(&n.to_be_bytes());
    k
}

pub type Res = Result<TransactionMetadata, (String, TransactionMetadata)>;

/// The custom error code a refusal carries.
pub fn code(e: RookError) -> u32 {
    e as u32 + 6000
}

#[track_caller]
pub fn expect_err(r: Res, e: RookError) {
    match r {
        Ok(_) => panic!("expected {:?}, got success", e),
        Err((s, meta)) => {
            let want = format!("Custom({})", code(e));
            assert!(s.contains(&want), "expected {:?} ({}), got {} / logs {:#?}", e, want, s, meta.logs);
        }
    }
}

#[track_caller]
pub fn expect_fail(r: Res) {
    assert!(r.is_err(), "expected a failure");
}

#[track_caller]
pub fn ok(r: Res) -> TransactionMetadata {
    match r {
        Ok(m) => m,
        Err((s, meta)) => panic!("failed: {} / logs {:#?}", s, meta.logs),
    }
}

pub struct Env {
    pub svm: LiteSVM,
    pub owner: Keypair,
    pub keeper: Keypair,
    pub guardian: Keypair,
    pub custodian: Keypair,
    pub user: Keypair,
    pub registry: Pubkey,
    pub listing: Pubkey,
    pub wrapper: Pubkey,
    pub wx_mint: Pubkey,
    pub item_set: Pubkey,
    pub module: Pubkey,
    pub vault: Pubkey,
    pub module_wx: Pubkey,
    pub treasury: Pubkey,
    pub desk: Pubkey,
    pub now: i64,
    pub listing_id: u32,
}

pub fn ix<D: InstructionData, A: ToAccountMetas>(data: D, accounts: A) -> Instruction {
    Instruction::new_with_bytes(rook_core::id(), &data.data(), accounts.to_account_metas(None))
}

impl Env {
    /// A fresh SVM with the program, the registry (owner, keeper, guardian), a converter, the treasury and the desk; no
    /// listing yet.
    pub fn bare() -> Self {
        Self::bare_on(BitcoinNetwork::Mainnet)
    }

    /// `bare()` with a registry serving `net`.
    pub fn bare_on(net: BitcoinNetwork) -> Self {
        Self::bare_with(net, vec![])
    }

    /// `bare_on(net)` with `init_registry`'s `initial_converters` (allowed at once, no `CHANGE_DELAY`).
    pub fn bare_with(net: BitcoinNetwork, converters: Vec<Pubkey>) -> Self {
        let mut svm = new_svm();
        let owner = Keypair::new();
        let keeper = Keypair::new();
        let guardian = Keypair::new();
        let custodian = Keypair::new();
        let user = Keypair::new();
        for k in [&owner, &keeper, &guardian, &custodian, &user] {
            svm.airdrop(&k.pubkey(), 100_000 * SOL).unwrap();
        }
        let mut env = Env {
            svm,
            owner,
            keeper,
            guardian,
            custodian,
            user,
            registry: pda(&[REGISTRY]),
            listing: Pubkey::default(),
            wrapper: Pubkey::default(),
            wx_mint: Pubkey::default(),
            item_set: Pubkey::default(),
            module: Pubkey::default(),
            vault: Pubkey::default(),
            module_wx: Pubkey::default(),
            treasury: pda(&[TREASURY]),
            desk: pda(&[DESK]),
            now: T0,
            listing_id: 0,
        };
        env.set_time(T0);
        let o = env.owner.pubkey();
        // the deployer: the owner is also the upgrade authority here (production: the deploy key or the Squads vault)
        env.set_upgrade_authority(Some(&o));
        let i = env.init_registry_with_ix(&o, &o, &o, net, converters);
        ok(env.send(&[i], &[&env.owner.insecure_clone()]));
        let k = env.keeper.pubkey();
        let g = env.guardian.pubkey();
        ok(env.registry_admin(rook_core::instruction::ProposeRoles { keeper: k }, None));
        ok(env.registry_admin(rook_core::instruction::SetGuardian { guardian: g }, None));
        ok(env.send(
            &[ix(
                rook_core::instruction::InitTreasury {},
                rook_core::accounts::InitTreasury {
                    owner: o,
                    payer: o,
                    registry: env.registry,
                    treasury: env.treasury,
                    system_program: system_program::ID,
                },
            )],
            &[&env.owner.insecure_clone()],
        ));
        ok(env.send(
            &[ix(
                rook_core::instruction::InitDesk { params: desk_params() },
                rook_core::accounts::InitDesk {
                    owner: o,
                    payer: o,
                    registry: env.registry,
                    desk: env.desk,
                    system_program: system_program::ID,
                },
            )],
            &[&env.owner.insecure_clone()],
        ));
        env
    }

    /// `bare()` plus one EthNft listing (Milady on chain 1) with its wrapper and module at the default parameters.
    pub fn new() -> Self {
        Self::with_params(default_params())
    }

    /// `new()` with `init_registry`'s `initial_converters` (rook_backing's `["converter"]` PDA in its real_rook_core test).
    pub fn with_converters(converters: Vec<Pubkey>) -> Self {
        Self::with_params_and(default_params(), converters)
    }

    pub fn with_params(p: Params) -> Self {
        Self::with_params_and(p, vec![])
    }

    pub fn with_params_and(p: Params, converters: Vec<Pubkey>) -> Self {
        let mut env = Self::bare_with(BitcoinNetwork::Mainnet, converters);
        let c = env.custodian.pubkey();
        ok(env.list(
            ListingKind::EthNft { chain_id: 1, collection: [0x5a; 20] },
            "Milady",
            "0x1111111111111111111111111111111111111111",
            c,
            c,
        ));
        env.init_wrapper();
        env.init_module(p);
        env
    }

    pub fn list(&mut self, kind: ListingKind, name: &str, custody: &str, custodian: Pubkey, poster: Pubkey) -> Res {
        let id = self.listing_count();
        let listing = pda(&[LISTING, &id.to_le_bytes()]);
        let o = self.owner.pubkey();
        let r = self.send(
            &[ix(
                rook_core::instruction::ListCollection {
                    args: ListArgs { kind, name: name.into(), custody: custody.into(), custodian, poster },
                },
                rook_core::accounts::ListCollection {
                    owner: o,
                    payer: o,
                    registry: self.registry,
                    listing,
                    system_program: system_program::ID,
                },
            )],
            &[&self.owner.insecure_clone()],
        );
        if r.is_ok() {
            self.use_listing(id);
        }
        r
    }

    /// Point the env at listing `id`'s PDAs.
    pub fn use_listing(&mut self, id: u32) {
        self.listing_id = id;
        self.listing = pda(&[LISTING, &id.to_le_bytes()]);
        self.wrapper = pda(&[WRAPPER, self.listing.as_ref()]);
        self.wx_mint = pda(&[WX_MINT, self.wrapper.as_ref()]);
        self.item_set = pda(&[ITEMS, self.wrapper.as_ref()]);
        self.module = pda(&[MODULE, self.listing.as_ref()]);
        self.vault = pda(&[MODULE_VAULT, self.module.as_ref()]);
        self.module_wx = pda(&[MODULE_WX, self.module.as_ref()]);
    }

    pub fn listing_count(&self) -> u32 {
        self.get::<Registry>(&self.registry).listing_count
    }

    pub fn init_wrapper(&mut self) {
        let meta = rook_core::WxMetadata { name: "Wrapped Milady".into(), symbol: "wMILADY".into(), uri: String::new() };
        ok(self.init_wrapper_with(meta));
    }

    pub fn init_wrapper_ix(&self, meta: rook_core::WxMetadata) -> Instruction {
        let o = self.owner.pubkey();
        ix(
            rook_core::instruction::InitWrapper { meta },
            rook_core::accounts::InitWrapper {
                owner: o,
                payer: o,
                registry: self.registry,
                listing: self.listing,
                wrapper: self.wrapper,
                wx_mint: self.wx_mint,
                item_set: self.item_set,
                metadata: metadata_pda(&self.wx_mint),
                token_metadata_program: rook_core::metadata::MPL_TOKEN_METADATA_ID,
                token_program: spl_token::ID,
                system_program: system_program::ID,
            },
        )
    }

    pub fn init_wrapper_with(&mut self, meta: rook_core::WxMetadata) -> Res {
        let i = self.init_wrapper_ix(meta);
        self.send(&[i], &[&self.owner.insecure_clone()])
    }

    pub fn set_wx_metadata_as(&mut self, who: &Keypair, meta: rook_core::WxMetadata) -> Res {
        let i = ix(
            rook_core::instruction::SetWxMetadata { meta },
            rook_core::accounts::SetWxMetadata {
                owner: who.pubkey(),
                registry: self.registry,
                wrapper: self.wrapper,
                metadata: metadata_pda(&self.wx_mint),
                token_metadata_program: rook_core::metadata::MPL_TOKEN_METADATA_ID,
            },
        );
        let w = who.insecure_clone();
        self.send(&[i], &[&w])
    }

    pub fn init_module_ix(&self, p: Params) -> Instruction {
        let o = self.owner.pubkey();
        ix(
            rook_core::instruction::InitModule { params: p },
            rook_core::accounts::InitModule {
                owner: o,
                payer: o,
                registry: self.registry,
                listing: self.listing,
                wrapper: self.wrapper,
                wx_mint: self.wx_mint,
                module: self.module,
                module_vault: self.vault,
                module_wx: self.module_wx,
                token_program: spl_token::ID,
                system_program: system_program::ID,
            },
        )
    }

    pub fn init_module(&mut self, p: Params) {
        let i = self.init_module_ix(p);
        ok(self.send(&[i], &[&self.owner.insecure_clone()]));
    }

    // ---- plumbing ------------------------------------------------------------------------------------------------------

    pub fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> Res {
        self.svm.expire_blockhash();
        let bh = self.svm.latest_blockhash();
        let msg = Message::new_with_blockhash(ixs, Some(&signers[0].pubkey()), &bh);
        let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), signers).unwrap();
        self.svm.send_transaction(tx).map_err(|e| (format!("{:?}", e.err), e.meta))
    }

    pub fn set_time(&mut self, t: i64) {
        self.now = t;
        let mut c: Clock = self.svm.get_sysvar();
        c.unix_timestamp = t;
        c.slot += 1;
        self.svm.set_sysvar(&c);
    }

    pub fn advance_time(&mut self, dt: i64) {
        let t = self.now + dt;
        self.set_time(t);
    }

    pub fn get<T: AccountDeserialize>(&self, k: &Pubkey) -> T {
        let a = self.svm.get_account(k).expect("account");
        T::try_deserialize(&mut a.data.as_slice()).unwrap()
    }

    pub fn exists(&self, k: &Pubkey) -> bool {
        self.svm.get_account(k).map(|a| a.lamports > 0).unwrap_or(false)
    }

    pub fn lamports(&self, k: &Pubkey) -> u64 {
        self.svm.get_account(k).map(|a| a.lamports).unwrap_or(0)
    }

    pub fn module(&self) -> Module {
        self.get(&self.module)
    }
    pub fn wrapper(&self) -> Wrapper {
        self.get(&self.wrapper)
    }
    pub fn items(&self) -> ItemSet {
        self.get(&self.item_set)
    }
    pub fn registry(&self) -> Registry {
        self.get(&self.registry)
    }

    /// Lamports the vault holds above its rent.
    pub fn vault_free(&self) -> u64 {
        let a = self.svm.get_account(&self.vault).unwrap();
        a.lamports - self.svm.minimum_balance_for_rent_exemption(a.data.len())
    }

    pub fn supply(&self) -> u64 {
        let a = self.svm.get_account(&self.wx_mint).unwrap();
        u64::from_le_bytes(a.data[36..44].try_into().unwrap())
    }

    pub fn balance(&self, token_account: &Pubkey) -> u64 {
        let a = self.svm.get_account(token_account).unwrap();
        u64::from_le_bytes(a.data[64..72].try_into().unwrap())
    }

    /// A fresh wX token account owned by `owner`.
    pub fn token_account(&mut self, owner: &Pubkey) -> Pubkey {
        let acc = Keypair::new();
        let rent = self.svm.minimum_balance_for_rent_exemption(165);
        let payer = self.owner.insecure_clone();
        let create = system_instruction::create_account(&payer.pubkey(), &acc.pubkey(), rent, 165, &spl_token::ID);
        let init = spl_token::instruction::initialize_account3(&spl_token::ID, &acc.pubkey(), &self.wx_mint, owner).unwrap();
        ok(self.send(&[create, init], &[&payer, &acc]));
        acc.pubkey()
    }

    /// The books invariant: the vault holds at least the sum of the books.
    #[track_caller]
    pub fn check_books(&self) {
        let m = self.module();
        assert!(self.vault_free() as u128 >= m.books(), "vault {} < books {}", self.vault_free(), m.books());
    }

    /// The wrapper ledger: supply = (held − open tickets)·ONE + eth_backed.
    #[track_caller]
    pub fn check_ledger(&self) {
        let w = self.wrapper();
        assert_eq!(self.supply(), (w.held - w.open_tickets) * ONE + w.eth_backed, "wX ledger");
    }

    // ---- registry ------------------------------------------------------------------------------------------------------

    /// rook_core's ProgramData account (PDA `[program_id]` of the upgradeable loader).
    pub fn program_data() -> Pubkey {
        Pubkey::find_program_address(
            &[rook_core::id().as_ref()],
            &anchor_lang::solana_program::bpf_loader_upgradeable::ID,
        )
        .0
    }

    /// LiteSVM 0.10 deploys every program under the upgradeable loader with `upgrade_authority_address: None` and has no API
    /// to name one, so the test writes the authority into the ProgramData account it created: bincode
    /// `UpgradeableLoaderState::ProgramData { slot: u64, upgrade_authority_address: Option<Pubkey> }` = tag u32 (3) @0, slot
    /// @4, option tag @12, key @13..45, then the ELF.
    pub fn set_upgrade_authority(&mut self, authority: Option<&Pubkey>) {
        let pd = Self::program_data();
        let mut acc = self.svm.get_account(&pd).expect("ProgramData");
        assert_eq!(acc.data[..4], 3u32.to_le_bytes(), "a ProgramData account");
        match authority {
            Some(a) => {
                acc.data[12] = 1;
                acc.data[13..45].copy_from_slice(a.as_ref());
            }
            None => {
                acc.data[12] = 0;
                acc.data[13..45].fill(0);
            }
        }
        self.svm.set_account(pd, acc).unwrap();
    }

    pub fn init_registry_ix(&self, authority: &Pubkey, payer: &Pubkey, owner: &Pubkey) -> Instruction {
        self.init_registry_on_ix(authority, payer, owner, BitcoinNetwork::Mainnet)
    }

    pub fn init_registry_on_ix(&self, authority: &Pubkey, payer: &Pubkey, owner: &Pubkey, net: BitcoinNetwork) -> Instruction {
        self.init_registry_with_ix(authority, payer, owner, net, vec![])
    }

    /// `init_registry` with its `initial_converters` (allowed at once, no `CHANGE_DELAY`).
    pub fn init_registry_with_ix(
        &self,
        authority: &Pubkey,
        payer: &Pubkey,
        owner: &Pubkey,
        net: BitcoinNetwork,
        initial_converters: Vec<Pubkey>,
    ) -> Instruction {
        ix(
            rook_core::instruction::InitRegistry { owner: *owner, bitcoin_network: net, initial_converters },
            rook_core::accounts::InitRegistry {
                authority: *authority,
                payer: *payer,
                registry: self.registry,
                program_data: Self::program_data(),
                system_program: system_program::ID,
            },
        )
    }

    pub fn registry_admin<D: InstructionData>(&mut self, data: D, signer: Option<&Keypair>) -> Res {
        let s = signer.map(|k| k.insecure_clone()).unwrap_or_else(|| self.owner.insecure_clone());
        let i = ix(data, rook_core::accounts::RegistryAdmin { signer: s.pubkey(), registry: self.registry });
        self.send(&[i], &[&s])
    }

    pub fn listing_admin<D: InstructionData>(&mut self, data: D, signer: &Keypair) -> Res {
        let i = ix(
            data,
            rook_core::accounts::ListingAdmin { signer: signer.pubkey(), registry: self.registry, listing: self.listing },
        );
        let s = signer.insecure_clone();
        self.send(&[i], &[&s])
    }

    // ---- floor ---------------------------------------------------------------------------------------------------------

    pub fn post_as(&mut self, poster: &Keypair, price: u64, conf: u16, depth: u32) -> Res {
        let i = ix(
            rook_core::instruction::PostFloor { price, confidence_bps: conf, depth_items: depth },
            rook_core::accounts::PostFloor {
                poster: poster.pubkey(),
                registry: self.registry,
                listing: self.listing,
                module: self.module,
            },
        );
        let s = poster.insecure_clone();
        self.send(&[i], &[&s])
    }

    /// The canister posts (confidence 9000, depth 20).
    pub fn post(&mut self, price: u64) -> Res {
        let c = self.custodian.insecure_clone();
        self.post_as(&c, price, 9_000, 20)
    }

    pub fn mint_ix(&self, buyer: &Pubkey, dest: &Pubkey, lamports: u64, min_units: u64) -> Instruction {
        ix(
            rook_core::instruction::Mint { lamports, min_units },
            rook_core::accounts::MintUnits {
                buyer: *buyer,
                registry: self.registry,
                listing: self.listing,
                module: self.module,
                module_vault: self.vault,
                wrapper: self.wrapper,
                wx_mint: self.wx_mint,
                dest: *dest,
                token_program: spl_token::ID,
                system_program: system_program::ID,
            },
        )
    }

    pub fn mint_units(&mut self, buyer: &Keypair, dest: &Pubkey, lamports: u64, min_units: u64) -> Res {
        let i = self.mint_ix(&buyer.pubkey(), dest, lamports, min_units);
        let b = buyer.insecure_clone();
        self.send(&[i], &[&b])
    }

    pub fn burn_units(&mut self, owner: &Keypair, owner_wx: &Pubkey, to: &Pubkey, units: u64, min: u64) -> Res {
        let i = ix(
            rook_core::instruction::Burn { units, min_lamports: min },
            rook_core::accounts::BurnUnits {
                owner: owner.pubkey(),
                registry: self.registry,
                listing: self.listing,
                module: self.module,
                module_vault: self.vault,
                wrapper: self.wrapper,
                wx_mint: self.wx_mint,
                owner_wx: *owner_wx,
                to: *to,
                token_program: spl_token::ID,
            },
        );
        let o = owner.insecure_clone();
        self.send(&[i], &[&o])
    }

    pub fn convert_ix(&self, converter: &Pubkey, payer: &Pubkey, dest: &Pubkey, lamports: u64, min_units: u64) -> Instruction {
        ix(
            rook_core::instruction::Convert { lamports, min_units },
            rook_core::accounts::Convert {
                converter: *converter,
                payer: *payer,
                registry: self.registry,
                listing: self.listing,
                module: self.module,
                module_vault: self.vault,
                wrapper: self.wrapper,
                wx_mint: self.wx_mint,
                dest: *dest,
                token_program: spl_token::ID,
                system_program: system_program::ID,
            },
        )
    }

    pub fn convert(&mut self, converter: &Keypair, dest: &Pubkey, lamports: u64, min_units: u64) -> Res {
        let i = self.convert_ix(&converter.pubkey(), &converter.pubkey(), dest, lamports, min_units);
        let c = converter.insecure_clone();
        self.send(&[i], &[&c])
    }

    /// Register `k` as a converter: propose, then accept once `CHANGE_DELAY` has passed. Test shortcut: the clock jumps
    /// forward for the accept and back afterwards, so suites that allow a converter mid-scenario keep their floor fresh (the
    /// delay itself is tested in registry.rs).
    pub fn allow_converter(&mut self, k: &Pubkey) {
        ok(self.registry_admin(rook_core::instruction::ProposeConverter { converter: *k }, None));
        let t = self.now;
        self.set_time(t + CHANGE_DELAY);
        ok(self.registry_admin(rook_core::instruction::AcceptConverter {}, None));
        self.set_time(t);
    }

    pub fn advance_ix(&self, custodian: &Pubkey, amount: u64) -> Instruction {
        ix(
            rook_core::instruction::Advance { amount },
            rook_core::accounts::Advance {
                custodian: *custodian,
                registry: self.registry,
                listing: self.listing,
                module: self.module,
                module_vault: self.vault,
                wrapper: self.wrapper,
            },
        )
    }

    pub fn advance(&mut self, amount: u64) -> Res {
        let c = self.custodian.insecure_clone();
        let i = self.advance_ix(&c.pubkey(), amount);
        self.send(&[i], &[&c])
    }

    fn settle_accounts(&self, custodian: &Pubkey) -> rook_core::accounts::Settle {
        rook_core::accounts::Settle {
            custodian: *custodian,
            registry: self.registry,
            listing: self.listing,
            module: self.module,
            module_vault: self.vault,
            wrapper: self.wrapper,
            wx_mint: self.wx_mint,
            module_wx: self.module_wx,
            token_program: spl_token::ID,
        }
    }

    pub fn settle(&mut self, cost: u64) -> Res {
        let c = self.custodian.insecure_clone();
        let i = ix(rook_core::instruction::SettleAdvance { cost }, self.settle_accounts(&c.pubkey()));
        self.send(&[i], &[&c])
    }

    pub fn item_arrived(&mut self, cost: u64) -> Res {
        let c = self.custodian.insecure_clone();
        let i = ix(rook_core::instruction::ItemArrived { cost }, self.settle_accounts(&c.pubkey()));
        self.send(&[i], &[&c])
    }

    pub fn return_advance_as(&mut self, who: &Keypair, lamports: u64) -> Res {
        let i = ix(
            rook_core::instruction::ReturnAdvance { lamports },
            rook_core::accounts::ReturnAdvance {
                custodian: who.pubkey(),
                listing: self.listing,
                module: self.module,
                module_vault: self.vault,
                system_program: system_program::ID,
            },
        );
        let w = who.insecure_clone();
        self.send(&[i], &[&w])
    }

    pub fn snapshot(&mut self) -> Res {
        let i = ix(
            rook_core::instruction::SnapshotCap {},
            rook_core::accounts::SnapshotCap { registry: self.registry, listing: self.listing, module: self.module },
        );
        let u = self.user.insecure_clone();
        self.send(&[i], &[&u])
    }

    pub fn fund_reserve(&mut self, lamports: u64) -> Res {
        let u = self.user.insecure_clone();
        let i = ix(
            rook_core::instruction::FundReserve { lamports },
            rook_core::accounts::FundReserve {
                payer: u.pubkey(),
                module: self.module,
                module_vault: self.vault,
                system_program: system_program::ID,
            },
        );
        self.send(&[i], &[&u])
    }

    pub fn skim(&mut self) -> Res {
        let u = self.user.insecure_clone();
        let i = ix(
            rook_core::instruction::Skim {},
            rook_core::accounts::Skim { module: self.module, module_vault: self.vault, treasury: self.treasury },
        );
        self.send(&[i], &[&u])
    }

    // ---- wrap ----------------------------------------------------------------------------------------------------------

    pub fn wrapper_admin<D: InstructionData>(&mut self, data: D, signer: &Keypair) -> Res {
        let i = ix(
            data,
            rook_core::accounts::WrapperAdmin {
                signer: signer.pubkey(),
                registry: self.registry,
                wrapper: self.wrapper,
                wx_mint: self.wx_mint,
            },
        );
        let s = signer.insecure_clone();
        self.send(&[i], &[&s])
    }

    pub fn mint_item_ix(&self, custodian: &Pubkey, args: MintItemArgs, dest: &Pubkey) -> Instruction {
        let w = self.wrapper();
        let item = pda(&[ITEM, self.wrapper.as_ref(), &w.item_count.to_le_bytes()]);
        let held = pda(&[HELD, self.wrapper.as_ref(), &args.key]);
        let source = pda(&[SOURCE, self.wrapper.as_ref(), &args.source]);
        ix(
            rook_core::instruction::MintItem { args },
            rook_core::accounts::MintItem {
                custodian: *custodian,
                listing: self.listing,
                wrapper: self.wrapper,
                wx_mint: self.wx_mint,
                item_set: self.item_set,
                item,
                held,
                source,
                dest: *dest,
                token_program: spl_token::ID,
                system_program: system_program::ID,
            },
        )
    }

    pub fn eth_args(token: u64, tx: [u8; 32]) -> MintItemArgs {
        let key = token_id(token);
        MintItemArgs {
            key,
            source: rook_core::hashing::keccak(&[&key, &tx]),
            tx,
            vout: 0,
            deposit: false,
            inscription_id: String::new(),
            proof: vec![],
        }
    }

    pub fn mint_item_as(&mut self, who: &Keypair, args: MintItemArgs, dest: &Pubkey) -> Res {
        let i = self.mint_item_ix(&who.pubkey(), args, dest);
        let w = who.insecure_clone();
        self.send(&[i], &[&w])
    }

    /// The canister wraps NFT `token` bought by `tx` into `dest`.
    pub fn wrap_nft(&mut self, token: u64, tx: [u8; 32], dest: &Pubkey) -> Res {
        let c = self.custodian.insecure_clone();
        self.mint_item_as(&c, Self::eth_args(token, tx), dest)
    }

    pub fn ticket_pda(&self, id: u64) -> Pubkey {
        pda(&[TICKET, self.wrapper.as_ref(), &id.to_le_bytes()])
    }

    pub fn redeem_ix(&self, owner: &Pubkey, owner_wx: &Pubkey, destination: Destination) -> Instruction {
        let id = self.wrapper().ticket_count;
        ix(
            rook_core::instruction::Redeem { destination },
            rook_core::accounts::Redeem {
                owner: *owner,
                listing: self.listing,
                wrapper: self.wrapper,
                wx_mint: self.wx_mint,
                owner_wx: *owner_wx,
                ticket: self.ticket_pda(id),
                token_program: spl_token::ID,
                system_program: system_program::ID,
            },
        )
    }

    pub fn redeem(&mut self, owner: &Keypair, owner_wx: &Pubkey, destination: Destination) -> Res {
        let i = self.redeem_ix(&owner.pubkey(), owner_wx, destination);
        let o = owner.insecure_clone();
        self.send(&[i], &[&o])
    }

    pub fn ticket(&self, id: u64) -> Ticket {
        self.get(&self.ticket_pda(id))
    }

    pub fn prove(&mut self, round: u64, randomness: [u8; 32]) -> Res {
        let k = self.keeper.insecure_clone();
        let i = ix(
            rook_core::instruction::ProveRound { round, randomness },
            rook_core::accounts::ProveRound {
                keeper: k.pubkey(),
                payer: k.pubkey(),
                registry: self.registry,
                drand_round: pda(&[DRAND, &round.to_le_bytes()]),
                system_program: system_program::ID,
            },
        );
        self.send(&[i], &[&k])
    }

    pub fn resolve_ix(&self, id: u64, with_round: bool) -> Instruction {
        let round = if with_round {
            let t: Ticket = self.ticket(id);
            Some(pda(&[DRAND, &t.round.to_le_bytes()]))
        } else {
            None
        };
        ix(
            rook_core::instruction::Resolve {},
            rook_core::accounts::Resolve {
                wrapper: self.wrapper,
                item_set: self.item_set,
                ticket: self.ticket_pda(id),
                drand_round: round,
            },
        )
    }

    /// Moves the clock to the start of drand round `round` if it is not live yet (`prove_round` refuses future rounds).
    pub fn reach_round(&mut self, round: u64) {
        if rook_core::clock::round_at(self.now) < round {
            let t = rook_core::clock::DRAND_GENESIS + (round as i64 - 1) * rook_core::clock::DRAND_PERIOD;
            self.set_time(t);
        }
    }

    /// Prove the ticket's round (if not yet; moving the clock to it if needed) and resolve it.
    pub fn resolve(&mut self, id: u64) -> Res {
        let t = self.ticket(id);
        let rp = pda(&[DRAND, &t.round.to_le_bytes()]);
        if !self.exists(&rp) {
            self.reach_round(t.round);
            let mut r = [7u8; 32];
            r[..8].copy_from_slice(&t.round.to_le_bytes());
            ok(self.prove(t.round, r));
        }
        let i = self.resolve_ix(id, true);
        let u = self.user.insecure_clone();
        self.send(&[i], &[&u])
    }

    pub fn custodian_ticket<D: InstructionData>(&mut self, data: D, id: u64, who: &Keypair) -> Res {
        let i = ix(
            data,
            rook_core::accounts::CustodianTicket {
                custodian: who.pubkey(),
                listing: self.listing,
                wrapper: self.wrapper,
                ticket: self.ticket_pda(id),
            },
        );
        let w = who.insecure_clone();
        self.send(&[i], &[&w])
    }

    pub fn record_delivery_as(&mut self, who: &Keypair, id: u64, tx: [u8; 32]) -> Res {
        let t = self.ticket(id);
        let item_pk = pda(&[ITEM, self.wrapper.as_ref(), &t.item_id.to_le_bytes()]);
        let item: Item = self.get(&item_pk);
        let i = ix(
            rook_core::instruction::RecordDelivery { tx },
            rook_core::accounts::RecordDelivery {
                custodian: who.pubkey(),
                listing: self.listing,
                wrapper: self.wrapper,
                ticket: self.ticket_pda(id),
                item: item_pk,
                held: pda(&[HELD, self.wrapper.as_ref(), &item.key]),
            },
        );
        let w = who.insecure_clone();
        self.send(&[i], &[&w])
    }

    pub fn record_delivery(&mut self, id: u64, tx: [u8; 32]) -> Res {
        let c = self.custodian.insecure_clone();
        self.record_delivery_as(&c, id, tx)
    }

    pub fn refund(&mut self, id: u64, owner_wx: &Pubkey) -> Res {
        let i = ix(
            rook_core::instruction::Refund {},
            rook_core::accounts::Refund {
                listing: self.listing,
                wrapper: self.wrapper,
                item_set: self.item_set,
                ticket: self.ticket_pda(id),
                wx_mint: self.wx_mint,
                owner_wx: *owner_wx,
                drand_round: pda(&[DRAND, &self.ticket(id).round.to_le_bytes()]),
                token_program: spl_token::ID,
            },
        );
        let u = self.user.insecure_clone();
        self.send(&[i], &[&u])
    }

    // ---- composite helpers ---------------------------------------------------------------------------------------------

    /// A fresh floor and `n` whole units minted to a new account of `user`; returns that account.
    pub fn user_with_units(&mut self, lamports: u64) -> Pubkey {
        let u = self.user.insecure_clone();
        let acc = self.token_account(&u.pubkey());
        ok(self.mint_units(&u, &acc, lamports, 0));
        acc
    }
}
