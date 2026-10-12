DRAFT. To be sent by the operator, not by anything in this repository.

Where it goes: Magic Eden issues Ordinals API keys through a form, not by email. The forms are linked from
docs.magiceden.io/reference/ordinals-api-keys (one for US applicants, one for non-US). Paste the text below into the form's
free-text field describing your use, and keep a copy of what you submitted and the date. If the form has no room for the
questions, submit the description and send the questions to the contact the form's confirmation gives you.

Scope: RUNES ONLY. Magic Eden documents a complete programmatic purchase path for runes (an unsigned PSBT for buying a rune, then
submitting the signed PSBT). It documents no endpoint for buying an inscription, so this application does not ask about
inscriptions, and etched will not drive their website with software (their Terms of Service, Sections 4(c) and 9(f), forbid it).

---

Project: etched (https://etched.meme)

What it is: etched is a token launchpad on Robinhood Chain. A share of each token's trading fees flows into an on-chain vault. When
a vault paired with a rune holds enough, its funds are used to buy that rune on Bitcoin, and the runes are distributed pro rata
to the token's holders. We would like an API key for the Ordinals API in order to do the rune purchases through your documented
rune endpoints.

What we would call:
- Rune market info and the orders for a specific rune, to read the current unit price.
- "Get an unsigned PSBT for buying a rune" and "Submit a signed PSBT for buying a rune", to buy existing sell orders at their
  listed price, paying your marketplace fees like any other buyer. We sign the PSBT ourselves; you never hold our keys or funds.
- Rune balances by wallet address, so that anyone can check what the vault's published Bitcoin address holds.

How it behaves:
- Every purchase is declared publicly on-chain first, with a maximum price, and waits one hour in public before any funds are
  released. Purchases are small and infrequent: a handful a day at the start.
- We only take existing sell orders at their asking price. We place no bids intended to mislead, do no wash trading, and make no
  claims anywhere about moving a rune's price.
- We do not resell, republish or aggregate your data; a price is used only to cap what a vault may spend. We will attribute Magic
  Eden and link to the rune's page on Magic Eden wherever a purchased rune is shown.
- We will stay inside the rate limit you give us and add a delay between calls, as your documentation asks.

Four questions we would be grateful to have answered in writing, because we do not want to operate on our own reading of them:

1. Terms of Service, Section 9(h) prohibits using the Services "for any commercial purpose or for the benefit of any third
   party". A vault that buys on behalf of a token's holders is arguably both. Is buying runes through the API for this purpose
   permitted?
2. Section 9(p) prohibits manipulative trading, including unduly influencing a market price. We buy only existing listings at
   their asking price, in declared, capped amounts. Is there anything about that pattern you would want done differently?
3. Section 9(q) prohibits using the Services for fundraising for a protocol, including buying assets that entitle owners to
   financial rewards. The runes we would buy are ordinary runes, and holding them entitles nobody to anything. We read 9(q) as
   being about the assets purchased, not about where a buyer's funds came from. Is that reading correct?
4. Section 4(c) points to API licensing terms on docs.magiceden.io. The "Magic Eden API Terms of Service" link in the
   documentation's sidebar currently resolves to a Google Doc that has been deleted, so we have not been able to read them. Could
   you point us to the current API terms?

If the answer to any of these is no, we would rather know now and not build on your API at all.

[Your name]
[Email]
[Country, for the correct form]
[Project link, once there is one]
