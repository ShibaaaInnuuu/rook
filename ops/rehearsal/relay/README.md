# C5-I-1: one real Relay transfer each way, and a refund (operator runbook)

Pre-launch task 3 of `docs/audit/2026-09-27-nft/review7/README.md` (C5-I-1; C7-L-1 in `review8/README.md`). The canister's late-payout
judge (`ethflow::judge_payout`) credits a written-off Relay order's payout in two shapes only:

1. **a plain transfer to our address whose input names the order** (the order id or Relay's request id in its calldata), at its value;
2. **the depository's allocator-signed `execute(CallRequest, signature)`** whose own `RelayCallExecuted` record (topic0
   `0x4be10945…c5ea`, id = the EIP-712 struct hash of the decoded request) pays our address, named by the operator stating the amount
   from Relay's records (`eth_credit_payout(<buy>, <chain>, <tx>, <wei>)`).

Nobody has yet measured which shape a real Relay fill and a real Relay **refund** take. This runbook measures it with the operator's own
wallet and a small amount, and the verifier runs the real transactions through the canister's own judge.

**Nothing here signs or sends.** The operator signs the deposits in their own wallet. The scripts only read: Relay's `POST /quote` and
`GET /requests`, and `eth_getTransactionByHash` / `eth_getTransactionReceipt`.

| File | What |
| --- | --- |
| `quote.ts` | A read-only quote each way, with the checks the canister makes of a deposit, and exactly what the wallet will be asked to sign |
| `verify.mjs` | Reads the deposit and the fill or refund, writes them as JSON, and runs `relay_verify` on them |
| `../../../autobuyer/canister-rs/examples/relay_verify.rs` | The verdict from the canister's own code: `relayconv::decode_execute`, `call_request_hash`, `call_executed` and `ethflow::judge_payout`. It is an example binary and is never part of `buyer.wasm` |

## Amounts (measured 2026-09-28, read-only quotes)

Relay refuses anything under roughly 0.0001 ETH ("Swap output amount is too small to cover fees"). A few thousand wei, as §5.1 suggested,
is refused. The 4663 → Ethereum leg pays a roughly fixed ≈ 0.000086 ETH (relayer gas on Ethereum):

| In | 4663 → Ethereum out (impact) | Ethereum → 4663 out (impact) |
| --- | --- | --- |
| 0.0001 ETH | 0.0000196 (−80 %) | 0.0000917 (−8.3 %) |
| 0.0005 ETH | 0.000416 (−17 %) | 0.000491 (−1.7 %) |
| 0.001 ETH | 0.000914 (−8.6 %) | 0.000991 (−0.9 %) |
| 0.002 ETH | 0.00191 (−4.5 %) | 0.00199 (−0.5 %) |

**Recommended: 0.002 ETH out (4663 → Ethereum) and 0.001 ETH back.** In all, including a forced refund, the operator spends under
0.01 ETH, most of it Relay's fees.

## The example quote (read-only, 2026-09-28, the public address `0x…dEaD`)

```
cd ops && node --import tsx rehearsal/relay/quote.ts --user 0x000000000000000000000000000000000000dEaD

=== robinhood (4663) → ethereum (1), 1000000000000000 wei (0.001000000 ETH), user = recipient = refundTo = 0x…dEaD
steps: deposit/transaction (1 item(s), requestId 0x179061184777df817818b946dc52324db1715bac0b228e6da2ceb37c10b64ca9)
WHAT YOUR WALLET SIGNS (one transaction; compare it field by field in the wallet before signing):
  chainId 4663  from 0x…dEaD
  to      0x4cd00e387622c35bddb9b4c962c136462338bc31
  value   1000000000000000 wei (0.001000000 ETH)
  data    0x49290c1c…dead6bb2558a57297491c2ced075cfe7784e89401d476798c9f3c40a70fae56f26f5
order id: calldata 0x6bb2558a…26f5; recomputed hashStruct(orderData) 0x6bb2558a…26f5; quote says 0x6bb2558a…26f5
solver 0xf70da97812cb96acdf810712aa562db8dfa3dbef (solverChainId base)
output on ethereum: pays 0x…dEaD expected 917910863266620 wei, minimum 891016074972909 wei; deadline 2026-10-05T16:10:47Z
refund on robinhood: to 0x…dEaD, minimum 0 wei, deadline 2026-10-05T16:10:47Z
refund on ethereum: to 0x…dEaD, minimum 0 wei, deadline 2026-10-05T16:10:47Z
fees in the order: 0
Relay's details: out 0.000917911 ETH (min 0.000891016 ETH); time estimate 4 s; total impact -8.21 %
checks: ok (the depository, depositNative(you, orderId), the order id recomputed, the output and refunds to you)

=== ethereum (1) → robinhood (4663), 0.001 ETH …
  chainId 1, to 0x4cd00e387622c35bddb9b4c962c136462338bc31, value 0.001 ETH, data 0x49290c1c…dead b3f590c1…76b6
order id recomputed = calldata = quote; output on robinhood 0.000991157 ETH (min 0.000962116); refunds on ethereum and robinhood to 0x…dEaD
checks: ok
Nothing was signed or sent.
```

Both directions use the **same depository address** and the same `depositNative(address depositor, bytes32 orderId)` (selector
`0x49290c1c`). The solver named in the orders is `0xf70da978…dbef`.

## The operator's steps

Use a wallet you control, with its **public** address as `$ME` (it is the depositor, the recipient and the refund address). Keep about
0.005 ETH on 4663 and 0.003 ETH on Ethereum.

**1. 4663 → Ethereum (a fill).**

```
cd ops && node --import tsx rehearsal/relay/quote.ts --user $ME --amount-wei 2000000000000000 --dir out --json /tmp/rook-golive
```

Only continue if it prints `checks: ok`. Then, in your own wallet, sign and send exactly one transaction on chain 4663: to
`0x4cd00e387622c35bddb9b4c962c136462338bc31`, the value and the `data` printed. The easiest way is to run the same quote from
relay.link's own UI with the same wallet and amount, then compare the wallet's prompt with the printed `to`, `value` and the
`0x49290c1c` selector. Record:

- the deposit transaction hash (4663);
- `requestId` and the order id (printed);
- the fill transaction on Ethereum (Relay's UI, or `/requests/v2?hash=<deposit>`).

**2. Ethereum → 4663 (a fill).** The same with `--dir back --amount-wei 1000000000000000`, signed on chain 1.

**3. A refund, each way.** Relay's documented test hook (docs.relay.link → API core concepts → Refunds) is the referrer
`debug-force-refund`. The solver then **refunds on the origin chain to `refundTo`** instead of filling. The quote needs an API key: with
the hook and no key, the keyless `/quote` answers `401 "Please provide an api key"` (measured 2026-09-28). With your Relay key in the
environment (never in a file; the script sends it as `x-api-key` and never prints it):

```
RELAY_API_KEY=… node --import tsx rehearsal/relay/quote.ts --user $ME --amount-wei 2000000000000000 --dir out --force-refund
RELAY_API_KEY=… node --import tsx rehearsal/relay/quote.ts --user $ME --amount-wei 1000000000000000 --dir back --force-refund
```

Sign each deposit as in step 1, **from the forced quote's own `data`** (its order id differs). Relay pays no refund that cannot cover
its gas, so keep 0.002 ETH on the 4663 → Ethereum leg. If the hook is not available, the one other refund you can cause yourself is a
failed fill, for example by letting the quote's deposit arrive long after the quote. That is not guaranteed; record whatever happens.

**4. Verify each pair (read-only).** Use `--payout` for the fill or refund and `--deposit` for your deposit:

```
# a fill of step 1: the deposit on 4663, the fill on Ethereum
node ops/rehearsal/relay/verify.mjs --us $ME --deposit 0x<deposit> --deposit-chain 4663 --payout 0x<fill> --payout-chain 1 \
     --request-id 0x<requestId> --relay-status
# a refund of step 3 (out): the deposit on 4663, the refund on 4663 (the origin chain)
node ops/rehearsal/relay/verify.mjs --us $ME --deposit 0x<deposit> --deposit-chain 4663 --payout 0x<refund> --payout-chain 4663 \
     --request-id 0x<requestId> --relay-status
```

`--relay-status` reads `/requests/v2` (keyless). **v2 has been throttled since 2026-09-01 and retires 2026-11-24.** With
`RELAY_API_KEY` set, the script reads `/requests/v3` instead. The runbook's release rule and every late credit rely on Relay's records,
so the operator needs a v3 key before 2026-11-24.

## Reading the verdict (C5-I-1)

`relay_verify` prints the fill's shape (from, to, value, selector, and whether the input carries the order id or the request id), each
`RelayCallExecuted` record (its emitter, whether its id equals the recomputed struct hash, whom it pays), and `judge_payout`'s answer
with no amount stated and with the record's amount stated. Its exit code is 0 if the payout is creditable, 3 if not, and 2 on bad input.

| What the verifier shows | Meaning for the canister |
| --- | --- |
| `a PLAIN TRANSFER to --us` and `input carries … request id: true` → `creditable as built (…, 0)` | Shape 1. A late payout of this shape is credited by `eth_credit_payout(<buy>, <chain>, <tx>, 0)` |
| `a PLAIN TRANSFER to --us` but the input names neither id → `NOT creditable` | The solver pays with no calldata: shape 1 cannot name the order. **The judge must change (C7-L-1) before such a late payout can be credited.** Record it |
| `execute(CallRequest, signature)`, a record from the depository, id `EQUALS` the struct hash, paying `--us`, `creditable … only with the amount stated` | Shape 2. Credited only by the operator stating the amount from Relay's records (`/requests/v3`) |
| `SENT BY --us: a withdrawal the payee had to claim itself` | The refund is a depository withdrawal the payee sends itself. The canister signs no such call, so **such a refund can never reach the canister**. §5.1 says to treat such an order as lost to Relay's records until a reviewed signer path exists |
| anything else | The judge must follow it before any order can be credited late (§5.1) |

Record in `docs/v7/phase1-nft.md` §5.1, under C5-I-1, for each of the four transactions: the hashes, the chain, the shape, and the verdict
line. This matters most for the **4663 refund of an outbound order**: that is where a written-off buy's ETH comes back.

## Checked against real transactions (read-only, 2026-09-28)

These are the two real depository `execute`s the reviews cite. Neither is a refund to someone else. Each is a payee withdrawing its own
balance from the depository:

```
node ops/rehearsal/relay/verify.mjs --us 0x008c8607387c8f5540287070c08c3e9acd4e8442 \
     --payout 0x86eee61f75068592cacb14dafcc6ddda09be653738daaea3a7e0e298e8a6d84d --payout-chain 1 --relay-status
payout … on chain 1 (block 26072289, success)
shape: from 0x008c8607…8442 to 0x4cd00e38…bc31, value 0, selector 2d9fb478, 516 input bytes
  a call of the depository's execute(CallRequest, signature) (selector 0x2d9fb478): Relay's allocator-signed payout
  SENT BY --us: a withdrawal the payee had to claim itself; …
  request: 1 call(s), nonce 0x8b63e873…01ed, …; EIP-712 struct hash 0x84334077101689714321be1e7c88eeaf265d4d8b7c5b91cbc197b6704cbe6659
  RelayCallExecuted (the depository): id 0x84334077…6659 EQUALS the recomputed struct hash; pays 0x008c8607…8442 991250000000000 wei  <- --us
judge_payout …: REFUSED: the transaction is ours (…)
VERDICT: NOT creditable as built
Relay /requests/v2 for this hash: 0 request(s)

node ops/rehearsal/relay/verify.mjs --us 0xf70da97812cb96acdf810712aa562db8dfa3dbef \
     --payout 0x89635e988b9ccca1f03ffbb30e1b1d8549e73394d2ad30e462f99b2658786a38 --payout-chain 4663
payout … on chain 4663 (block 74888060, success)
shape: from 0xf70da978…dbef to 0x4cd00e38…bc31, execute, 516 input bytes
  EIP-712 struct hash 0x14ba700e42fd4d3c8057169b8f5f41d925b6bf0eb81cff7cdcc7e81e708f8157
  RelayCallExecuted (the depository): id 0x14ba700e…8157 EQUALS the recomputed struct hash; pays 0xf70da978…dbef 7.151468212 ETH  <- --us
VERDICT: NOT creditable as built (sent by the payee)
```

On both chains the canister's decoder recomputes the log's id from the input. `0xf70da978…dbef` is the **solver** named in every quote
above: the 4663 `execute` is Relay's solver withdrawing 7.15 ETH of its own depository balance. It is not a user's refund.

The credit path was also run on a copy of the 4663 transaction with its `from` changed to a stranger (a SYNTHETIC input, only to exercise
the code). It gives `no amount stated: REFUSED (does not name order)`, `stating 7151468211844133347: CREDITED`, and a wrong stated amount:
`REFUSED (none of this request's execution records … is the 5 wei stated)`. That is shape 2 as built.

## Build note

`verify.mjs` runs `cargo run --example relay_verify` in `autobuyer/canister-rs` (toolchain 1.98.1, `~/.cargo/bin`). Set
`CARGO_TARGET_DIR` to keep it out of the canister's own target directory. The example uses only the library's public API. It adds
nothing to `buyer.wasm`: the wasm is built from the library, not the examples, so its hash is unchanged.

## The page (2026-10-09)

The same four transfers from a browser, with your own wallet signing:

```sh
cd ops && RELAY_API_KEY=… npx tsx rehearsal/relay/server.ts        # then open http://localhost:8766
```

`server.ts` listens on 127.0.0.1 only and keeps the key (Relay's API refuses the key from a browser page). It quotes with the runner's own
body and judges each quote with the canister's own rules (`ops/src/ethnft.ts` `preJudgeEth` at 3 %) before the page offers to send; after
the payout it runs `verify.mjs` on the real transactions. **Measured 2026-10-09:** Ethereum → 4663 at 0.001 ETH is refused by that 3 % rule
(Relay's minimum is 3.95 % under); 0.0015 ETH sits on the edge and 0.002 ETH is 2.49 % under, so both Ethereum-side rows use 0.002 ETH.
Relay routes the refunds of the burn address `0x…dEaD` to its own `0xf3d6…691e`, which the rules refuse: test quotes need a real address.
