//! etched v2: the autonomous buyer.
//!
//! A program that holds a Bitcoin key and an EVM key that no person can extract, takes a vault's released ETH, obtains BTC,
//! buys the asset a draw's winner is owed, delivers it straight to them, and posts the receipt back on chain.
//!
//! The security argument lives in two pure modules that never touch the network and are the most heavily tested code here:
//! [`runestone`] decodes what a Bitcoin transaction actually moves, and [`psbt`] refuses to sign anything that does not do
//! what we asked for. Everything else is plumbing around them. See `docs/v2/state-machine.md`.

pub mod runestone;
pub mod psbt;
pub mod sighash;
pub mod state;
pub mod venue;
pub mod bridge;
pub mod evmsend;
pub mod pin;
// reading chain 4663 through providers with real limits, and the wrapper's events at finality (audit D, H-1)
pub mod evmread;
pub mod ticketlog;
pub mod evmtx;
pub mod btcsend;
pub mod btcnet;
pub mod delivery;
pub mod custody;
pub mod redeem;
pub mod custodynet;
// hosted ordinals APIs, two at a time (docs/v6/ordinals-apis.md)
pub mod ordapi;
pub mod ordnet;
pub mod looper;
pub mod listing;
pub mod rates;
pub mod runeindex;
pub mod oracle;
pub mod abi;
pub mod bridgecall;
pub mod policy;
pub mod relay;
pub mod relayquote;
// Relay conversion: a graduation's own ETH pays for its frog (docs/v6/relay-conversion.md)
pub mod relayconv;
pub mod convflow;
pub mod seaport;
pub mod chains;
pub mod nftnet;
// graduation buys through item sinks, the mint desk and holder deposits (docs/v3/curve.md, "Canister: …")
pub mod frogbuy;
pub mod itemsink;
pub mod curvebuy;
pub mod buyqueue;
pub mod mintdesk;
pub mod deposits;
pub mod frogflow;
pub mod frognet;
// the Floor Module as an item sink, per-collection purchase limits, NFT wants (docs/v5/floor-feed.md §6)
pub mod floorsink;
pub mod floortrigger;
pub mod floornet;
// custody hardening (docs/v5/custody-hardening.md, H2–H4): caps, outbound rate limits, the outbound verifier
pub mod custodycap;
pub mod outflow;
pub mod outbound;
// H8: leaked signed bytes killed by spending their fee input back to custody
pub mod conflict;
// permissionless offer hand-ins with a bond and a reward (docs/v6/offer-handins.md)
pub mod handin;
pub mod handinflow;
pub mod handinnet;
// protocol sales from a module's inventory, and YOYO relist tickets (docs/v6/canister-floor-followups.md)
pub mod sale;
pub mod saleflow;
pub mod salenet;

// Phase 1 on Ethereum NFTs: Relay to Ethereum, Seaport buys, custody at our Ethereum address, the floor (docs/v7/phase1-nft.md)
pub mod relayeth;
pub mod ethnft;
pub mod ethflow;
pub mod ethnet;
// dormant, wake-on-demand collections: cycles scale with ACTIVE collections, not listed ones (docs/v7/dormant-collections.md)
pub mod dormancy;
// Ordinals Wallet protected listings (passthrough v4): taproot script trees and the judge (docs/v8/ow-protected.md §3.1)
pub mod taptree;
pub mod owbuy;

// the Solana home (docs/v9/solana-migration.md §2.4, §4): the foundation (transactions, agreed reads, the threshold key, the send
// lines). The rook_core / rook_backing readers, builders and keeper (`solread`, `solix`, `solcrank`) are retired: decision D-l
// (docs/v9/canister-only-solana.md §0), Rook deploys no Solana programs and the canister keeps the books
pub mod soltx;
pub mod solrpc;
pub mod solkey;
pub mod solsend;
pub mod relaysol;
// phase B1: the Ethereum NFT side's home on Solana (its lines, its money rules in two currencies, the IC's world)
pub mod solhome;
// phase B2: the frog side's home on Solana (the books' Inscriptions listing, Relay SOL to BTC, the floor post)
pub mod frogsol;
// the drand rounds the books' draws and raffles are resolved with, proven by the canister itself
pub mod drand;
// the crank line's rules (a step never queued, signed on a free line) and the crank passes' world
pub mod crankline;
pub mod raffle;
// the raffle's snapshot on the books (docs/v9/canister-only-solana.md §9.3): the coins from `coins::state`, the canister's own keys excluded
pub mod raffle_books;
// W14b: the canister's own timer drives the four Solana send lines (docs/v9/canister-only-solana.md §13.1)
pub mod soltimer;
// W2 (docs/v9/canister-only-solana.md §2, §3.3, §5): SPL Token, ATA, Memo, Metaplex, Meteora DBC and DAMM v2 builders and decoders
pub mod solix2;
// W1 (docs/v9/canister-only-solana.md §4): the floor module, wrapper ledger, treasury and raffle books (rook_core's rules), in the canister
pub mod books;
// Solana NFT collections held by the canister (docs/v9/solana-nfts-canister.md): membership, custody, buys, delivery, the floor
pub mod solnft;
pub mod solnftbuy;
pub mod solnftsend;
pub mod solnftfloor;
// W4/W5 (docs/v9/canister-only-solana.md §4.4, §6): the links between the books and the chain, and the intent desk
pub mod chainlink;
pub mod desk;
// W6 (docs/v9/canister-only-solana.md §2, §3, §7, §9.1): the coins (open, graduation, fees, redemption)
pub mod coins;
// W7 (docs/v9/canister-only-solana.md §5, §17): the wX/SOL pool desk
pub mod pooldesk;
// W9 (docs/v9/canister-only-solana.md §9): the treasury, the draws and the raffles as the canister's cranks on the books
pub mod cranks;
// spike S5 (docs/v9/spikes/s5-magic-eden-m2.md): Magic Eden M2 purchases without the notary
pub mod solnftm2;
// W8c (docs/v9/solana-nfts-canister.md §1, §9): the Solana NFT flow on the books home: the floor from Magic Eden's listings verified on
// chain, the advance + purchase in one judged transaction, custody and delivery on the books
pub mod solnftflow;

pub mod canister;
