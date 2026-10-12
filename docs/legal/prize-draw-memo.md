# Internal memo: the random draw, and the other questions a lawyer must answer before mainnet

**For:** the owner and their counsel. **Not for the site. Not legal advice.** Written 2026-09-21 by the people who wrote the
code, who are not lawyers, without access to a law library or a search engine. It cites no statute or case, because none could be
checked; where it names a body of law it names the kind, not the section. Its job is to make the first hour with a lawyer useful.

## 1. The issue in one paragraph

etched buys a Bitcoin asset with a coin's fees and, for an inscription, gives it to **one** holder chosen at **random**. In most
legal systems a scheme is a lottery (or an illegal gambling or prize promotion) when three things are present together: a
**prize**, **chance**, and **consideration** (the participant paid or gave something of value to take part). Lotteries are, almost
everywhere, either a state monopoly or a licensed activity. The question is whether an etched draw has all three.

## 2. How the three elements map onto this design

- **Prize: yes.** An inscription with a market price, bought on the open market. There is no honest argument that it is not a prize.
- **Chance: yes, for the two draw modes.** A drand round that did not exist when the draw was pinned picks the ticket. That is
  the point of the design, and it is pure chance, not skill. `DrawWeighted` is chance weighted by balance; `DrawPerWallet` is
  chance with one ticket a wallet.
- **Consideration: this is the question.** Nobody pays to enter a draw. A ticket comes from holding the coin at the snapshot.
  But the coin was bought, the purchase paid Pons's fee and the creator tax, and **those fees are what bought the prize**. A
  regulator can say: people paid money (for the coin), part of it funded a prize, and the prize was awarded by chance; the
  holding *is* the entry and the purchase price *is* the consideration. The counter-arguments a lawyer will weigh: the buyer
  received the coin itself, at its market price, and still holds it (consideration for the coin, not for a chance); a holder who
  received the coin by transfer paid nothing to anyone; the fee is charged on every trade whether or not any draw ever happens.
  We do not know which view prevails where. Jurisdictions differ on whether "consideration" means money paid **for the chance**,
  any payment that is a **condition of entry**, or merely any detriment. Under the broader tests, holding-as-entry is likely
  consideration.
- **The "no purchase necessary" route is not available to us.** Sweepstakes are lawful in some places because anyone may enter
  free by an alternative method with equal odds. **There is no way to enter an etched draw without holding the coin**, and the
  contracts cannot add one: a ticket list that included non-holders would be the keeper's invention. Do not let anyone describe
  etched as a sweepstakes on that basis.

## 3. The modes that have no chance element

- **`ProRata` (runes):** every holder receives in proportion to balance. No chance. It looks like a distribution to token
  holders, which raises section 5's questions instead, not a lottery's.
- **`Named`:** the creator names a recipient. No chance; it is a gift by the creator's decision.
- **`Hold` / `Never`:** nothing is awarded.

So the lottery question attaches to **inscription draws only**. That matters for sequencing: a rune-first launch (which the venue
research already favours, because runes are the only asset with a fully documented purchase API) avoids the draw entirely.

## 4. Levers, and what each is worth

| Lever | What it does | Honest value |
|---|---|---|
| Launch with runes and `ProRata` only; list no inscription collections at first | removes chance from the product | **High.** The cleanest way to defer the lottery question until counsel has answered it. Costs nothing in code: the owner simply lists no inscription collection. |
| The creator, not the protocol, chooses the mode | moves the decision to run a draw to a third party | Low to moderate. The protocol built and offers the draw; "the user chose it" rarely moves a regulator. It may matter for who is the *promoter*. |
| Make `DrawPerWallet`/`DrawWeighted` unavailable by policy or in a future deployment | removes draws | High, and blunt. Inscriptions would then be `Named`, `Hold` or `Never` only. |
| Geo-blocking the website | hides the front end in listed countries | **Cosmetic.** The contracts are reachable by anyone. It shows intent; it does not change who can take part. Counsel may still want it. |
| Terms that make jurisdiction the user's responsibility | allocates risk on paper | Low. Useful, not a defence to operating an unlicensed lottery. |
| No marketing of the draw as a way to win something | reduces the promotional character | Moderate. The site today describes the mechanism plainly and promises nothing. Keep it that way: no "win", no countdown hype, no jackpots. |
| Verifiable randomness, public snapshot, no house edge | fairness | Helps with consumer-protection and fraud concerns. **Does not** make an unlicensed lottery lawful: fair lotteries are still lotteries. |

## 5. The other questions a fee-funded distribution raises

- **Securities-type analysis.** Holders of a coin receive assets bought with fees generated by others' trading, through the
  ongoing efforts of identifiable parties (the keeper, the purchaser, the attestor, the owner). Counsel should test the coin, and
  the arrangement as a whole, against the investment-contract tests of each target jurisdiction (in the United States, the
  familiar four-part test: an investment of money, a common enterprise, an expectation of profit, from the efforts of others),
  and against collective-investment-scheme rules elsewhere. Facts that cut our way: nobody is promised anything, the site makes
  no price or return claim, the creator receives nothing from fees, holders have **no claim** on a vault. Facts that cut the
  other way: a holder plainly hopes to receive something of value because of others' work; the protocol takes 20%.
- **The 20% treasury share.** Who receives it, in what capacity, and whether taking a cut of fees that fund prizes or
  distributions changes the protocol's role (promoter, operator, manager).
- **Money transmission / custody.** The purchaser receives ETH from a vault, bridges it and buys an asset for others. Is that
  transmitting money or acting as a custodian or agent for holders? The vault's Bitcoin wallet holds assets that are, in
  substance, for others.
- **Consumer protection and advertising.** Unaudited code, irreversible losses, a vault with no withdraw function: what must be
  disclosed, how prominently, and whether the draft Risk Disclosure is adequate. Unfair-terms rules where users are consumers.
- **Sanctions and AML.** The contracts are permissionless. What screening, if any, is expected of the site, the purchaser
  (who deals with a marketplace and a bridge), and the entity.
- **Tax.** For the entity (the treasury's receipts), for winners and recipients, and any withholding or reporting duty on the
  party that delivers an asset.
- **Intellectual property.** Coin creators choose names and images; the site displays them. Inscriptions are third parties'
  works. Marketplace terms (UniSat §3.1 requires written permission for commercial use; Magic Eden's terms bar commercial use and
  manipulative trading) are contractual exposure, already being pursued with the venues.
- **Entity and liability.** What entity operates the site and holds the roles, where, and whether the individuals behind the
  keys are personally exposed. The site's terms leave the entity and the governing law blank until this is decided.

## 6. Places that are typically strict about chance-based prizes

Stated as general knowledge, unverified, and no substitute for advice: the United States (lotteries are state-run; private
lotteries are generally unlawful under state law, and the three-element test above is the usual starting point; the states
differ on consideration), the United Kingdom (prize competitions and free draws are exempt only within defined limits;
otherwise a lottery needs a licence), most of the European Union (national gambling monopolies or licensing), Australia and
Canada (permit regimes and criminal-law prohibitions respectively), Singapore, the UAE and mainland China (broad prohibitions).
Many other countries restrict games of chance for religious or public-policy reasons.

## 7. Questions to put to counsel, in order

1. In our target jurisdictions, is holding a purchased token "consideration" for a random draw funded by that token's trading fees?
2. If yes (or arguably yes), can inscription draws be offered anywhere without a licence, and is any licence realistically obtainable?
3. Does launching with **runes and pro-rata only** avoid the gambling analysis entirely, and what does it trigger instead?
4. Is the coin, or the arrangement, a security or a collective investment scheme where we intend to operate or to be reachable?
5. What is the right entity and jurisdiction, and who should hold the owner, keeper, purchaser and attestor roles?
6. Is the purchaser a money transmitter, custodian or agent, and what follows?
7. Are the draft Terms, Risk Disclosure and Privacy pages adequate, and what must be added before any deployment?
8. Is geo-blocking or wallet screening expected of us, given that the contracts are permissionless?
9. What are the tax consequences for the treasury and for recipients, and do we have reporting duties?

## 8. What we would do while waiting for answers

Stage and test everything (it costs nothing and involves no real prize). Do not list an inscription collection on a real
deployment. If a real test is run, run it with a rune and `ProRata`, or with `Named`.
