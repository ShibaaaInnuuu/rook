DRAFT. To be sent by the operator, not by anything in this repository.

To: legal@unisat.io   (the only contact address in UniSat's own Developer Service Legal Disclaimer; copy a business or developer
contact if you have one)
Subject: Request for written permission for commercial API use (Section 3.1) - etched

Hello UniSat team,

I'm building etched (https://etched.meme), a token launchpad on Robinhood Chain. Trading fees from each token flow into an on-chain vault, and the vault's
funds are used to buy Bitcoin inscriptions and runes, which are then distributed to that token's holders. I would like to use the
UniSat Open API, including the marketplace endpoints, to make those purchases, and I'm writing to ask for your written permission
for commercial use under Section 3.1 of your Developer Service Legal Disclaimer.

What we would do with the API:
- Read collection statistics and rune market listings to determine current floor prices.
- Buy listed inscriptions and runes through the documented purchase flow (create_bid_prepare, create_bid, confirm_bid, and the
  batch versions for runes), paying the asking price and all UniSat marketplace fees like any other buyer.
- Read rune balances and collection membership so that anyone can verify what the vault holds.

How it works, and why I think it fits your guidance:
- Every purchase is declared publicly on-chain before it happens, with a maximum price, and there is a one-hour public waiting
  period before funds are released. Your API spec asks that state-changing marketplace calls have explicit confirmation first;
  this is that step, and it is public.
- We only buy existing listings at their listed price. We do not place misleading bids, wash trade, or make any claims about
  price impact.
- We do not resell, redistribute or republish UniSat data. Prices are used only to cap what the vault may spend.
- The vault's Bitcoin address is published, so every purchase can be checked against the chain.
- We will respect rate limits and move to a paid plan as volume requires.

Expected volume is small at first: a handful of purchases per day across a few collections and runes, growing with usage. I'm
happy to share the contract code, the design document and the trust model, and to adjust anything you would like done
differently, including attribution or a link to UniSat wherever purchased assets are shown.

Could you confirm in writing whether this use is permitted, and whether the marketplace endpoints are available on the standard
plans or need separate approval?

Thank you,
[Your name]
[Email]
[Project link, once there is one]
