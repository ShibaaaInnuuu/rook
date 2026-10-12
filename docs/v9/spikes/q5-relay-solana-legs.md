# Spike Q5: Relay legs from a Solana origin

**Date:** 2026-10-09. **Answers:** [solana-migration.md](../solana-migration.md) §2.8 and §8, question Q5. **Background:**
[ref-solana-research-2026-10.md](../ref-solana-research-2026-10.md) §4.

**How it was run.** Everything was read-only. Relay quotes came from `POST https://api.relay.link/quote/v2` with **no API key**, which
still answered on 2026-10-09. Chain data came from the public RPC `https://api.mainnet-beta.solana.com` (`getSignaturesForAddress`,
`getTransaction`, `getAccountInfo`), plus two keyless `GET /requests/v2` lookups. Nothing was deposited, signed or sent. The addresses
in the quotes are throwaways: an ed25519 public key and a P2TR address made from a random key that was never stored. The one
exception is a known-active taproot address taken from the latest mainnet block.

**Proof:** [`ops/rehearsal/relay-sol/`](../../../ops/rehearsal/relay-sol/) is a standalone crate holding the saved quotes and
mainnet transactions as fixtures. `cargo test` is green: 17 tests, covering every saved quote and every saved transaction.

## 0. The answers

| Question | Answer | |
|---|---|---|
| Is SOL→BTC quotable? | **Yes.** It worked for 1, 5, 20 and 200 SOL, to the throwaway taproot and to the active one, at every retry over ~25 min. The ceiling is between **250 and 300 SOL per order** (≈ 0.33 BTC, ≈ $27k). Above it Relay answers `INSUFFICIENT_FUNDS "Insufficient funds"`, which is the solver's BTC inventory running out. | measured |
| What was the earlier `INSUFFICIENT_FUNDS "No UTXOs available to spend"`? | **The input, not solver liquidity.** Relay gives this answer when the **origin is Bitcoin** and the `user` address has no UTXOs, because Relay builds the PSBT from the user's own coins. BTC→SOL with a funded `user` quotes fine, and so does the deposit-address flow. Amount and recipient had nothing to do with it. The SOL→BTC failure recorded on 2026-10-09 in ref §4 could not be reproduced with the `relay.ts` request shape (`user` = the Solana sender, `recipient` = the taproot address, `refundTo` = the Solana sender). | measured |
| How does the canister bind a Solana-origin order? | **The same way as on 4663.** The deposit is one `deposit_native(amount: u64, id: [u8; 32])` instruction to Relay's depository program, and `id` **is the same EIP-712 `hashStruct(Order)`** of `protocol.v2.orderData`. Solana addresses enter the hash as their 32 raw bytes. `relayconv::order_id` needs exactly one change: `"solana"` added to `vm_of`, encoded as base58 → 32 bytes. | proved on 9/9 Solana-origin quotes, and on the 5 others |
| What shows a payout or refund on Solana? | **Not the depository.** On Solana, Relay's depository is only an input escrow. Refunds and fills are **plain transfers from Relay's solver wallet, with a memo `0x<order id>`** (a fill carries a second memo, the request id). `execute_transfer` is the solver taking settled deposits out of the vault and names no order. The canister reads (a) its own `deposit_native` and the `DepositEvent` it emits, and (b) its balance rise, with the memo naming the order. | real mainnet examples, §5 |
| New risk not visible on EVM | **The Solana depository program is upgradeable**, and its upgrade authority is the same key as its owner, `6Bz8…aMWD`. The 4663 depository is not a proxy. The judge must pin the program's deploy slot and authority (rule S3). | read from chain |

## 1. Quotes (2026-10-09, about 17:06 UTC, keyless)

The request shape is `ops/src/relay.ts` `quoteBody` / `ops/src/ethnft.ts` `ethQuoteBody`: `{user, originChainId, destinationChainId,
originCurrency, destinationCurrency, recipient, tradeType: "EXACT_INPUT", amount, refundTo}`. Solana `792703809` / `111…1`. Bitcoin
`8253038` / `bc1qqq…mql8k8`. Ethereum `1` / `0x0…0`. Fees are Relay's `fees.*.amountUsd`. "Time" is `details.timeEstimate` in
seconds. Every successful quote has `minimumAmount / expectedAmount = 0.9800` and a deadline at the quote time + 7 days (604,800 s)
on the output and on every refund.

| Fixture | In | Out | Impact | Relayer fee (gas + service) | Origin gas | Time | Answered in |
|---|---|---|---|---|---|---|---|
| `sol-btc-1` | 1 SOL ($109.65) | 0.00130881 BTC | −1.53 % | $1.18 (0.93 + 0.25) | $0.0008 | 652 | 1.4 s |
| `sol-btc-5` | 5 SOL ($548.23) | 0.00658506 BTC | −0.91 % | $2.50 (1.33 + 1.17) | $0.0008 | 652 | 0.8 s |
| `sol-btc-5-active` | 5 SOL | 0.00658422 BTC | −0.92 % | $2.50 | $0.0008 | 652 | 0.8 s |
| `sol-btc-20` | 20 SOL ($2,192.90) | 0.02636867 BTC | −0.80 % | $7.56 (2.95 + 4.61) | $0.0008 | 652 | 1.8 s |
| `sol-btc-20-active` | 20 SOL | 0.0263663 BTC | −0.81 % | $7.56 | $0.0008 | 652 | 0.6 s |
| `sol-btc-200` | 200 SOL ($21,923) | 0.26379155 BTC | −0.74 % | $68.25 (22.33 + 45.91) | $0.0007 | 652 | 0.3 s |
| `sol-btc-norefund` (no `refundTo`) | 5 SOL | 0.00658571 BTC | −0.88 % | $2.50 | | 652 | 0.2 s |
| `sol-eth-1` | 1 SOL | 0.043783 ETH | −0.69 % | $0.35 (0.10 + 0.25) | $0.0008 | 3 | 0.5 s |
| `sol-eth-20` | 20 SOL | 0.876567 ETH | −0.58 % | $4.71 (0.10 + 4.61) | $0.0008 | 3 | 0.2 s |
| `eth-sol-0.5` | 0.5 ETH ($1,243.56) | 11.2944 SOL | −0.42 % | $1.76 | $0.13 | 1 | 0.3 s |
| `btc-sol-active-user` (PSBT) | 0.01 BTC ($824.93) | 7.4592 SOL | −0.88 % | $4.87 | $0.94 | 1 | 1.0 s |
| `btc-sol-depositaddr` (`useDepositAddress`) | 0.01 BTC | 7.4384 SOL | −1.16 % | $6.95 | $4.12 | 1 | 0.3 s |
| `eth-btc-0.5` (control) | 0.5 ETH | 0.01497315 BTC | −0.67 % | $3.52 | $0.13 | 652 | 0.7 s |
| `rh-btc-0.05` (control, 4663) | 0.05 ETH | 0.001484 BTC | −1.55 % | $1.45 | $0.002 | 652 | 0.6 s |

**Errors, saved in `fixtures/errors/`:**

| Request | Answer | Meaning |
|---|---|---|
| BTC→SOL, `user` = a fresh taproot with no coins | 400 `INSUFFICIENT_FUNDS` "No UTXOs available to spend" | The **origin** user has no UTXOs for Relay to put in the PSBT |
| SOL→BTC 300 / 400 / 500 / 1,000 / 1,500 / 2,000 SOL | 400 `INSUFFICIENT_FUNDS` "Insufficient funds" | Solver BTC inventory. 250 SOL quoted (0.3293 BTC) |
| SOL→BTC with the taproot address as `user`/`refundTo` | 400 `INVALID_ADDRESS` "Invalid refundTo … for chain 792703809" | The input was mixed up |
| BTC→SOL for 50 BTC | 400 `INSUFFICIENT_LIQUIDITY` | Size |
| SOL→ETH to `0x4e59…956c` (the CREATE2 deployer, a contract) | 400 `DESTINATION_TX_FAILED` | The payee reverts on a plain transfer |
| BTC→SOL `user` = `bc1qar0s…5mdq` | 400 `BLOCKED_WALLET_ADDRESS` | Relay screens addresses |

**Retries, from 17:06 to 17:31 UTC.** Eight rounds, about 4 min apart (`t0`, `t1`, then `t2`–`t7`). Each ran SOL→BTC 1/5/20 SOL
(throwaway and active taproot), ETH→BTC 0.5 and SOL→ETH 1. **All 44 to-BTC quotes in those rounds succeeded.** Output varied by under
0.5 %. Relay's own figures moved with the BTC network: `timeEstimate` fell from 652 s to 339 s and then to 206.5 s, and the relayer fee
for a 20-SOL order ranged from $6.08 to $8.53. The log is in §8.

## 2. The deposit instruction from a Solana origin

Every Solana-origin quote has exactly one step (`id: "deposit"`, `kind: "transaction"`) with one item. Its `data` is
`{instructions: [one], addressLookupTableAddresses: ["Hm9fUgcn7qwDaiNTFiGh6pNtVATgnaRcmK6Bbx6EMZfP"]}`. There is no compute-budget
instruction and no `chainId`, `from` or `value` field.

**The program.** `99vQwtBwYtrqqD9YSXbdum3KBdxPAVxYTaQ3cfnJSrN2` is the Anchor program `relay_depository` 0.1.0. Its IDL ships in
`@relay-protocol/settlement-sdk@0.0.146` (`messages/common/solana-vm/idls/RelayDepositoryIdl`). That IDL is newer than the docs page:
it adds `migrate_domain_separator`, `RelayDepository.domain_separator` and `TransferRequest.{domain, vault_address}`, and the live
state account (106 bytes) matches the IDL.

**Accounts.** They are in the program's order, and every quote has exactly these five:

| # | Account | Value | Signer | Writable |
|---|---|---|---|---|
| 0 | `relay_depository` | `Dodg2HifwU8rmaVVyMyUZDGTRbqAJTyVYxXPwcbNpBKc` = PDA(`"relay_depository"`) | no | no |
| 1 | `sender` | **ours** | **yes** | yes |
| 2 | `depositor` (the account credited) | **ours** | no | no |
| 3 | `vault` | `7uTT8Xi5RWXzy7h9XL244GRgEycDYDhLjr3ZyNdXi8pZ` = PDA(`"vault"`), bump 255 = the state's `vault_bump` | no | yes |
| 4 | `system_program` | `11111111111111111111111111111111` | no | no |

**Instruction data.** It is exactly **48 bytes**. Real example (`sol-btc-5`):

```
0d9e0ddf5fd51c06                                                    disc   = sha256("global:deposit_native")[0..8]
00f2052a01000000                                                    amount = u64 LE = 5_000_000_000 lamports
63b1643f6478d1006c4d5b7afaaa26ce13e9800d2b455ee1e84e9ddcae04ec33    id     = [u8; 32]
```

| Bytes | Field | Encoding | Must equal |
|---|---|---|---|
| 0..8 | discriminator | `0d 9e 0d df 5f d5 1c 06` | constant |
| 8..16 | `amount` | u64 little-endian, lamports | the lamports asked = `orderData.inputs[0].payment.amount` = `paymentDetails.amount` |
| 16..48 | `id` | 32 raw bytes | `hashStruct(Order)` of `protocol.v2.orderData` = `protocol.v2.orderId` |

Compared with the EVM deposit:

| | 4663 / Ethereum (`relayconv`) | Solana |
|---|---|---|
| Call | `depositNative(address depositor, bytes32 id)`, selector `0x49290c1c`, 68 bytes | `deposit_native(u64 amount, [u8;32] id)`, discriminator `0d9e0ddf5fd51c06`, 48 bytes |
| Amount | `tx.value` | in the data (`amount`), and moved by a System CPI from sender to vault |
| Depositor | in the calldata (word 1) | account #2 (and the payer is account #1, which signs) |
| Id | calldata word 2 | data bytes 16..48 |
| **What the id commits to** | **`hashStruct(Order)`** | **the same `hashStruct(Order)`**, with Solana addresses as 32 raw bytes |

**What `id` commits to.** The EIP-712 types are the ones in `relayconv.rs` (`Order`, `Input`, `InputPayment`, `InputRefund`,
`Output`, `OutputPayment`, `Fee`), unchanged. They are recomputed by `rook_relay_sol_spike::order_id`, which is a copy of
`relayconv::order_id` with one addition. The SDK's `encodeAddress(addr, "solana-vm")` is `bs58.decode(addr)`, which must come out
to 32 bytes, and it applies to:

- `inputs[0].payment.currency` = `111…1` (→ 32 zero bytes);
- a Solana `refunds[].recipient` (our address) and its `currency`;
- a Solana `output.payments[].recipient` / `currency` (ETH→SOL, BTC→SOL).

So, exactly as on 4663, the 32 bytes the canister would sign name the **destination recipient, the minimum out, the output chain and
currency, the deadline, every refund recipient (on both chains), the calls (none) and the fees (none)**. If any of these changes,
the hash changes.

The tests check this for every saved quote (`tests/binding.rs`):

- `every_solana_origin_quote_binds_its_instruction_id_to_the_order_hash`: for all **9** Solana-origin quotes, the instruction id =
  `order_id(orderData)` = `protocol.v2.orderId`; the amount = the asked lamports = the payment amount; the metas are exactly the
  table above.
- `the_evm_and_bitcoin_origin_quotes_hash_the_same_way_with_solana_addresses_in_them`: for ETH→SOL, the `depositNative` calldata id
  equals the hash with a Solana output. For BTC→SOL, the hash equals `orderId`, and the PSBT's OP_RETURN carries `"0x<order id>"` as
  text. ETH→BTC and 4663→BTC are the regression controls.
- Five refusal tests. Each takes a real quote and changes one thing: a changed recipient fails S6. A substituted order that is
  re-hashed end-to-end (orderId and instruction id rewritten too) fails the content rules S8/S9/S10/S11. A foreign depositor, vault,
  signer flag, program or extra data byte, a wrong amount, an extra instruction, a rotated allocator, an upgraded program, a stale
  hand-in and a replay are each refused, and each refusal names its rule.

**Building the transaction.** The canister does not need the quote's lookup table. Five static keys, plus a durable-nonce advance and
compute-budget instructions, come to about 450 bytes, well under 1,232. Recommendation: **the canister writes its own message from
[`deposit_native_metas`] and [`encode_deposit_native`]**, using only the judged `amount` and `id`. It ignores
`addressLookupTableAddresses`, and so never resolves a table it did not pin. This is the Solana form of today's "sign only what was
recomputed".

## 3. The Solana rules S1–S12 (the equivalents of `relayconv::judge` rules 1–12)

These are implemented in `judge_sol` (`src/lib.rs`), and the 9 real quotes pass them. "Same" means equal after Relay's
`encodeAddress` for that chain.

| Rule | 4663 today | Solana |
|---|---|---|
| **S1** shape | 1 step `deposit`/`transaction`, 1 item | Same, plus **exactly one instruction** in `data.instructions`. The lookup tables are never used (the canister builds its own message). |
| **S2** chain | `data.chainId` = 4663 | `paymentDetails.chainId == "solana"` and `inputs[0].payment.chainId == "solana"` (the item carries no chain id) |
| **S3** contract and trust | `to` = pinned depository; read `allocator()` = pinned | `programId` = `99vQ…rSN2`, the same as `paymentDetails.depository`. Read `RelayDepository` (`Dodg…pBKc`): `allocator` = `HuAj…eW6Z`. **New:** read the program's `ProgramData`: upgrade authority = `6Bz8…aMWD` and deploy slot = `386211280`. A change means review before converting, because unlike 4663 the program is upgradeable. |
| **S4** value | `value` = the asked wei | `amount` (data[8..16], LE) = the asked lamports = `paymentDetails.amount` |
| **S5** calldata | 68 bytes, selector, depositor = us | **48 bytes**, discriminator `0d9e0ddf5fd51c06`; metas **exactly** `[state r/o, us signer+w, us r/o, vault w, system r/o]`, flags included. The depositor (account #2) is the account Relay credits, so it must be us. |
| **S6** binding | calldata id = `order_id(orderData)` = `orderId` | data[16..48] = `order_id(orderData)` = `orderId` (`vm_of` gains `"solana"` → base58 → 32 bytes) |
| **S7** input | one input, `robinhood`, ETH, amount = value | One input, `solana`, currency `111…1`, amount = the deposit amount |
| **S8** refunds | every refund ours; one on the origin | Every refund ours: `solana` → our Solana address and `111…1`; the destination chain → our recipient there and its native currency. **There must be a `solana` refund.** (Relay adds the destination-chain refund to the recipient. Without `refundTo`, Relay still refunds on Solana to `user`.) |
| **S9** output | `bitcoin`, one payment to the float, native, no calls, no fees | Same, with the destination as a parameter: `bitcoin` → float taproot / `bc1qqq…mql8k8`; `ethereum` → our EVM address / `0x0…0`. |
| **S10** minimum | ≥ our rate less the fee bound | Same, with a SOL→BTC (or SOL→ETH) reference rate. Relay's minimum is 98.0 % of expected, so the default 300 bps bound passes with the market rate. |
| **S11** time | TTL 300 s; deadlines in [now+1 d, now+8 d], uint32 | Same. Relay's deadlines are quote + 7 d. (The BTC deposit-address flow has 180 d, but it is not a Solana origin.) |
| **S12** replay | the order id never accepted before | Same, over the `id` (unique per quote: `salt`) |

**ETH-leg note (outside Q5).** In SOL→ETH, both the Ethereum refund and the output carry `extraData = 0x…b92fe925dc43a0ecde6c8b1a2709c170ec4fff4f`
(Relay's router) even though the payee is a plain EOA. The hash covers it, so the binding holds. But
`docs/audit/2026-09-27-nft/relay-evidence-2026-09-28.md` treats router payouts as a separate shape, so this case needs a check if a
Solana→Ethereum leg is ever judged.

## 4. The depository on mainnet (read 2026-10-09, `fixtures/chain/`)

| Item | Value |
|---|---|
| Program | `99vQwtBwYtrqqD9YSXbdum3KBdxPAVxYTaQ3cfnJSrN2`, owner `BPFLoaderUpgradeab1e…`, **upgradeable** |
| ProgramData | `6y7C7Lfh1WRRbKohE2FQmFBD2asw3yMi17kStwEcAWWF`: deployed at slot **386,211,280**; **upgrade authority `6Bz8AWWipCDf7XCSqNuRXrEtKZM5M4Y56uP9ZRjSaMWD`** (a system-owned, 0-byte account. From the chain alone it cannot be told whether this is a single key or a multisig vault PDA.) |
| State `Dodg…pBKc` (106 B) | `owner` = `6Bz8…aMWD` (the same key); **`allocator` = `HuAjVESMnrR2wuthKHeAsJeDsfN2jKUEi7TCcZC7eW6Z`**; `vault_bump` 255; `domain_separator` = `01c7a22107be8f4973259032638387b984782556d68bd9b8e3fa23c48f473eb4` |
| Traffic | In the last 100 finalized signatures (about 30 s): 34 `deposit_token`, 4 `deposit_native`, 2 `execute_transfer`; 0 failed |

The tests check all of this: the PDAs are derived (`find_program_address`), the state and the `ProgramData` header are decoded, and
the Anchor discriminators are recomputed.

## 5. Payout and refund recognition on Solana

**The model, from the chain.** The Solana depository is an **input escrow**. A deposit moves SOL into the vault and emits
`DepositEvent {depositor, token: None, amount, id}`. The vault is emptied only by `execute_transfer`, and only with an Ed25519
signature by the allocator over `sha256(borsh(TransferRequest))`. In both `execute_transfer`s seen (`5Vpwp…` saved as a fixture, `2N696zGER98R…` in the same slot) the recipient of that release is
**Relay's solver itself** (`F7p3dFrjRTbtRp8FRF6qHLomXbKRBzpvBLjtQcfcgmNe`): it is settlement of deposits the solver has already
filled, and it **names no order id**. Payouts and refunds to users never come from the vault. **The solver pays them from its own
wallet, as ordinary transfers with memos.**

### 5.1 Our deposit landed (the Solana counterpart of "the deposit mined")

Read the finalized deposit transaction (`getTransaction`, commitment `finalized`) and check it with `deposit_in`:

- `meta.err == null`;
- one top-level instruction to `99vQ…` whose data is the 48 bytes the canister signed (discriminator, amount, the order id), with
  accounts `[Dodg…, us, us, 7uTT…, 111…]`;
- exactly one `Program data:` event **logged while `99vQ…` was on top of the invoke stack**, decoding as `DepositEvent` with
  depositor = us, `token = None`, the same amount and the same id;
- the vault's lamport delta = amount.

Real example: `5coWyuBNnXjhXkCLhusNKyWDFXjpcmybTG36ZszofdKPKRcra1conGvcCC8wWDXfCvbbZrJW9A9hpUgsgu7r3RvA` (slot 454,936,031), a deposit of
2,676,498,130 lamports. The vault delta is +2,676,498,130, and the event agrees with the instruction (test
`a_real_deposit_native_its_event_and_the_vault_credit_agree`).

### 5.2 A refund of a Solana-origin order (the counterpart of `relayconv::refunded`)

Relay's `/requests/v2?originChainId=792703809&status=refund` listed two SOL refunds on 2026-10-09 (`failReason: SLIPPAGE`). Both look
the same:

| | Case 1 | Case 2 |
|---|---|---|
| Order id (deposit `id`) | `0x5d99b8c7d0f14e90d0c2726d6b1272e9e455a08b39de70edb07c73b44f29758a` | `0xdb546ab6953596c1802834ea4415dc0f7f4dd429e7df71d00da34113c2638fd2` |
| Deposit tx | `3NFihMg28Rj6hWqDzmiG4YX94zWAK95r32BpXspxznACqmBqMK8DPPrCLce9RZW7TyJegevS4eDtVRDcNV5WmD3w` (slot 454,935,177), 105,905,434 lamports to the vault | `2dkBPFqAJYzSXYytFruj571ppztzm2CAAnsBw47WDg4n8iPhkj1T2UrADS1QvfzTiwBTTfGjhWedkKyfLfJW3Yc2` (454,935,047), 105,923,061 |
| Refund tx | `2jRVRQ5rntgBnxXngq9grbc7S13Tc3uUj8W861uh9cfkbww4fFdcz7fot6dEC7y9E3bFy3n4VmEC8BFJK7r91fmM` (454,935,182, **+1 s**) | `4wrsNEWwZf2vdufDQMqdUNywsDxhjXYQT3KdqSiALzanPjhMLcVEfjfqdwAE5Tq2gL41KShccTwZ2r8ovgntcbvy` (454,935,051, +1 s) |
| What it is | `System.transfer(F7p3… → depositor, 105,905,184)` + `Memo("0x5d99…758a")` | `System.transfer(F7p3… → depositor, 105,922,800)` + `Memo("0xdb54…8fd2")` |
| Back | the deposit − 250 lamports | the deposit − 261 lamports |

The depository program does not run in the refund, and the vault is not touched (test
`a_real_refund_is_a_solver_transfer_memoed_with_the_order_id`).

**What the canister reads at `finalized` to call a deposit refunded:** our address's lamports have risen by at least
`REFUND_SHARE_BPS` (50 %) of the deposit since the deposit's slot, which is today's balance rule unchanged. The trustworthy fact is
the balance; the memo is an unauthenticated label. **For attribution:** `getSignaturesForAddress(our address, until = the deposit
signature)` and `getTransaction` of each new one. The transaction that credits us carries `Memo == "0x" ‖ hex(order id)`
(`refund_of`). Because the memo is only a label, it should decide which conversion a credit belongs to, never whether we were paid.
Also accept an `execute_transfer` whose request names us as `recipient` with `token = None`, which `decode_execute_transfer` reads.
None was seen, but the allocator could release that way.

### 5.3 A payout **on** Solana (ETH→SOL, 4663→SOL, BTC→SOL; the counterpart of `home_payout_tx`)

The same shape applies: the solver transfers to the recipient, with **two memos, `0x<order id>` then `0x<request id>`**. Real example
`mURjRG1YRJNZEF9YRPo9VnjP6FvdsLCvDrvsnWaGS79j73AUCkdo3AumMYPMhqUJ1fDwsqfPELwrR6tA1Cx8T3A` is a 4663 → Solana token fill. Relay's
own record (`/requests/v2?id=0x17915657778570368daee545a0006e03d1bd163431c4d852f9a55ea60c4b04f1`) gives `orderId =
0x8a906e86…49f9` = the first memo (test `a_real_solver_fill_carries_order_and_request_memos`). The canister's read: a finalized
credit to its address of at least the order's `minimumAmount`, with the memo naming the order.

### 5.4 `execute_transfer` (for completeness; the canister never needs it)

Real example `5VpwpWBHjBZ4yiAs2kSihtNUFuFvKp49EQ785VPdMhCEiPc8etK6x2VuPkmygxnCxmt7sf1kKMA9kP1moBQW21rV`:

- Instruction 2 is `Ed25519SigVerify` by public key `HuAj…eW6Z` (the allocator) over 32 bytes = `sha256(borsh(TransferRequest))`.
- Instruction 3 is `execute_transfer` with data
  `e97ea0b8ebce1f77 ‖ domain[32] ‖ recipient[32] ‖ Option<mint> (1 + 32) ‖ amount u64 ‖ nonce u64 ‖ expiration i64 ‖ vault[32]`.
- The domain is the state's `domain_separator`, the vault is `7uTT…`, and recipient = executor = the solver.
- `TransferExecutedEvent {request, executor, id}`, where `id` is the `used_request` account, = PDA(`"used_request"`,
  `sha256(borsh(request))`). This is the replay guard. Test `a_real_execute_transfer_is_the_solver_settling_and_names_no_order`.

## 6. What changes in the trust, compared with 4663

1. **The upgradeable program.** The EVM depository is not a proxy, so its address pins its code. On Solana, `6Bz8…aMWD` can replace
   the program's code, and as owner it can also rotate the allocator. Rule S3 must read `ProgramData` (deploy slot and authority) on
   every judgement and stop on any change, just as it stops today on a new allocator. If the canister reads through the SOL RPC
   canister, this is one more `getAccountInfo` (45-byte data slice).
2. **Refunds and payouts come from a solver wallet, not from the escrow contract.** On 4663 a refund is likewise "our balance rose",
   so the canister's logic is unchanged. The difference is that the vault never pays us directly. Recognition depends on balance plus
   memo, and there is no contract event to read.
3. **The memo is unauthenticated.** Anyone can send 1 lamport with our order id as the memo. Requiring the credit to be ≥ 50 % of
   the deposit (refund) or ≥ the minimum (payout) makes spoofing cost at least the amount, and paying us cannot hurt us.
4. **The allocator signs with ed25519** (Solana), not secp256k1. The pinned value is the 32-byte public key above.

## 7. Recommended edits to the plan

- §2.8: replace "SOL→BTC and ETH→BTC quotes failed" with the §0 answers. Also, the API key turned out not to be required on
  2026-10-09 for `/quote/v2` or `/requests/v2`. `/requests/v3` does require it, and v2 retires on 2026-11-24, with rate limits
  already tightening (one 429 seen).
- §2.8: the payout recognition is "balance + memo `0x<order id>`" (§5), not "`execute_transfer` / allocator read".
- §4 canister: add `ProgramData` (deploy slot + upgrade authority) to the reads S3 makes; `relayconv::vm_of` gains `"solana"`.
- §6 security model: add the upgradeable depository (§6.1).
- Size: a SOL→BTC order is capped at about 250 SOL by solver inventory (2026-10-09). The frog-sized conversions (1–20 SOL) are far
  below the cap.

## 8. Retry log (SOL→BTC every ~4 min, keyless)

```
round time | pair | HTTP | ms | errorCode | message | out | impact % | relayer fee $ | timeEstimate s
t0 17:06 | sol-btc-1 | 200 | 0.00130888 | -1.52 | 1.175980 | 652
t0 17:06 | sol-btc-5 | 200 | 0.00658541 | -0.90 | 2.498342 | 652
t0 17:06 | sol-btc-20 | 200 | 0.02637534 | -0.78 | 7.557786 | 652
t1 17:06 | (the saved fixtures: every SOL→BTC 200; see §1)
t2 17:10:37 | sol-btc-1 | 200 | 1331ms |  |  | 0.00130398 | -1.86 | 1.483540 | 652
t2 17:10:37 | sol-btc-5 | 200 | 700ms |  |  | 0.00657426 | -1.05 | 2.938377 | 652
t2 17:10:37 | sol-btc-20 | 200 | 1209ms |  |  | 0.02633608 | -0.90 | 8.528762 | 652
t2 17:10:37 | sol-btc-20-active | 200 | 689ms |  |  | 0.02633616 | -0.90 | 8.528762 | 652
t2 17:10:37 | sol-eth-1 | 200 | 252ms |  |  | 0.043772231392556802 | -0.70 | 0.347217 | 4
t2 17:10:37 | eth-btc-0.5 | 200 | 630ms |  |  | 0.01495519 | -0.78 | 4.233550 | 652
t3 17:14:42 | sol-btc-1 | 200 | 940ms |  |  | 0.00130278 | -1.79 | 1.483021 | 652
t3 17:14:42 | sol-btc-5 | 200 | 718ms |  |  | 0.00656829 | -0.97 | 2.936143 | 652
t3 17:14:42 | sol-btc-20 | 200 | 431ms |  |  | 0.02631225 | -0.82 | 8.521502 | 652
t3 17:14:42 | sol-btc-20-active | 200 | 316ms |  |  | 0.02631225 | -0.82 | 8.521502 | 652
t3 17:14:42 | sol-eth-1 | 200 | 372ms |  |  | 0.043752149957211759 | -0.56 | 0.328354 | 3
t3 17:14:42 | eth-btc-0.5 | 200 | 170ms |  |  | 0.01495478 | -0.79 | 4.231920 | 652
t4 17:18:45 | sol-btc-1 | 200 | 1247ms |  |  | 0.00131402 | -1.12 | 0.711618 | 339
t4 17:18:45 | sol-btc-5 | 200 | 843ms |  |  | 0.00659135 | -0.80 | 1.830393 | 339
t4 17:18:45 | sol-btc-20 | 200 | 561ms |  |  | 0.02637668 | -0.76 | 6.275889 | 339
t4 17:18:45 | sol-btc-20-active | 200 | 485ms |  |  | 0.02637668 | -0.76 | 6.275889 | 339
t4 17:18:45 | sol-eth-1 | 200 | 213ms |  |  | 0.043789325355450627 | -0.59 | 0.314439 | 4
t4 17:18:45 | eth-btc-0.5 | 200 | 336ms |  |  | 0.01497966 | -0.70 | 2.452367 | 339
t5 17:22:48 | sol-btc-1 | 200 | 1042ms |  |  | 0.00131238 | -1.29 | 0.712241 | 206.5
t5 17:22:48 | sol-btc-5 | 200 | 606ms |  |  | 0.00658049 | -1.01 | 2.032367 | 206.5
t5 17:22:48 | sol-btc-20 | 200 | 664ms |  |  | 0.02633752 | -0.95 | 6.681683 | 206.5
t5 17:22:48 | sol-btc-20-active | 200 | 812ms |  |  | 0.02633771 | -0.95 | 6.681683 | 206.5
t5 17:22:48 | sol-eth-1 | 200 | 211ms |  |  | 0.043761332184001937 | -0.77 | 0.306034 | 3.5
t5 17:22:48 | eth-btc-0.5 | 200 | 602ms |  |  | 0.01496916 | -0.69 | 2.853755 | 206.5
t6 17:26:52 | sol-btc-1 | 200 | 1049ms |  |  | 0.00131665 | -1.10 | 0.712007 | 206.5
t6 17:26:52 | sol-btc-5 | 200 | 541ms |  |  | 0.00660431 | -0.78 | 1.832340 | 206.5
t6 17:26:52 | sol-btc-20 | 200 | 1609ms |  |  | 0.02643478 | -0.72 | 6.084666 | 206.5
t6 17:26:52 | sol-btc-20-active | 200 | 407ms |  |  | 0.02643865 | -0.70 | 6.085490 | 206.5
t6 17:26:52 | sol-eth-1 | 200 | 332ms |  |  | 0.043897556699089878 | -0.66 | 0.300104 | 4
t6 17:26:52 | eth-btc-0.5 | 200 | 716ms |  |  | 0.01499072 | -0.48 | 2.448623 | 206.5
t7 17:30:57 | sol-btc-1 | 200 | 2477ms |  |  | 0.00130947 | -1.45 | 1.019702 | 206.5
t7 17:30:57 | sol-btc-5 | 200 | 234ms |  |  | 0.00658172 | -0.93 | 2.271651 | 206.5
t7 17:30:57 | sol-btc-20 | 200 | 1413ms |  |  | 0.02635143 | -0.84 | 7.050677 | 206.5
t7 17:30:57 | sol-btc-20-active | 200 | 730ms |  |  | 0.02635117 | -0.84 | 7.050677 | 206.5
t7 17:30:57 | sol-eth-1 | 200 | 134ms |  |  | 0.043783296825738674 | -0.64 | 0.289252 | 4
t7 17:30:57 | eth-btc-0.5 | 200 | 818ms |  |  | 0.01497004 | -0.71 | 3.163516 | 206.5
```

## 9. Files

- `ops/rehearsal/relay-sol/src/lib.rs`: `order_id` (with `solana`), `decode/encode_deposit_native`, `deposit_native_metas`,
  `judge_sol` (S1–S12), PDA derivation, the transaction readers (`deposit_in`, `refund_of`, `memos`, `system_transfers_to`,
  `program_data`), and the `execute_transfer` / event / state / `ProgramData` decoders.
- `tests/binding.rs` (8 tests, all quotes) and `tests/chain.rs` (9 tests, all chain fixtures).
- `fixtures/quotes/`: 14 quotes, each with `.quote.json` (the raw answer) and `.request.json` (the body sent), plus `manifest.json`
  (fetch times).
- `fixtures/errors/`: 4 error answers with their requests.
- `fixtures/chain/`: the program, `ProgramData` header and state accounts; a deposit; two refund pairs, plus Relay's request records
  for them; an `execute_transfer`; a solver fill, plus its request record.
