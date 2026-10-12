/**
 * docs/v6/phase1-scope.md §0 and §2, for the Trust page of a Phase-1 build (K5): the launch's own risk statement in the visitor's
 * words, not a summary that is more comfortable than the contracts. Every figure here is the document's, which cites the code
 * (DeployStack.s.sol, StackChecks.sol, ProtocolTreasury.sol, Wrapper.sol, the canister's custodycap.rs and outflow.rs), and the
 * roles-and-powers table of docs/audit/2026-09-26/fixes-B-C-keeper-draws.md §3 (C-06: the Safe takes the whole treasury; C-09: the
 * keeper's reach, the guardian, the pause and rotation, the stale fallback). Plain data, so test/phase1.test.ts holds the sentences
 * that must be said, and refuses the ones the code does not back (no "no withdraw path").
 */
export type Para = { text: string; list?: string[] };
export type Section = { id: string; title: string; body: Para[] };

export const PHASE1_TRUST_LEDE =
  "Read this before you put anything in. It is the launch's own risk statement (docs/v6/phase1-scope.md §0), said plainly. If any sentence here is more comfortable than the code, that is a bug: say so.";

export const PHASE1_TRUST: Section[] = [
  {
    id: "caps",
    title: "The stage-0 caps bound custody only",
    body: [
      { text: "At stage 0 the canister that holds the items may hold at most 15 ETH of value per Ethereum collection (one item up to 15 ETH). At launch it holds at most 13 Miladys, 17 Azukis, 13 Mutant Apes, 3 Pudgy Penguins and 2 Bored Apes, 48 items in all, and may send out at most 12 items a day per collection and 24 a day in all, mint 3 a day per collection and move 25 ETH a day, with a tripwire at three times any of those (it trips on asks for more than custody holds; a waiting mint or advance counts once, however long it waits). Every item must be able to leave within the timelock's delay less a day: each collection alone and the whole custody, on the caps and on what custody actually holds at any upgrade; the canister refuses an install or upgrade that does not fit, and delivers the collection with the most items waiting first, so they all can. The operator's plan (the 2026-09-27 review, round 5): once the launch install has run, a timelocked upgrade raises only the daily total to 31 items (announced, 72 hours of review; it runs no earlier), and after two clean weeks a second one raises the item caps to what 15 ETH buys at each floor (16 Miladys, 22 Azukis, 17 Mutant Apes, 4 Pudgy Penguins, 2 Bored Apes: 61, the operator's decision J7), which 31 a day lets out within the delay. There is no canister-wide total beyond the sum of the collections' caps. A deposited NFT counts at its collection's floor. This page states the numbers installed at each step. These caps bound what custody holds, signs and pulls." },
      {
        text: "They do not bound the ETH that sits in the contracts on Robinhood Chain. At stage 0 that is:",
        list: [
          "each curve's raise: 5 ETH normally, 8 ETH early, 30 ETH at most;",
          "the Floor Module's ETH-backed units, up to 40 items' worth (about 25 ETH at a 0.63 ETH frog floor), a limit that does not shrink with the stage;",
          "the locked pools, which can never be withdrawn and have no cap;",
          "the sweep vaults, the protocol treasury, the mint desk's escrow and the offer board's bonds.",
        ],
      },
    ],
  },
  {
    id: "curves",
    title: "The number of open curves, and the ETH in them, is not capped",
    body: [
      { text: "Anyone can launch a coin, and nothing limits how many curves are filling at once, so nothing limits the total ETH held in curve contracts. Each curve is bounded only by its own raise target. The operator was asked on 24 September 2026 whether to cap the number of open curves, and decided not to." },
      { text: "Graduation can wait. The buy that would close a curve is refused while the floor module has no room for the coin's item budget, and while the keeper is paused; the curve keeps trading and closes when room appears (conversions and burns free it). A public mint at the module may take room a closing buy was about to need: that is accepted and costs only time. A graduation already queued when the keeper is paused holds the curve's refunds while it is paused and for 6 hours after the unpause (so the keeper posts and the graduation is filled before anyone can force refunds), up to 4 more days, never longer. A curve that closed but never graduates opens refunds through its escape hatch 7 days after the close." },
    ],
  },
  {
    id: "audit",
    title: "The contracts are not professionally audited",
    body: [
      { text: "No audit firm has reviewed these contracts. On 26 September 2026 they were AI-reviewed, not professionally audited: five automated reviews of the Phase-1 stack reported 1 critical, 6 high, 18 medium, 26 low and 24 informational findings, and fixes for them were written before anything was deployed. The findings, the fixes and what was left open are in the repository at docs/audit/2026-09-26/README.md. An AI review can miss what a professional reviewer would find." },
      { text: "The launch gate that requires an audit (G1) is waived for stage 0 only, by the operator's decision of 24 September 2026. Because there is no professional audit, the caps may not rise past stage 1." },
    ],
  },
  {
    id: "safe",
    title: "What the Safe can do, and so anyone holding both of the operator's devices",
    body: [
      {
        text: "The operator is one person with two devices, A and B. The contracts' owner is a 2-of-2 Safe with one key on each device. Whoever holds the Safe can, at once and with no delay:",
        list: [
          "take the whole protocol treasury, allocated pots included: the Safe (2-of-2) can take the whole ProtocolTreasury at once, by registering a module of its own (addCollection), setting the shares, the operations address and the loan (ProtocolTreasury.setShares and setOps, setBootstrapLoan), and then allocating, distributing and bootstrapping it all away;",
          "pause any listing, which stops sweeps and draws (never redemptions, never burns at the floor module);",
          "cancel any draw before its round is pinned (the whole unit goes back to the coin);",
          "set a sweep vault's cap ceiling (the first one at once, a lower one at once, a higher one after 7 days);",
          "pause and unpause the keeper, and name or replace the guardian;",
          "transfer ownership of the contracts (the two-step ownership transfer has no waiting period).",
        ],
      },
      { text: "After 7 public days it can install a keeper of its own, and then do what the keeper can (below). It cannot move the floor module's ETH, a sweep vault's ETH, a curve's ETH or the locked liquidity, record a delivery, or post a floor or a cap without first becoming the keeper." },
      { text: "Through the canister's timelock, after its 72-hour public delay and with nobody else to veto it, they could also upgrade the canister, then mint unbacked wrapped units up to the wrapper's limit of 3 a day and sell them into the pools." },
      { text: "So \"both devices compromised means at most the custody caps lost\" (75 ETH for the five Ethereum collections) is true of custody only. It is not true of the treasury, the curves or the pools." },
    ],
  },
  {
    id: "keeper",
    title: "What the keeper key can do, and the worst case until it is paused",
    body: [
      { text: "The keeper is a hot key on a server. It posts the floor module's floor (at most ±5% an hour and ±20% in any 24-hour window), posts the sweep vaults' price caps (never above a ceiling the Safe set, which must exist, and up by at most 20% a post), and announces and opens raffle draws." },
      {
        text: "If it is stolen, then until someone pauses it the thief can:",
        list: [
          "mint wrapped units at the floor module at a floor walked down within those bounds, worth at most about 1.3 ETH a day for frogs (the public ETH-backed limit × the floor × 20%);",
          "let sweeps buy at prices up to the ceiling;",
          "publish a forged ticket list for a draw, or an empty one (\"nobody held a whole coin\") to throw a snapshot time away: anyone can catch it by rebuilding the root, which commits the ticket count and the snapshot time, and the Safe or the guardian can cancel the draw until its round is pinned or it is closed; the coin's next draw then gets the same snapshot time;",
          "freeze the floor module by posting nothing, or a floor with too little confidence: pausing the keeper ends that at once (burns then pay the floor it left, at most the backing per unit), and the stale fallback below ends it after 7 days if nobody pauses.",
        ],
      },
      { text: "It can no longer drain the reserve's surplus through burns, record a delivery, choose a draw's snapshot block, re-roll a draw, or keep the floor module's ETH locked." },
    ],
  },
  {
    id: "guardian",
    title: "The guardian can pause and cancel, nothing else",
    body: [
      { text: "The guardian is an optional automated key kept on the monitoring host, not with the keeper. Its powers are pause and cancel only: it can pause the keeper, pause one Ethereum collection's floor poster (that collection only), and cancel a draw before its round is pinned. It cannot unpause, move any ETH or token, post a price or change a setting. At worst it stops things, and the Safe unpauses and replaces it." },
    ],
  },
  {
    id: "rotation",
    title: "Pausing the keeper is instant; replacing it takes 7 days",
    body: [
      { text: "The Safe or the guardian can pause the keeper at once. While it is paused every keeper power stops everywhere, and so do mints at the floor module, graduation fills, curve closes and new draws. Burns do not stop: from the moment the keeper is paused they pay the floor the keeper left or the backing per unit, whichever is lower, less the base spread, with no 7-day wait (so a coin minted just before a pause cannot exit above what it paid); the backing per unit itself after a week with no good post. A new keeper takes 7 public days on both clocks: the Safe proposes it, waits the 7-day keeper rotation, accepts it, then unpauses." },
    ],
  },
  {
    id: "stale",
    title: "If the floor goes stale for 7 days, holders can always burn at backing",
    body: [
      { text: "If no good floor is posted for 7 days and 3 hours, the stale fallback turns on: anyone can burn ETH-backed wrapped units at the backing per unit, less the base spread, with no floor price and no key. While the keeper is paused (or none is named) burns pay exactly that price at once, from the moment of the pause, and nothing can be minted meanwhile, so nobody can buy into it. So a paused keeper does not delay exits at the floor module at all, and a silent one that nobody pauses delays them at most 7 days; neither can keep the ETH that backs the units locked." },
      { text: "An ordinary keeper outage (not paused, stale for less than 7 days) is the one time a burn waits: it reverts until the next good post, or until someone pauses the keeper." },
    ],
  },
  {
    id: "keyless",
    title: "Raffles and redemptions finish without the operator",
    body: [
      { text: "A raffle relies on the keeper's published ticket list plus a cancel before the pin: anyone can check the list against the root. Pinning a round, resolving it, claiming for the winner and voiding a stuck draw are anyone's to do, and this site offers each step. A drand round, once pinned, is final: there is no redraw." },
      { text: "A redemption ticket nobody resolves is refundable by anyone 7 days after its pin. One resolved and not delivered is refundable 72 hours after its delivery deadline once the custodian attests on chain that the delivery can no longer confirm (Wrapper.markUndeliverable; the canister does this for every honest non-delivery), and 30 days after its deadline without that word, for a custodian that is gone. Until then a late delivery is still recorded, so a refund can never race a delivery that confirms after all." },
    ],
  },
  {
    id: "relay",
    title: "Buying a frog trusts Relay for a few minutes",
    body: [
      { text: "No operator money buys frogs. When the floor module wants a frog, it advances that frog's budget (at most the floor plus 3%) to the canister's own address on Robinhood Chain; the canister converts it to bitcoin through Relay (relay.link), then buys the frog with it. While the ETH is on its way, that one wrapped unit is backed by it, not by a frog, and no burn can take it." },
      { text: "The canister checks every quote itself before it signs: the deposit goes only to Relay's pinned depository, the order it commits to (its hash) pays only the canister's own bitcoin address and refunds only its own addresses, and it must promise at least our own reference rate less 3%. It never asks Relay's servers anything." },
      {
        text: "What is trusted, and how far:",
        list: [
          "Relay's solver and its settlement keys, for the minutes between the deposit and the bitcoin arriving (about five, measured): they could fail to pay and fail to refund;",
          "the exposure is one advance at a time, at most three a day, each at most the floor plus 3%;",
          "a conversion that is not paid or refunded by its order's deadline is flagged as stuck and nothing new is bought; its recovery is Relay's withdrawal process or its support.",
        ],
      },
    ],
  },
  {
    id: "ethnft",
    title: "Ethereum NFTs: held by the canister's own Ethereum address",
    body: [
      { text: "For an Ethereum collection (docs/v7/phase1-nft.md), each NFT is bought on Ethereum through Seaport by the canister and stays at the canister's own threshold-key address on Ethereum: no person holds that key, and no bridge or 6-day message is involved. Anyone can check every wrapped item with ownerOf on Ethereum; the reserves page does it on two providers." },
      { text: "The floor module advances one item's budget to the canister, which moves it to its own Ethereum address through Relay (seconds, measured) after checking the order's hash names only its own addresses. It buys one listing only if the price is at most its own floor reading plus the collection's premium (the install's max_premium_bps, at most 10%; 5% at launch, which the go-live check holds) and at most what arrived, the order sells exactly one NFT of this collection to it, fees go only to known recipients, and the exact purchase succeeds in a simulation first. The item is counted on Robinhood Chain only once the purchase is final on Ethereum. No advance is made unless what would reach Ethereum covers the module's floor plus 1% after Relay's estimated fee (a pooled collection whose floor rose past its pool waits on Robinhood Chain, where its ETH stays burnable), and ETH that has waited on Ethereum 48 hours after it first arrived (a purchase that fails does not restart the clock) without a listing within the bound goes back to the module through Relay the same way it came. If no acceptable quote for that way back comes within 2 hours, the ETH waits for a listing again and tries a day later; a way back Relay neither pays nor refunds stays watched past its order's deadline, so a late payment is read as that order's own (the other collections' buys wait meanwhile); no earlier than a day after the deadline, the operator may write it off once Relay confirms nothing more will come, as a loss to that collection's pool, and even then no new Relay order starts until a payment of it is credited or the operator, from Relay's own records, releases it (only once Relay's records show its ETH will never reach us), so a late payment is not read as another order's; a payment that comes after all can be credited to that same collection from its own transaction, which must name the order, or be Relay's own payment to the canister out of its deposit contract, which the operator names from Relay's records for that order, at an amount that contract recorded paying the canister. A collection whose advance came back waits a day before its next one, so repeated round trips cannot eat its pool." },
      { text: "The canister posts this collection's floor, not the keeper: the second-cheapest of the live Seaport listings it verifies itself (signature, owner, status, approval, fillable by anyone), one per seller. Once a collection has a floor, recent sales can only hold the floor back, never set it and never stop a post (its first post waits for three of the collection's own counted sales): the canister reads the collection's own sales on Ethereum itself (nobody hands them in), counts one sale per token and per wallet, and none of a token that went back to an earlier holder, a self-trade or a far price; it reads the collection's transfers one per token and per sender first, oldest first, so a flood of transfers cannot hide a sale from it, and a range too full of transfers to read is read through the marketplaces' own sale records (Seaport and Blur) and the reading goes on past it, its other sales read later, so it cannot stall the reading either; the floor rises only as far as 10% over the lower quarter of those sales, and follows the listings down within the module's own limits, ±5% an hour and ±20% a day. So wash trading can slow a rise, never freeze a posted module. The monitor's guardian can pause this collection's floor alone (its poster), when the canister's reading is more than 20% from two public references that agree; pausing the keeper at the registry stops every collection's posts at once." },
      {
        text: "What is trusted, and how far:",
        list: [
          "the canister's threshold key holds the NFTs at rest: an upgrade through the 72-hour timelock could move them; the stage-0 caps (15 ETH a collection, 75 ETH for the five) bound that;",
          "Relay, for one advance at a time (at most what the buy may cost: the floor plus the 5% premium, the gas reserve and Relay's largest allowed fee, never over the module's item cap), for the seconds between the deposit and the ETH arriving on Ethereum; what the item did not cost is swept back to that collection's floor module once more than 0.05 ETH waits at the canister's Ethereum address;",
          "the operator's runner chooses which valid listings to hand in (not the sales: the canister reads those itself): by withholding cheap listings it can lift the posted floor, but never above 10% over the lower quarter of the collection's own recent sales, and by handing in no listings it can stop posts (the module then goes stale); the module's ±5% an hour and ±20% a day and the monitor bound it. Burns never pay more than a unit's backing; with holder deposits on, a deposit is minted only while the module's floor is at most 5% over the lower of the listing floor and the lower quarter of the collection's own recent sales (one per token and per wallet), so a floor above the market is not paid out through deposit then burn (the 2026-09-27 review, finding A-M-1, round 2). What remains: a wash trader with more separate wallets and tokens than three quarters of the week's real sellers and buyers can lift that quarter (the canister reads the whole week itself, from two Ethereum providers that must agree; a reading that falls behind, a part of the week both providers refuse, or a provider rate-limiting it for hours pages the operator, and each provider is checked at go-live on a past range known to hold sales);",
          "holder deposits are on at launch: you send an NFT of the collection to the canister's Ethereum address from your own wallet with safeTransferFrom and the data \"EWN1\", the chain 4663, the wrapper and your Robinhood Chain address (48 bytes); once it is final on Ethereum you get one wrapped unit. The 48 bytes are exactly \"EWN1\", then 4663 as a 4-byte big-endian number (0x00001237), then the 20-byte wrapper address, then your 20-byte address. An NFT of the collection sent any other way goes back to the address it came from once final, one at a time within a daily budget; it is held for the operator instead (not returned automatically) when the return cannot be judged or keeps failing: the wrapper already holds that token, an open buy or claim uses it, it left the canister after it came in, more than four came in one transaction, or every Ethereum provider agrees for an hour that sending it back fails. An NFT of a collection the canister does not serve is never touched and cannot be returned.",
        ],
      },
      { text: "A redeemed NFT is sent with transferFrom to the address you give, on Ethereum. If it cannot be sent (the collection blocks it, or it is still pending a day before its deadline and is cancelled), the canister says so on chain and your wrapped unit is refunded 72 hours after the deadline." },
      { text: "A delivery is recorded from its own transaction once that is final on Ethereum: what you do with the NFT after it arrives (sell it, move it to another wallet) does not matter. Once the transfer is final the canister also notes it on the wrapper (noteSent) before it records it, and from then on the wrapper's 30-day late refund is off for that ticket: a delivered NFT, once noted sent, can never be refunded as well (the 2026-09-27 review, finding D3-H-1, round 3). A transfer that a reorganisation of Ethereum drops before it is final is never noted, so its ticket can still be refunded (round 4, A4-L-2)." },
    ],
  },
  {
    id: "redemption",
    title: "Redemption cannot be paused",
    body: [
      { text: "Burning one whole wrapped unit for a random item has no pause, by design: nobody, the operator included, can switch it off. Once any item is in custody the delivery path is live for as long as the contracts exist." },
    ],
  },
  {
    id: "fueler",
    title: "The ops share of the fees is operator income, and part of it buys the canister's cycles through a hot key",
    body: [
      { text: "The treasury pays 10% of what it receives to one address, the ops address, at once and with no delay. That share is operator income. A service called the fueler holds the ops key: it claims the curves' platform fees, splits the treasury, and when the buyer canister runs low on cycles it converts some of that ETH into cycles (Relay to Ethereum, ckETH, ICP on ICPSwap, then the cycles minting canister) and tops up only the buyer and its timelock." },
      {
        text: "It is a hot key on money, bounded in its own code and watched from outside:",
        list: [
          "it can sign only five kinds of transaction: a Relay deposit to itself, a fee claim, the treasury's split, the ckETH deposit to its own principal, and a transfer to the operator's cold address. Anything else is refused before it is signed;",
          "it converts at most 20 T cycles a run, 60 T in seven days, 0.10 ETH a day and 0.25 ETH in seven days, and checks two price sources and the pool's quote before it swaps;",
          "it keeps at most 0.15 ETH and sends everything above that to the operator's cold address, so a stolen key takes at most that buffer and the caps until the Safe replaces the ops address, which is instant;",
          "the monitor reads the ops address's transaction count on both chains and pages at the highest severity when anything the fueler did not record was signed with that key.",
        ],
      },
      { text: "None of this touches custody: the fueler is not the canister's controller, cannot call it, and moves none of the items or the collection reserves. The other 90% of the treasury's income stays in the reserves." },
    ],
  },
];
