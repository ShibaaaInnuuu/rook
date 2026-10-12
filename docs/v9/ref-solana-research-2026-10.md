# Reference: the Solana stack as researched on 2026-10-09

Web research for the Solana migration (`solana-migration.md`). Facts and prices dated 2026-10-09; sources inline. Secondary reporting is marked as such in the text.


No code was changed. I checked some sources directly: the GitHub repos for Anchor, Meteora DBC and DFINITY sol-rpc-canister (cloned into /tmp), and the live Relay API (`/chains` plus real quotes). The rest comes from web sources, and secondary reporting is marked as such.

**Five findings change the plan:**
1. **Switchboard has shut down.** It was announced on 2026-09-19 and services ended on 2026-09-25. Rook cannot use it for oracles or for randomness.
2. **Durable nonces are slated for removal on Solana.** No date is set yet. The DFINITY docs rely on them for canister signing.
3. **Meteora DBC can pay part of the raise to a partner at graduation.** A migration fee of up to 99% of the threshold goes to the partner. That fits the FloorModule.
4. **Solana is no longer clearly the top memecoin venue.** In early September, Pons on Robinhood Chain beat pump.fun on volume and fees.
5. **Relay's to-BTC quotes failed in my tests.** SOL→BTC and ETH→BTC both returned `INSUFFICIENT_FUNDS` "No UTXOs available to spend". Quotes into Solana and SOL→ETH worked.

---

## 1. Writing Solana programs

**Anchor**
- **Version:** v1.x is the stable line. The latest tag is **v1.2.1 (2026-10-06)**; v1.2.0 came out 2026-09-04, and the 1.0.x line was out by 2026-05-02. Back-ports v0.30.2, v0.31.2 and v0.32.2 shipped 2026-09-14. https://github.com/solana-foundation/anchor/releases
- **Maintainer:** OtterSec took over Anchor from the Solana Foundation in about April 2026 (Colosseum newsletter). Release tags are signed by an OtterSec engineer. https://blog.colosseum.com/frontier-directory-ottersec-anchor-realms/
- **Anchor v2:** in alpha, on the `anchor-next` branch. It is built on Pinocchio and `no_std`, and claims binaries up to 95% smaller and 3–50× fewer compute units (CU). https://v2.anchor-lang.com/docs/v1/
- **Ecosystem check:** Meteora DBC itself builds on Anchor 1.0.2 and Solana 3.1.10.

**Alternatives**
- **Pinocchio (Anza):** zero-dependency, `no_std`. It is still on 0.x; I found no 1.0, and 0.11 renamed core types (AccountView/Address). p-token, the SPL Token rewrite on Pinocchio, cut a transfer from 4,645 to 76 CU and went live in spring 2026 (Anza blog, 2026-05-14). https://anza.xyz/blog/febo-on-pinocchio-p-token-and-pushing-solanas-limits , https://docs.chainstack.com/docs/solana-pinocchio-vs-quasar
- **Native solana-program:** possible, but more boilerplate and more audit surface.
- **Recommendation:** Anchor 1.x for the curve, FloorModule and wX programs. Optimise hot paths with Pinocchio later if CU becomes a problem.

**Token-2022 extensions that matter for Rook**
- **Metadata pointer and token metadata:** puts metadata on the mint itself, no Metaplex needed for fungibles.
- **Transfer hooks:** useful for enforcing rules on wX. But hooks add accounts to every transfer and have limited aggregator support (see §9).
- **Permanent delegate:** Jupiter's UI flags it as able to "drain wallets", and community compatibility tables mark it unsupported. Keep it off any token you want traded. https://hub.jup.ag/guides/swap/tutorials/trade-safely
- **Transfer fee:** DBC refuses quote mints with a non-zero transfer fee (DBC CHANGELOG 0.2.1).

**Metaplex Core (for redemption receipts or per-NFT custody records)**
- One account per asset, about 0.0029–0.0037 SOL per mint, versus about 0.022 SOL with Token Metadata.
- Plugins include Freeze Delegate, Permanent Freeze Delegate (only settable at creation), an Oracle plugin that can veto transfer, burn or update, and AppData.
- A good fit for a non-fungible "redemption ticket" or a custody certificate per real NFT.
- https://www.metaplex.com/docs/smart-contracts/core , https://developers.metaplex.com/zh/smart-contracts/core/external-plugins/oracle

**Upgrade authority and timelock**
- Use the upgradeable BPF loader and hand the upgrade authority to a **Squads v4** vault PDA. Program `SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf`, audited by OtterSec, Neodyme, Certora and Trail of Bits.
- **Timelock:** the v4 multisig account has `time_lock: u32`, the seconds between approval and execution. Set it with a `SetTimelock` config action. A 72 h lock is `259200`.
- Keep `config_authority = Pubkey::default()` (an "autonomous" multisig). Otherwise a single key can change the timelock.
- **Guardian pause:** a second Squads multisig with no timelock (or a dedicated guardian key) that can only call a `pause` instruction in your program. This mirrors the EVM setup.
- A third-party guide warns that one wrong character in the vault PDA permanently loses the upgrade authority. Test an upgrade first.
- https://docs.squads.so/main/development/reference/accounts , https://hyperdrive.squads.so/reference/time-locks , https://github.com/Squads-Protocol/v4

**Verifiable builds**
- Tool: `solana-verify` from https://github.com/solana-foundation/solana-verifiable-build , maintained by Ellipsis Labs and OtterSec.
- Flow: run `verify-from-repo` to write the on-chain verification PDA, then `solana-verify remote submit-job --program-id … --uploader …`.
- OtterSec's API rebuilds the program and publishes to verify.osec.io and the explorers. For multisig-owned programs, use `export-pda-tx` and run it through Squads.
- The old `--remote` flag is deprecated.
- https://solana.com/docs/programs/verified-builds

**Audit firms active on Solana:** OtterSec, Neodyme, Zellic, Sec3, Offside Labs, Ackee, Halborn, Accretion, Certora (formal verification) and Trail of Bits. Several of them audited Squads v4.

## 2. Launchpad: build our own or integrate

**Meteora Dynamic Bonding Curve (DBC)**
- Program `dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN`, version 0.2.1, repo HEAD 2026-09-09.
- **Model:** a partner (Rook) creates a config key, and creators launch pools against it.
- **Custom curve:** up to 16 curve points through the public builders, 20 on-chain. You also set the start price, the quote mint (SOL, USDC and others), SPL or Token-2022, decimals 6–9, and the supply.
- **Fees:**
  - Pre-graduation fee from 0.25% to 99%, with a time-based fee scheduler (anti-sniper) and an optional dynamic fee. The rate-limiter mode was deprecated in 0.2.1.
  - The protocol takes 20% of trading fees, and the partner gets the rest minus `creator_trading_fee_percentage`.
  - The protocol takes 10% of an optional pool-creation fee (0.001–100 SOL).
- **Graduation:**
  - Graduation triggers when the quote reserve reaches `migration_quote_threshold`. The pool moves to **DAMM v2**; DAMM v1 is deprecated for new configs.
  - The LP split between partner and creator is configurable, locked or claimable. At least 10% must stay locked for one day, and vesting is available.
  - The migrated pool's fee tier is configurable: 0.25–6% or custom.
- **Graduation payout:** `MigrationFee { fee_percentage ≤ 99, creator_fee_percentage ≤ 100 }`. At migration, `fee = threshold × fee_percentage / 100` is taken out of the raise, and the partner share is withdrawn with `withdraw_migration_fee` (source: `programs/.../state/config.rs::get_migration_quote_amount`, constant `MAX_MIGRATION_FEE_PERCENTAGE = 99`).
  - **This is how a set share of every raise reaches the FloorModule reserve.** If `fee_claimer` is a PDA of our program, our program can sign the withdrawal and credit the reserve.
- **What DBC cannot do:** run arbitrary graduation logic. There are no custom hooks; all you get is fee and LP routing.
- **Transfer hooks during the curve:** a Token-2022 base token can have a transfer hook while on the curve. The hook authority is revoked when the curve completes, so it cannot enforce rules after graduation.
- **Distribution and partners:** Jupiter integrates DBC directly. Partners include Bags, Star.fun and America.fun.
- Sources: https://github.com/MeteoraAg/dynamic-bonding-curve , https://docs.meteora.ag/developer-guides/dbc/program/accounts , https://docs.meteora.ag/developer-guide/guides/dbc , https://www.kucoin.com/blog/ph-met-2026-q1-project-update

**Raydium LaunchLab "Platforms"**
- A platform is a PDA with its own fee in basis points (subject to program caps), a creator-fee setting, and the CPMM `AmmConfig` used after graduation.
- There are four additive fees: protocol (example 0.25%), platform (example 1%), creator (capped at 5% per the Japanese docs) and per-trade referral.
- **Curves:** you choose from the supported curves; you cannot add your own.
- **Graduation:** all new launches graduate to **CPMM**. After the **2026-08-17** upgrade, platform plus creator LP becomes one platform-owned locked-LP position, and the rest is burned.
- **No share of the raise is documented**, only trading fees and LP-fee rights.
- https://docs.raydium.io/products/launchlab/platforms , https://docs.raydium.io/products/launchlab/tips-and-gotchas/launchlab-cpmm-fee-reference

**pump.fun**
- No partner or white-label config. Graduation is to PumpSwap.
- Creators can route creator fees to up to 10 wallets (Jan 2026), so a Rook PDA could be one recipient, but only voluntarily, per coin.
- Fee policy keeps changing: cashback in Feb 2026, then Holder Rewards.
- **Cannot enforce Rook's invariant.**
- https://cryptobriefing.com/pumpfun-holder-rewards-cashback-deprecated/

**Believe, Bags and others** are partners on top of Meteora tooling (Bags is confirmed). They are competitors, not integration targets.

**Which preserves the NFT-backed floor?**
- **Our own curve program** gives full control: graduation can atomically send X% of the raise to the FloorModule and mint or lock wX. It costs a full build plus audit, and Jupiter routing has to be earned.
- **DBC** gives an enforceable partial version through the migration fee (up to 99%) plus partner trading fees. It brings Jupiter routing and an audited curve on day one. The FloorModule then sweeps fees and the migration fee into the reserve. **This is the strongest off-the-shelf option.**
- **LaunchLab:** fees only.
- **pump.fun:** nothing enforceable.

## 3. Internet Computer and Solana

- **Threshold Ed25519 signing is live on mainnet.** Use `sign_with_schnorr` with algorithm `ed25519`.
  - Keys: `key_1` (production) and `test_key_1`. There is no local Ed25519 key, so testing happens on mainnet.
  - Cost is about **26.15B cycles per signature**, roughly $0.035 at about 1 XDR per trillion cycles.
  - The 32-byte public key, base58-encoded, is the Solana address. The canister signs the whole message, not a hash.
  - https://docs.internetcomputer.org/guides/chain-fusion/solana/
- **SOL RPC canister** (DFINITY, controlled by the NNS, crate version 1.3.2, repo updated 2026-10-08):
  - Providers: Alchemy, Ankr, Chainstack, dRPC, Helius and PublicNode. No API keys are needed.
  - It queries **3 providers by default**. Consensus can be `Equality` or a `Threshold` such as 3-of-5.
  - Cost is about 1–5B cycles per small request. Set `max_response_bytes` tight.
  - **Canister ID needs checking:** the repo README says `tghme-zyaaa-aaaar-qarca-cai` (fiduciary subnet), while the docs and protocol-canister reference say `2xib7-jqaaa-aaaar-qai6q-cai`.
  - Status: launched March 2026, completing the "Helium" milestone. Docs still call the API "evolving", with no SPL helpers and no official ICP library for building Solana transactions.
  - https://github.com/dfinity/sol-rpc-canister , https://thedefiant.io/internet-computer-woos-blockchain-devs-with-solana-access-amid-dwindling-onchain-activity
- **ckSOL** exists: minter `lh22c-kyaaa-aaaar-qb5nq-cai`, repo https://github.com/dfinity/cksol . The repo says it is under active development. Rook does not need it.
- **Examples:** `examples/basic_solana` in sol-rpc-canister covers SOL transfers, SPL transfers and durable-nonce sends.
- **Blockhash timing problem:**
  - Blockhashes are valid for about 150 slots (about 60 s at current slot times).
  - `getLatestBlockhash` cannot reach consensus through HTTPS outcalls because the value changes every 400 ms.
  - The README's two options: (a) a **durable nonce**, or (b) call `getSlot` (rounded to 20 slots) and then `getBlock` for its blockhash.
  - **Solana plans to remove durable nonces.** The replacement is an offline-signer program (Anza "Programmatic Signer" on devnet and unaudited, or Blueshift "Vector"). Removal is expected about 6 months after the replacement ships, with no date set. https://solana.com/upgrades/durable-nonce-deprecation
  - **So for the canister:** use getSlot plus getBlock (IC round-trip of a few seconds), or durable nonces now with a planned migration later.
- **Transaction size:** legacy and v0 transactions are capped at 1,232 bytes. A new **v1 format allows 4,096 bytes**; it was reported live 2026-09-15 at epoch 1035 (secondary sources). https://solana.com/docs/core/constants-reference

## 4. Relay (relay.link) on Solana

- **Chain IDs** (live `/chains`, 2026-10-09):
  - Solana `792703809`: SVM, deposits enabled, any Jupiter token; native SOL address `11111111111111111111111111111111`.
  - Bitcoin `8253038`: BVM, BTC only; currency address `bc1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqmql8k8`.
  - Ethereum `1`.
- **Quotes from a Solana origin:** the `deposit` step returns `data.instructions[]` plus `addressLookupTableAddresses`, not an EVM transaction. You build the VersionedTransaction yourself, and it must fit 1,232 bytes; otherwise you get `SOLANA_TX_TOO_LARGE`, mitigated with `maxRouteLength` or `includedOriginSwapSources`.
- **SDK:** `@relayprotocol/relay-svm-wallet-adapter`.
- **API key:** `/quote/v2` "requires `x-api-key` from 2026-10-02". My key-less calls still worked on 10-09, so plan for the key anyway.
- https://docs.relay.link/references/api/api_guides/solana
- **Live quotes (2026-10-09):**
  - **SOL→ETH:** 20 SOL ($2,190.54) → 0.8764 ETH ($2,180.60). Relayer fee $4.71 (about 0.21%), estimated time about 4 s. One instruction to program `99vQwtBwYtrqqD9YSXbdum3KBdxPAVxYTaQ3cfnJSrN2` plus one lookup table.
  - **BTC→SOL:** 0.01 BTC → 7.438 SOL. Fees about $11.08 (gas $4.12 plus relayer $6.95), estimated time 1, units not given. Used a deposit address with `refundTo`.
  - **SOL→BTC and ETH→BTC:** both returned `INSUFFICIENT_FUNDS` "No UTXOs available to spend" with my test recipient. It may be solver liquidity or an input quirk. **Check before relying on SOL→BTC for ordinals buys**; a fallback is routing through ETH or USDC with a different BTC leg.
- **Bitcoin notes:** deposit-address flow needs 1 confirmation, or 2 for high value. Withdrawals to BTC come back as a PSBT. https://docs.relay.link/references/api/api_guides/bitcoin
- **Solana depository program:**
  - An Anchor program holding funds in a PDA vault. Instructions: `deposit_native` and `deposit_token(amount, id[32])`, using token_interface so Token-2022 works.
  - Funds go out through `execute_transfer` with an allocator signature and replay protection. The owner can rotate the allocator.
  - Refunds are not a program instruction. Relay's off-chain refund paths, including "Fast Refund", go through the allocator.
  - https://docs.relay.link/references/protocol/contracts/solana-depository

## 5. Oracles

- **Pyth:** no NFT or ordinals floor feeds found. Its catalogue covers crypto, FX, equities and commodities.
- **Switchboard:** shut down. Announced 2026-09-19, all support ended **2026-09-25**; it recommends Pyth or RedStone. https://solanacompass.com/news/switchboard-oracle-protocol-shuts-down-giving-solana-defi-six-days-to-migrate
- **Conclusion:** Rook's keeper keeps posting floors.
  - Port the EVM rules on-chain: 3 h maximum age, deviation bounds, and a guardian pause.
  - Consider several keeper signers verified with the Ed25519 precompile, or let the IC canister co-sign (it already has a threshold key).

## 6. Randomness (if draws are needed)

- **Switchboard VRF** is gone with the shutdown.
- **MagicBlock VRF** is live on mainnet with proof checked on-chain and a callback. It costs about 0.0005–0.0008 SOL per request (Korean pricing page, confirm the English one). https://docs.magicblock.gg/pages/verifiable-randomness-functions-vrfs/introduction/solana-vrf
- **ORAO VRF:** Ed25519-based, with a callback crate. I could not confirm 2026 status. https://docs.rs/orao-solana-vrf
- **Fallback:** commit-reveal with a future slot hash, keeper-seeded. Weaker.

## 7. Wallets, web stack, indexing, RPC

- **Client library:** `@solana/kit` (formerly web3.js v2) is the default. web3.js v1 gets security fixes only, and web3.js v3 (rc) is a compatibility layer.
- **Wallets:** use Wallet Standard discovery (`@solana/kit-plugin-wallet`) instead of per-wallet adapters. Phantom, Solflare and Backpack all register through Wallet Standard. https://solana.com/docs/frontend/web3-compat
- **Anchor clients:** check Kit compatibility via Codama-generated clients; some sources say Anchor's TypeScript client is still v1-centric.
- **Helius:**
  - Free: 1M credits, 10 rps.
  - Developer: $49, 10M credits.
  - Business: $499, 100M credits, mainnet LaserStream gRPC.
  - Professional: $999, 200M credits.
  - Streaming costs 2 credits per 0.1 MB. Webhooks are available.
  - https://helius.dev/pricing
- **Triton:** Yellowstone/Dragon's Mouth gRPC on usage pricing (about $0.08/GB per a competitor; no official card found). https://www.triton.one/solana
- **QuickNode:** supports Solana, including Yellowstone add-ons.
- **Dwellir:** its pricing is multi-chain ($49, $299, $999 tiers), but one comparison lists Robinhood Chain only, so **confirm Solana availability**. https://www.dwellir.com/docs/getting-started/pricing
- **Note:** Helius is also one of the SOL RPC canister's providers.

## 8. Market context: is Solana the right move?

- **Solana launchpads:** pump.fun is dominant inside Solana at roughly 62–73% revenue share; one undated snapshot gave pump 73.3%, LetsBonk 6.95% and Meteora DBC 5.48%. August weekly fees hit a record $10.03M. https://intellectia.ai/news/crypto/pumpfun-dominates-solana-launchpad-market-with-733-share
- **Cross-chain:**
  - Aug 30–Sep 5, 2026: **Pons (Robinhood Chain) averaged $625M per day against pump.fun's $562.4M**. Weekly fees were $36.4M for Pons and $9.4M for pump.fun. https://cryptocomresearch.substack.com/p/major-memecoin-launchpad-recorded (2026-09-07)
  - On Aug 31, Pons took about 64% of all launchpad fees, and Robinhood Chain was #2 in DEX volume behind Solana. https://thedefiant.io/news/blockchains/robinhood-chain-dex-volume-hits-usd1-49-billion-as-pons-takes-two-thirds-of-launchpad-fees
- **Bottom line:** Solana is still the deepest memecoin ecosystem (DEX volume #1, Jupiter, wallets), but not uncontested. Robinhood Chain (EVM, and Dwellir supports it) deserves a look as an alternative or second target that would reuse Rook's EVM code.

## 9. Gotchas

- **Compute:** 1.4M CU per transaction and 200k default per instruction. Blocks rose to 100M CU on 2026-07-29 (SIMD-0286); writable accounts are capped at 12M CU per block, which matters for one hot curve or reserve account.
- **Accounts:** at most 64 locked accounts per transaction (128 behind a feature gate). Accounts up to 10 MiB, realloc up to 10 KiB per call, heap 32 KiB by default (256 KiB max).
- **CPI depth:** 4 nested calls, rising to 8 with SIMD-0268. Official docs describe it as shipping, but one tracker says it is not active on mainnet, so check the feature gate.
  - Our program → DBC → Token-2022 → transfer hook already uses 3 of the 4.
- **Rent:** every account needs a rent-exempt deposit. Budget for per-pool, per-user and per-redemption PDAs and close them to recover rent.
- **Fees and MEV:**
  - Use priority fees with simulated CU limits, and Jito bundles or tips for graduation and keeper transactions.
  - Sandwiching moved to private mempools after Jito's mempool shut down. Jito DontFront narrows the window but guarantees nothing. https://solana.com/docs/defi/mev-protection
  - Bonding curves are deterministic, so on-chain min-out/slippage checks, per-transaction buy caps and fee scheduling at launch are required. The graduation slot is a known snipe point.
- **Token-2022 compatibility:**
  - DBC supports Token-2022 and hooks; the migration target is DAMM v2.
  - Jupiter shows extensions as warning labels. Transfer hooks get limited routing, and permanent delegate scares users.
  - Keep the meme token and wX free of transfer fees and permanent delegate. Hook-gate only if necessary.
- **Finality:** Alpenglow (about 150 ms finality) runs on testnet since 2026-09-24, with no mainnet date. Today, finalized commitment takes about 12.8 s, so the canister or keeper should act only on finalized state.

---

## Recommendations for Rook

1. **Launch on Meteora DBC first.**
   - Set `migration_fee.fee_percentage` to the floor share, with the partner `fee_claimer` as a FloorModule PDA. Add partner trading fees, an anti-sniper fee schedule at launch, graduation to DAMM v2, and partner LP locked.
   - Build our own curve only if the full invariant (wX mint and lock at graduation) must be atomic.
2. **Write the FloorModule, wX and redemption programs in Anchor 1.2.x.**
   - Use plain SPL or Token-2022 with metadata only for wX; no permanent delegate and no transfer fee.
   - Use Metaplex Core for per-NFT custody and redemption receipts.
3. **Governance:**
   - Squads v4 vault as upgrade authority, `time_lock = 259200`, autonomous config.
   - A separate guardian-pause authority.
   - Verified builds through solana-verify and OtterSec, and two audits (for example OtterSec or Neodyme plus Zellic or Offside).
4. **Floors:** the keeper posts floors with a 3 h staleness limit enforced on-chain, optionally co-signed by the canister's threshold Ed25519 key. No oracle covers NFT floors, and Switchboard is gone.
5. **Canister:**
   - Use threshold Ed25519 and SOL RPC with 3-of-N `Equality`, and pin the correct canister ID.
   - Fetch blockhashes with getSlot plus getBlock. Use durable nonces only behind an abstraction, because they are being deprecated.
   - Budget about 26B cycles per signature.
6. **Relay:**
   - Get an API key.
   - Handle instruction-and-lookup-table responses and enforce the 1,232-byte limit, or adopt v1 transactions when Relay supports them.
   - **Test SOL→BTC before relying on it**; the to-BTC quotes failed on 2026-10-09.
7. **Frontend and infra:** @solana/kit, Wallet Standard, Helius Business ($499 for LaserStream) or Triton for indexing, with Jito-routed sends.
8. **Before committing to Solana only,** compare Robinhood Chain (Pons) traction. The EVM codebase may port there almost free.
