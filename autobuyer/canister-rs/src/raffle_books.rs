//! The holder raffle's snapshot on the BOOKS (canister-only, docs/v9/canister-only-solana.md §9.3; docs/v9/raffle-design.md §4–§5):
//! the same rules as [`crate::raffle`], the coins and their backing read from where the canister-only home keeps them instead of
//! rook_backing's vaults (the registry keeper's version, retired with the programs: decision D-l).
//!
//! What changes against the on-chain version (and nothing else):
//! - the coins of a collection are the `Backed` records of [`crate::coins::state`] (their mint, and their vault: `K_coin[c]`'s wX
//!   token account), `wx(c)` = that account's live balance;
//! - the exclusions add EVERY canister-derived address (`K_main`, `K_relay`, `K_fc`, `K_tre`, every `K_coll[i]` and `K_desk[i]` of
//!   the listed collections, every `K_coin[c]` of the known coins, every live `K_int[n]`), since they are on the curve and would
//!   otherwise count as holders: their balances are listed in the snapshot's `excluded` (in `supply`, out of `circ`), like a pool
//!   vault's ([`crate::raffle::build_with`]);
//! - `listing` is the collection's wX mint and `raffle` the books' raffle key (`books::Raffle::key`), the 32 bytes the books'
//!   `raffle_ticket` is keyed by, so [`winner`] and `Books::claim_raffle` agree.
//!
//! The crank publishes what comes out of [`take`] (`raffle::publish`) and commits it to the books; after the draw, [`winner`] gives
//! the leaf and the proof `claim_raffle` takes. Nothing here writes any state.
use crate::coins::state::{self as coins, CoinStage};
use crate::desk::intent::State as IntentState;
use crate::desk::state as desk;
use crate::raffle::{self, CoinRead, Leaf, SnapError, Snapshot};
use crate::solhome::SolIo;
use crate::solkey::KeyPath;
use crate::solrpc::{Account, MAX_KEYS, SLOT_BUCKET};
use crate::soltx::{self, b58_encode, Pubkey};

/// A token account's balance (SPL Token: mint 32, owner 32, amount u64).
pub const TOKEN_AMOUNT_OFFSET: usize = 64;
/// A mint's supply (SPL Token: an optional authority, `COption<Pubkey>` = 36 bytes, then the supply).
pub const MINT_SUPPLY_OFFSET: usize = 36;
/// Token accounts of one coin listed by one read (owner and amount, ≈ 236 bytes an entry: ≈ 1.9 MB at most, under the IC's 2 MB);
/// past it the listing is split by the owner's first byte (256 reads).
pub const HOLDERS_PER_READ: usize = 8_000;
/// The first token-account listing of a coin is asked at this many entries (≈ 240 kB of response cap), and only past it at
/// [`HOLDERS_PER_READ`] (≈ 1.9 MB): a response cap is paid for whether used or not.
pub const HOLDERS_FIRST: usize = 1_024;
/// The snapshot's reads must all fall within this many slots (≈ 20 min), or it is taken again.
pub const SNAPSHOT_MAX_SPAN: u64 = 3_000;

fn hard(why: String) -> SnapError {
    SnapError { soft: false, why }
}

/// Every SPL Token account of `mint`: `(account, owner, amount)`, sorted by account, from agreed `getProgramAccounts` reads of the owner and
/// amount only. One read when it fits `per_read` entries; past it, one read per first byte of the owner (256), so a coin with any number
/// of holders is listed whole (or refused, never truncated).
pub async fn token_holders<I: SolIo>(io: &mut I, mint: &Pubkey, per_read: usize) -> Result<Vec<(Pubkey, Pubkey, u64)>, String> {
    let tp = soltx::key(soltx::TOKEN_PROGRAM);
    let parse = |v: Vec<(Pubkey, Pubkey, Vec<u8>)>| -> Result<Vec<(Pubkey, Pubkey, u64)>, String> {
        v.into_iter().map(|(k, o, d)| {
            if o != tp || d.len() != 40 {
                return Err(format!("token account {} of coin {}: not an SPL Token account", b58_encode(&k), b58_encode(mint)));
            }
            Ok((k, d[..32].try_into().expect("32"), u64::from_le_bytes(d[32..40].try_into().expect("8"))))
        }).collect()
    };
    let mut levels = vec![HOLDERS_FIRST.min(per_read), per_read];
    levels.dedup();
    for n in levels {
        match io.sol_program_slices(&tp, 165, &[(0, mint.to_vec())], (32, 40), n).await {
            Ok(v) => return parse(v),
            Err(e) if e.contains("disagree") => return Err(e),
            Err(_) => {}
        }
    }
    // past one read: by the owner's first byte
    let mut out = Vec::new();
    for b in 0..=255u8 {
        out.extend(parse(io.sol_program_slices(&tp, 165, &[(0, mint.to_vec()), (32, vec![b])], (32, 40), per_read).await?)?);
    }
    out.sort_unstable_by_key(|x| x.0);
    Ok(out)
}

/// A token account's (or a mint's) u64 at a slice read, `None` unless the token program owns it.
fn token_u64(a: &Option<Account>) -> Option<u64> {
    a.as_ref().filter(|a| a.owner == soltx::key(soltx::TOKEN_PROGRAM) && a.data.len() == 8).map(|a| u64::from_le_bytes(a.data[..8].try_into().expect("8")))
}

async fn slices_paged<I: SolIo>(io: &mut I, keys: &[Pubkey], offset: usize, len: usize) -> Result<Vec<Option<Account>>, String> {
    let mut out = Vec::with_capacity(keys.len());
    for c in keys.chunks(MAX_KEYS) {
        out.extend(io.sol_accounts_slice(c, offset, len).await?);
    }
    Ok(out)
}

/// A `Backed` coin of the collection as the books know it: its mint and its vault (`K_coin[c]`'s wX token account).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookedCoin {
    pub mint: Pubkey,
    pub vault_wx: Pubkey,
}

/// Collection `coll`'s backed coins, by mint (pure over the coins state: a coin counts once it is `Backed` and has its vault).
pub fn backed_coins(coll: u16) -> Vec<BookedCoin> {
    coins::with(|s| {
        s.coins.values()
            .filter(|c| c.coll == coll as u32 && c.stage == CoinStage::Backed)
            .filter_map(|c| c.vault.as_ref().map(|v| BookedCoin { mint: c.coin, vault_wx: v.address }))
            .collect()
    })
}

/// The key paths whose addresses are the canister's own (docs/v9/canister-only-solana.md §9.3): every one of them is on the curve, so
/// the raffle must exclude them by name. `K_main`, `K_relay`, `K_fc`, `K_tre`, the listed collections' `K_coll[i]` and `K_desk[i]`,
/// every known coin's `K_coin[c]`, and every intent `K_int[n]` that may still hold a deposit (not refunded or expired).
pub fn canister_paths() -> Vec<KeyPath> {
    let mut paths = vec![KeyPath::Main, KeyPath::Relay, KeyPath::Claimer, KeyPath::Treasury];
    desk::with(|s| {
        for id in s.books.collections.keys() {
            paths.push(KeyPath::Coll(*id as u32));
            paths.push(KeyPath::Desk(*id as u32));
        }
        for (n, i) in &s.intents {
            if matches!(i.state, IntentState::Open | IntentState::Funded | IntentState::Executing | IntentState::Done) {
                paths.push(KeyPath::Intent(*n));
            }
        }
    });
    coins::with(|s| {
        for mint in s.coins.keys() {
            paths.push(KeyPath::Coin(*mint));
        }
    });
    paths
}

/// The design's name of a canister key (`K_desk[0]`), the reason an excluded account shows.
pub fn key_name(p: &KeyPath) -> String {
    match p {
        KeyPath::Main => "K_main".into(),
        KeyPath::Relay => "K_relay".into(),
        KeyPath::Coll(i) => format!("K_coll[{i}]"),
        KeyPath::Claimer => "K_fc".into(),
        KeyPath::Coin(m) => format!("K_coin[{}]", b58_encode(m)),
        KeyPath::Desk(i) => format!("K_desk[{i}]"),
        KeyPath::Treasury => "K_tre".into(),
        KeyPath::Intent(n) => format!("K_int[{n}]"),
        KeyPath::PositionNft(n) => format!("K_nft[{n}]"),
        KeyPath::Config(i) => format!("K_cfg[{i}]"),
    }
}

/// Every canister-derived address with its reason (`canister key K_desk[0]`), derived through `io` (the threshold key on the IC, the
/// fake signer's in tests).
pub async fn canister_exclusions<I: SolIo>(io: &mut I) -> Result<Vec<(Pubkey, String)>, String> {
    let mut out = Vec::new();
    for p in canister_paths() {
        let k = io.sol_key_of(&p).await?;
        out.push((k, format!("canister key {}", key_name(&p))));
    }
    Ok(out)
}

/// The holder snapshot of collection `coll` on the books: every `Backed` coin's live wX backing, supply and every token account, read
/// between two finalized clocks; then [`raffle::build_with`] with the canister's own addresses excluded. `round` is the round it
/// will be committed as (the books' `Raffle.round + 1`).
pub async fn take<I: SolIo>(io: &mut I, coll: u16, round: u64) -> Result<Snapshot, SnapError> {
    let (listing, raffle_key) = desk::with(|s| {
        let c = s.books.get(coll).map_err(|e| hard(format!("collection {coll}: {e:?}")))?;
        let r = c.raffle.as_ref().ok_or_else(|| hard(format!("collection {coll}: no raffle on the books")))?;
        Ok::<_, SnapError>((c.wx_mint, r.key))
    })?;
    let mut booked = backed_coins(coll);
    booked.sort_unstable_by_key(|c| c.mint);
    booked.dedup();
    if booked.is_empty() {
        return Err(SnapError { soft: true, why: "no backed coin yet".into() });
    }
    let excluded = canister_exclusions(io).await.map_err(hard)?;
    let (_, slot_lo) = io.sol_clock().await.map_err(hard)?;
    let mints: Vec<Pubkey> = booked.iter().map(|c| c.mint).collect();
    let vaults: Vec<Pubkey> = booked.iter().map(|c| c.vault_wx).collect();
    let wx = slices_paged(io, &vaults, TOKEN_AMOUNT_OFFSET, 8).await.map_err(hard)?;
    let supply = slices_paged(io, &mints, MINT_SUPPLY_OFFSET, 8).await.map_err(hard)?;
    let mut reads = Vec::new();
    for (i, c) in booked.iter().enumerate() {
        let w = token_u64(&wx[i]).unwrap_or(0);
        if w == 0 {
            continue;
        }
        let s = token_u64(&supply[i])
            .ok_or_else(|| hard(format!("coin {}: not an SPL Token mint (a Token-2022 coin is not snapshotted)", b58_encode(&c.mint))))?;
        let accounts = token_holders(io, &c.mint, HOLDERS_PER_READ).await.map_err(hard)?;
        // audit B3-07: a holder's own burn between the supply read and the holders' (nothing mints: the authority is revoked) leaves the
        // holders' sum under the supply; the supply as it stands after the holders is taken when it is their sum (an incomplete listing
        // is still the hard refusal `build_with` makes)
        let all: u128 = accounts.iter().map(|a| a.2 as u128).sum();
        let s = if all != s as u128 {
            let again = slices_paged(io, &[c.mint], MINT_SUPPLY_OFFSET, 8).await.map_err(hard)?;
            match token_u64(&again[0]) {
                Some(s2) if s2 as u128 == all => s2,
                _ => s,
            }
        } else {
            s
        };
        reads.push(CoinRead { mint: c.mint, vault_wx: c.vault_wx, wx: w, supply: s, accounts });
    }
    let (time, slot_hi) = io.sol_clock().await.map_err(hard)?;
    if slot_hi.saturating_sub(slot_lo) > SNAPSHOT_MAX_SPAN {
        return Err(hard(format!("the reads spanned slots {slot_lo}..{slot_hi}, over {SNAPSHOT_MAX_SPAN}: taken again")));
    }
    // the clock's slot is rounded down to a bucket: the balances were read at or below its end
    raffle::build_with(listing, raffle_key, round, slot_lo, slot_hi + SLOT_BUCKET - 1, time, &reads, &excluded)
}

/// After the draw: the leaf whose range holds the books' ticket for `randomness` (the verified drand randomness of the pinned round),
/// keyed by the books' raffle `key` and the committed `round` (`books::raffle::raffle_ticket`), and its proof: what
/// `Books::claim_raffle(id, owner, cum_start, weight, proof, now)` takes. `None` when the snapshot is empty (never after a commit).
pub fn winner(s: &Snapshot, randomness: &[u8; 32], key: &[u8; 32], round: u64) -> Option<(Leaf, Vec<[u8; 32]>)> {
    let ticket = crate::books::raffle::raffle_ticket(randomness, key, round, s.total_weight);
    let (_, leaf, proof) = s.winner(ticket)?;
    Some((leaf.clone(), proof))
}
