# Trust model

Who can do what, and what is left to somebody's word, for canister-only Rook on Solana (`docs/v9/canister-only-solana.md`). Read with
that design's §10 and the audit record `docs/v9/audit-2026-10/README.md`. Internally reviewed with proof-of-concept tests and fixes
(A1–A6, the B0 review, Round B, and Round C for the Solana NFT flow, the Ordinals beyond Bitcoin Frogs, Robinhood Chain NFTs and opening collections on demand); not yet deployed with real money. The trust model of the
earlier Robinhood Chain design (contracts, keeper, purchaser, attestor) is `docs/v5/trust-model-evm.md`; nothing of it is deployed.

**The one sentence.** Rook deploys no programs on Solana. The wrapped tokens (wX) are ordinary SPL tokens whose only mint authority
is the custodian canister, and the books that say what backs them are the canister's own state. Solana's validators enforce
nothing about the backing; the canister's code does. What makes that checkable rather than blind: every book is a public query,
every wX mint or burn carries a memo naming its book entry, the canister checks the chain against its books before every
signature, and an outside reconciler matches the books against Solana, Ethereum, Robinhood Chain and Bitcoin every few minutes.

## Actors and their powers

| Actor | Can | Cannot |
|---|---|---|
| **The custodian** (the canister: threshold Ed25519, secp256k1 and BIP340 keys, split across an IC subnet) | mint and burn every wX; claim every DBC config's fees and migration fee; hold every coin's wX vault, every pool desk position, the treasury key and every intent's deposit address; buy NFTs on Ethereum, Robinhood Chain, Bitcoin and Solana and deliver them; sign any transaction **its code builds** (every builder is byte-checked against landed fixtures; every judged foreign transaction is rebuilt and compared) | sign anything its books do not cover (`pre_sign`: supply never over the ledger's, a payout never under the module's books, one open link per collection); act while two providers disagree; move an NFT except to a redeemer's chosen destination or a prize winner; stop a holder burning their own wX |
| **The controller** (a 72 h timelock) | install any new build of the canister after the delay, in public | shorten the delay; act on the canister's keys without a build that does |
| **The operator** | pin collections (checked as the canister checks them); set the treasury split, the ops key, the reserve cap and the raffle share (bounded; **24 h** delay, the first ops key at once); leave an account out of a redemption's supply (24 h); pause a collection, the desk, every mint, a module, or the whole canister; clear a halt or an incident once looked at; book unsolicited lamports as surplus; decide a stuck link **after the chain is asked** (a finalized verdict wins; "landed" is never taken on their word); ask a resume | move money by hand (there is no transfer method); lift a pause at once (every resume applies **24 h** after it is asked, and a new pause clears it); mint, burn or redeem outside the books; change a pinned collection |
| **The guardian** (separate keys, all held by the operator: decision 2026-10-12, no outside guardians) | pause a collection, a pool desk or every mint | anything else |
| **A holder** | open an intent (free, bounded: 200 never-funded, 300 funded, 5 an hour a caller, 3 open a recipient); send SOL or wX to it and get a mint, a burn, a coin redemption or an NFT ticket at the price of the moment, or the deposit back less the fee; burn their own wX on chain (booked by the reconciliation; the backing stays with the module); redeem a whole wX for a random NFT; claim a raffle prize | pick which NFT; be refused by another's free openings; make the canister sign below the minimum deposit |
| **Meteora** | upgrade DBC, DAMM v2 and DLMM with one key | change or close a Rook config (none exists); re-point a fee claimer |
| **Two Solana RPC providers** | each answer anything | forge a read alone: every read that moves money needs both to agree; both colluding can |
| **Two drand relays** | each serve any beacon | bias a round alone: both must agree and the BLS signature is verified by the canister; the League of Entropy's own threshold could |
| **Relay** | fill or refuse a SOL → ETH / BTC conversion | reach more than the one deposit's key; be paid under the judged quote's floor |
| **Marketplaces** (OpenSea, Tensor, Magic Eden, Ordinals Wallet, UniSat) | list, edit, cancel, cut API access | raise what the canister pays past the ask it judged (the venue's own price check, verified on Tensor's mainnet bytes); deliver anything but the judged item |
| **The IC** | run the code; hold the key shares | — (it is the root of everything above) |
| **Anyone** | burn their own wX; plant accounts at derived addresses; send dust to any address; read every book | halt a collection by any of those (each is booked, allocated over, or ignored) |

## Residual trust, in order of how much money it touches

1. **The custodian's code and its controller.** A build that does the wrong thing can mint wX, pay a vault elsewhere or deliver an
   NFT wrongly. The memo on every mint and burn, the public books and the reconciler make it **visible within one reconcile cycle
   (10 min)**; nothing prevents it. The timelock makes a new build public 72 h before it runs; the reproducible build lets anyone
   compare what runs with what was reviewed.
2. **The IC.** The subnet that runs the canister and the subnet that holds the key shares. A subnet's nodes jointly are the
   custodian.
3. **The two providers.** Every read that leads to a signature is agreed by both at `finalized`. Two colluding providers can show
   the canister a chain that is not there; one cannot.
4. **Floor prices.** Read by the canister from the marketplaces, bounded to 20 % a day, stale after 3 h. A wrong floor prices a
   mint or a burn wrongly within those bounds and stops at the stale mark.
5. **NFT purchases.** The canister buys within a limit the backing sets, through asks it verifies on chain and transactions it
   judges instruction by instruction; the conversion legs go through Relay from a key that holds only that deposit. A venue that
   lies about an ask is refused by the venue's own program; a venue that goes dark delays purchases.
6. **Custody.** Keys, not contracts: threshold keys on the IC. 48 NFTs at launch, capped per collection.
7. **drand.** Two relays, verified signatures; the beacon's own threshold could bias a round, and nothing can see that.
8. **Meteora.** An upgrade could change how every coin and pool behaves; the canister reads Meteora's current version and must be
   retested whenever it changes.
9. **The venues' terms.** Any marketplace can cut access; the design depends on no single venue.

## What is NOT trusted

Each of these has tests that try to break it (`autobuyer/canister-rs/tests`, the `audit_*` files among them):

- the books are written before a signature and undone if the transaction never lands; the chain is checked against the books
  before every signature (`pre_sign`): the wX supply never exceeds the ledger's, a payout never leaves the module short of its
  books, one books-changing link per collection at a time;
- a supply under the ledger's is a holder's own burn, booked, never a halt; over it is a hard stop;
- a book write made while a link is open is deferred, never wiped by the link's undo;
- every money setting waits 24 h; every resume waits 24 h; no method lifts a pause at once;
- a raffle snapshot is published before it is committed, committed at a later slot, drawn with a drand round pinned after the
  commit and verified; the canister's own addresses carry no weight;
- a judged transaction carries only the instructions the canister rebuilt and a compute-unit limit, never a priority fee;
- a token refund is paid from the deposit's own rent, so a griefer pays more than the canister;
- a pinned curve DBC would refuse is refused before it takes an index; the migrated pool's fee shape is pinned;
- memos are labels: they decide which entry a landed transaction is read as, never whether money moved.

## Operational rules the code cannot enforce

- **The guardians are the operator (decision 2026-10-12).** Separate keys on separate devices protect against one lost or stolen key,
  not against the operator. Nobody outside the operator can veto an upgrade: the 72 h delay makes it public, nothing more.
- **Two providers on different infrastructure**, and the reconciler run outside the canister with its own two.
- **The ceremony's read-back.** Every wX mint (authority `K_coll[i]`, no freeze authority, 9 decimals) and every DBC config are
  read back from chain before a collection is listed; the canister refuses a config that differs from its pins.
- **The subnet.** The canister's subnet and the key subnet are the trust root: chosen once, named in the launch record.
