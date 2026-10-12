# C5-I-1 from other people's real Relay transfers (read-only, 2026-09-28)

**Question** (C5-I-1; C7-L-1 in `fixes-round7.md` §6; B8-I-2 in `review8/README.md`; runbook `docs/v7/golive-runbook.md` §2 and
`docs/v7/phase1-nft.md` §5.1). What shape do a real Relay fill and a real Relay **refund** take between Robinhood Chain (4663) and
Ethereum (1), in both directions? Would the canister's payout judge (`ethflow::judge_payout`) credit them as built?

**Method.** No wallet was used and nothing was signed, sent or spent. Only keyless public sources were read:

- Relay's `GET /requests/v2`, filtered by `originChainId`, `destinationChainId` and `status=refund|success`;
- `eth_getTransactionByHash`, `eth_getTransactionReceipt`, `eth_getLogs` and `eth_getCode` on `rpc.mainnet.chain.robinhood.com`,
  `rpc.mevblocker.io` and `ethereum-rpc.publicnode.com`.

Each pair was run through the repo's own verifier, `ops/rehearsal/relay/verify.mjs`, which calls `autobuyer/canister-rs/examples/relay_verify.rs`
(the canister's `decode_execute`, `call_request_hash`, `call_executed` and `judge_payout`). The worktree was at `origin/main` `f8db5ef`.
`--us` is the address Relay paid (the depositor or the recipient). It stands in for the canister's address.

## Verdict

| Question | Answer (measured) |
| --- | --- |
| Shape of a fill, each way | A **plain ETH transfer** from one of Relay's solver hot wallets (many different EOAs) to the recipient. Its whole input is 32 bytes: **the order id** (the `bytes32` of the deposit's `depositNative(depositor, orderId)`). No logs. It is not Relay's request id |
| Shape of a refund, each way | **The same shape, on the origin chain.** A plain ETH transfer from a solver hot wallet to the depositor, input = the order id, value = the deposit less the refund's gas. 4663 → Ethereum orders are refunded on **4663**. Ethereum → 4663 orders are refunded on **Ethereum** |
| Depository `execute` / `RelayCallExecuted` for ETH payouts? | **None seen.** In 1,004 executes on 4663 (the last 300,000 blocks) and 833 on Ethereum (the last 2,000 blocks), every execute is sent by Relay's solver `0xf70da978…dbef`. Each either withdraws the solver's own ETH or pays ERC-20 token contracts or Relay's token helper `0x5fc5360d…` with value 0. None pays ETH to a third party |
| A refund as a withdrawal the depositor must claim? | **Never.** Every one of 540 refunded requests (4663-origin and Ethereum → 4663, 2026-09-27T19:09Z to 2026-09-28T21:55Z) has exactly one out-transaction sent by Relay. `refundFailReason` is `N/A` on all of them |
| Batched refunds (B8-I-2)? | **None.** No out-transaction is shared by two requests among the 540. No depository transaction carries more than one `RelayCallExecuted` |
| The one other shape | When the payee **has code** (an EIP-7702 delegation, `0xef0100…`), Relay pays through its router `0xb92fe925…4fff4f` (selector `0xcd6e13f7`, 484+ bytes) instead of a plain transfer. This applies to fills and refunds alike. The judge refuses it. The correlation was 100 % in the 65 checked: every router payout was to an address with code, and every plain payout was to an address without code |
| **Would the canister credit real refunds as built?** | **Yes**, with `eth_credit_payout(<buy>, <chain>, <tx>, 0)` (shape 1, no amount stated). This holds as long as the canister's address has no code, which is true of a threshold-ECDSA EOA that never signs a 7702 authorization. 19 of 20 real plain refunds and 9/9 real plain fills were CREDITED. The 20th refund was refused only by the existing at-least-half bound (a 0.00018 ETH order). The 6 router payouts were REFUSED, as expected |
| Code change needed? | **None for the judge.** Two notes follow, neither blocking (see "Notes") |
| Is the operator's live test still needed? | **Not for the shape question.** The shapes are measured on 35 real payouts in both directions: 29 plain (9 fills, 20 refunds) and 6 router. The remaining value of a live run is only the end-to-end check of the canister's *own* address and deposit bytes (see "What a live test would still add") |

## Fills (the verifier on real transactions)

All 4663 → Ethereum fills are paid on Ethereum and all Ethereum → 4663 fills on 4663. Every deposit is `depositNative` into
`0x4cd00e387622c35bddb9b4c962c136462338bc31`. "deposit (68 B)" means the verifier read the deposit itself: the order id, the value,
and the block after which a payout must be mined. "flags" means the deposit carried an 8-byte suffix (`0xdf24fbe5865d8597`, the
relay.link site's tag, which Relay's `/quote` API does not add), so the verifier refuses it as a deposit. The order id and value were
passed with `--order-id` and `--order-value` taken from that deposit.

| Direction | Request | Deposit | Fill | Sender (solver) | Shape | Verifier | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 4663→1 | `0x1790632306540292218eada215559b802398bf796631b0129a6b9107159c1a00` | `0x6e17e8d9b5ac1985a3acd7d6d0aad3c236b2167aeb0bff97ec8604c9c51d4d1b` | `0x7f6244d1898b0c90f04735ec84e893a7ba7eaa120f84a172b3b4b51ae2d11b72` | `0xca7ded7e…4589` | plain, input = order id | deposit (68 B) | CREDITED 0.148323654 ETH, unstated |
| 4663→1 | `0x17906325802b84bc90b34698e4bdc4c459031b02485d670d1c2c33cb22357813` | `0xb82b780123597f928f17757b3888e345890d5347487dbee9fd353926278afafb` | `0x3db0d4b97731718c1a046583c5b2c4983297c752f6dc53f6656d612cafddb937` | `0xa5a5491b…a142` | plain, input = order id | deposit (68 B) | CREDITED 0.001584810 ETH, unstated |
| 4663→1 | `0x1790632508b7da8c5dbd8ab64b029f4102adecf10ad44ae90e90ca398bb40c36` | `0x6e06b9925a55c00feaabf4d6d378727e2876754b262a8e63b7928a9b199a90d1` | `0xd06041b9fe92521c15df26b8e9fd8227977975c9d53d3839bc52f2cfff88a0c3` | `0x18dd3c14…8e25` | plain, input = order id | deposit (68 B) | CREDITED 0.000868142 ETH, unstated |
| 4663→1 | `0x17906324667404d0a22d3b56b344ecd121aa611a116d2f20c71eb4abdc4f0e24` | `0xa2c8ceaa2f7c32cdf5ad059fa6a3291eb9eda3023be81821deff55130ab98eb5` | `0x2bb0a3c92764f700f0bcb6a7a729c9440280b5ea3b680e6d6fcb8e0ac5ce6512` | `0x56c26202…8a8b` | plain, input = order id | deposit (68 B) | CREDITED 0.000974711 ETH, unstated |
| 4663→1 | `0x1790632403291847853fa5450bdbab76bb3e1803dad56b81447193a844a3adf9` | `0x29715187a534499909169aad3d4708b15f8d0ef727e4dc164e3e028646dea850` | `0x83676a9b18345032bf6588f1c9b5cbf57fc83353ceca5bbddf545c4fa100b8b1` | `0xada5bb90…f7cb` | plain, input = order id | flags | CREDITED 0.002991595 ETH, unstated |
| 4663→1 | `0x1790632473f70924f9105cfab01e11e4b82914f15e64e0105aecdb41b4f2b9e9` | `0xb4e7f78d9485dfc3edb48d708c4ca478b0fdcc1aa2dd2032ce2ee8130b58741f` | `0x6df5f396932918d46160f1423c52fec6301dadea24e93b54c955a5c18a0a23ca` | `0x18dd3c14…8e25` | **router** (payee has 7702 code) | deposit (68 B) | REFUSED (neither a payment to us nor a depository execute) |
| 1→4663 | `0x17906325607adf2da357f3aafffa4edcc912940cbf8c031cdf4efce0a6a08f63` | `0x590f60417300764eff439cfec02a16c610a431d9db67b3403e06f7a5c44478f5` | `0x907bbaa03dd64079a230a2a13660caf46aea8bbd396c4a178ad53226a60220eb` | `0x51180844…6f72` | plain, input = order id | deposit (68 B) | CREDITED 0.054204994 ETH, unstated |
| 1→4663 | `0x179063251616cd3bdb838e270bb68d4bfb644476b64576ebfc3829b76465a701` | `0x102ac2760b4ba3268a92947a7a08f97f35d68f3ceea399eade17d7598d860f5e` | `0x4be75be48f1a455c4ae1c4589da75b7b5a9f6b08c43af4ecd86c96470077ea94` | `0xd460b418…181b` | plain, input = order id | deposit (68 B) | CREDITED 0.000461468 ETH, unstated |
| 1→4663 | `0x1790632463ca9e5e9db06cc83e85c627489b5b4237d96dff7fdba86b792be4e0` | `0x5ccb3d936e21c68b2907e52c1f0397912cd954263fb069bcbe62c0c9c6017158` | `0x0b71c2d37687f80b2a1ab982de5b26c1b906e8b48781bff70060a429441a6b04` | `0x81adf99f…c13c` | plain, input = order id | deposit (68 B) | CREDITED 0.001990557 ETH, unstated |
| 1→4663 | `0x17906324631d21e4519191b2028999ebd97e097aa8da4f4f716e65042bb78efc` | `0x44bd36cc1677574ad90fb6e771d655e51bd8cf76a322ac5d76c4041cc1aca28a` | `0xff3d9071541bb41ac8ad7ef465f14278acfacb7317f2bb3c289ae4ae19df6a5d` | `0xabb2acd3…cd10` | plain, input = order id (recipient ≠ depositor) | deposit (68 B) | CREDITED 0.140007697 ETH, unstated |
| 1→4663 | `0x1790632518b7f5f70a6900070eaac21db066b28ad06dbe8b4765e8708d26a4b5` | `0x3dc246da1424948ff7934341bbac4d72245d8495553f3dd6b2d8ee6371c60737` | `0xa9c89bb945546c427a323b9d734fc1aa867d8da1675d1c270a29e5f23e781449` | `0x69374f82…7ef5` | **router** (payee has 7702 code) | flags | REFUSED (as above) |

Plain fills: **9/9 CREDITED**, with no amount stated. Router fills: 2/2 REFUSED.

## Refunds (the key question)

Each deposit is `depositNative` of ETH into the depository. Each refund is on the **origin** chain. "Reason" is Relay's `failReason`.

| Order → refunded on | Reason | Request | Deposit | Refund | Sender (solver) | Shape | Verifier | Verdict |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 4663→1, on 4663 | ORDER_ALREADY_FILLED | `0x179062684993324a8e949ef58cb7f7351f38f0aabfc754a23f45357bc48be011` | `0xcc50d2c15735418718a896e041a3e53af0c9e0c2f16456aea4b77fdbe2ffc5d9` | `0xf2fe033e7c3b0889bc89c412295ced781387db8d7dafedbf9580c351cd6e88e2` | `0xabb2acd3…cd10` | plain, input = order id | flags; trimmed* | CREDITED 12075107282958603 wei, unstated |
| 4663→1, on 4663 | ORDER_ALREADY_FILLED | `0x1790626835c6c652494872d7ca74ccc7acd20c968a06e4bd9603a64af287a91a` | `0xcd4691f1cb7177f941341f783c843d47166f4c7fc60a59e140bd3da82df1305a` | `0x90628c0fd84586f24ae3bbd3a3972201979fb9c53caca69132a615d021410910` | `0xd460b418…181b` | plain, input = order id | flags; trimmed* | CREDITED 12695017763968463 wei, unstated |
| 4663→1, on 4663 | ORDER_ALREADY_FILLED | `0x1790626589e2b62812fd3023106b761585ae018416f6a8413e61f7f581345eed` | `0x207826c12c2e693a70fa2347dfa2359ba88b847c57496ad72451167e8c709a9f` | `0xff51af92f51457be48538cf47da36caed29741df5f07c03c4f6291c8a8244f92` | `0xcc4bc788…7a1a` | plain, input = order id | flags; trimmed* | CREDITED 15474644566595287 wei, unstated |
| 4663→1, on 4663 | ORDER_ALREADY_FILLED | `0x179062641560953627aaea77c796898da9dd20b6bd45f0523917da5672ad1c7e` | `0x10e2ee118d6f6045cd3150188d86e8c9120bd2cfd7a9624e2dfd245e88144e9f` | `0xf0f7596a2f82e9ecc169b100226b51f1c0a4be6ecb88cd745ef1fd55d1907f1e` | `0x698dd6c0…aaed` | plain, input = order id | flags; trimmed* | CREDITED 10780843430226420 wei, unstated |
| 4663→1, on 4663 | ORDER_ALREADY_FILLED | `0x17906263690430ec8a03728ee8760c70d69ebf5d0b5ab00260bf6a48cd7ee3ae` | `0x4c9a6ad8db22ecc3728631f63dbeda5ba0d3b9d4c5e62f367319b39d02dd2595` | `0x0bd58239def341eb4084ef37aa4710d2e52d8976415dfa68ba8f26a2acdcef65` | `0x69374f82…7ef5` | plain, input = order id | flags | CREDITED 8747349345013175 wei, unstated |
| 4663→1, on 4663 | ORDER_IS_CANCELLED | `0x17906252094087e9809368d71bfed4013965700b1d337dee6d052821364c9576` | `0x62edac66766ce48d1db5e4936e27cc3010ec90f909846747faf129db00dad934` | `0x4c4e9260c99d7b71239425c840a820e13f75b6adcfb8087b46b49ce5a21d289a` | `0x9bc462bc…1083` | plain, input = order id | flags | CREDITED 10518345002100188 wei, unstated |
| 4663→1, on 4663 | ORDER_ALREADY_FILLED | `0x1790625177caf87ceea8902c094433a3d6aa6b3dc096765910a2c6c1b4f0a89a` | `0xc822082cb15af6ef6ae4c73bf895e0b5429051558f7e5097ed0858229804ed87` | `0x1101f8bf14ae7b640b41b3bfb5d35ac16c8680ea0c7b80f91159c2ca87e9f9dd` | `0x43810450…2646` | plain, input = order id | flags | CREDITED 10292495964566537 wei, unstated |
| 4663→1, on 4663 | ORDER_ALREADY_FILLED | `0x179062022741bd4a4abe7c4b5f6244129358f203db3f74f65892a063b8b09bf4` | `0x5670df94ae505b916b41de70364aaca096c2691274234dfe2a2816401f0a690f` | `0x560f9764f1ca4a0f06ff135408d98cdfba9faec6a5362663280c060a2b0c9a00` | `0x56c26202…8a8b` | plain, input = order id | flags | CREDITED 11189628800263020 wei, unstated |
| 4663→1, on 4663 | ORDER_ALREADY_FILLED | `0x179061985243ec84ea48f68334ef350b609f40cb57dae7fd1896e10b8d780b4e` | `0x952c499f309f9929f0fd084f5733e89db036adc297fd65470481cc8d908e0c10` | `0xae8060372c3629d927cf0da1ba1ee69fee44e2c1d8e9deeb80cf65125b1d9121` | `0x60859328…fdab` | plain, input = order id | flags | CREDITED 13669926840870955 wei, unstated |
| 4663→1, on 4663 | MINT_QUANTITY_EXCEEDS_MAX_SUPPLY | `0x1790619710cb86a24155493c573d12709d0192be5e521ee1070ade581df24627` | `0xad690725809d0b80f3f5d83d6283b49d9e52640c7d4bdca6a1e856482239177a` | `0x14b8c07be0150a0f04b3f4ba95f5deb13f0e398a006d4178edb91f6a2eaa27ba` | `0x0ec4e45f…038f` | plain, input = order id | flags | CREDITED 4370358286558308 wei, unstated |
| 4663→1, on 4663 | ORDER_ALREADY_FILLED | `0x1790628585b51fcb1db6206800449803394a6c895f10c40ac01b7a4684956736` | `0x52b47c4b6c526867472ae83b0a92fbe210e12b12a213df4db1768f7c577d5fd7` | `0x45154c1824550277468b3ab1c7a2fed9e44292b95b6de5e816d31a5ae10bcd41` | `0x9bc462bc…1083` | **router** (depositor has 7702 code) | flags | REFUSED |
| 4663→1, on 4663 | ORDER_ALREADY_FILLED | `0x1790626364934ba32bab4e2e450a9325895e43cb7805b8438cbef6bd156ec795` | `0x38ba2b3a89e3adaa9894cd02c5d14b6b256b487b9b4498c7cb748b3e8af6453b` | `0x8ab1a860162b95786aa7fb7ee8ac1a0330350201847a1d190d20532cc02109ed` | `0x43810450…2646` | **router** (depositor has 7702 code) | flags | REFUSED |
| 1→4663, on 1 | SLIPPAGE | `0x179063226023b487749b00f0f2ac0312a389022388a26e3de4a895c4f7fb3e8c` | `0x871b2ac726166bc330ac22d9c8624824eb42c3b84fa3e21642f2de2928e776b2` | `0x544d741cbf9464f307ac6e2ce46fc1e33b53dd709173b0a112987268369ece93` | `0xa67d7eb4…3fbb` | plain, input = order id | deposit (68 B) | CREDITED 1416142322337774 wei, unstated |
| 1→4663, on 1 | EXECUTION_REVERTED | `0x17906318366b72ad55522a7f04bb59196d46e969f275f4ecdca61f8de0f38768` | `0x178d521bf78510d2ef671b81c64cd27be88818d41f1d9a86c5929805fbb395bb` | `0x4b257542cef30eef872f57878deb058e74657bcebece2d75e3ba5d0fb32c0220` | `0x370a7e2d…12cf` | plain, input = order id | deposit (68 B) | CREDITED 3319312025232570 wei, unstated |
| 1→4663, on 1 | ORDER_ALREADY_FILLED | `0x17906322451297180a804be6cb94ae48e273c9b0733202d3cb23904b24845a4b` | `0xeb86d3a3cc0a4eb29ff26b9e0178c011ae3673683fe5bba940f0ba9697fe483a` | `0xea357b68be0d70f5ba32e47cff5cd90f9f21b6d760b0c8a580bb30a4b044941d` | `0xa67d7eb4…3fbb` | plain, input = order id | flags; trimmed* | CREDITED 5382971857774734 wei, unstated |
| 1→4663, on 1 | ORDER_ALREADY_FILLED | `0x179062894372a16aea3f778569779a464ee7d225130b27003f85cc231a216657` | `0x214f89bdda29326b92904805f3976a30a9c9d874105801f01fc5cb7b5702cba9` | `0xcd08636e2ecb3dcb86a9afd5162a91ccb097f47113cfd02565976019d3d4c072` | `0x18dd3c14…8e25` | plain, input = order id | flags; trimmed* | CREDITED 2692451257316857 wei, unstated |
| 1→4663, on 1 | ORDER_EXPIRED | `0x1790628899235f829d263a8ac984be6d96bed9f9dbac799c2c9e676730cd16e7` | `0x1bba9bc5873ad7e37516cc8bb18803debc5910c4c2aeb52a34ed04e5c5ddeac4` | `0xaf7be35a9e723df9cd569c3d0dfdbbad5557e8e2e869514956d5c88fa63e5b62` | `0x331d9a04…2466` | plain, input = order id | flags; trimmed* | CREDITED 1039474438422028 wei, unstated |
| 1→4663, on 1 | ORDER_ALREADY_FILLED | `0x179062887406b0fd2e738939832a4278a9335575f0d41e4b140047ae2cdd4d6b` | `0x8386a72942e58155f8c1d99fa3659de9794a833d486167855de6b2edc215cf9e` | `0x8a04d4f18f0ee3169b80e77287358c54fe3f2ce4d1c2d56440d5ba8c8d130043` | `0xa67d7eb4…3fbb` | plain, input = order id | flags | CREDITED 2512495544655658 wei, unstated |
| 1→4663, on 1 | ORDER_EXPIRED | `0x17906288508e7b125b7f394f64b7aa1e675873026938dd1de2608bc800a4e2a7` | `0x14e7a3847d7298bb3a6a75a941401da88badc835a8e467e94c23501135d58e6b` | `0xf2eac2903e7c40fd2a25452142ef49b68732bc48df48740fb1199158810555b2` | `0xa67d7eb4…3fbb` | plain, input = order id | flags | CREDITED 1038974107696485 wei, unstated |
| 1→4663, on 1 | ORDER_ALREADY_FILLED | `0x1790623901950b3a68a4d904c7a213028d3b6deb25929f3b26ef6bb8bccb21b2` | `0x9c84fd4fd32386f88f99e4650ee52f8a42ee394cfdc96025b279428a889289b2` | `0x40cfc590ca076e519267fc70e47f91055fb469687e1a7b35b05d7427c3808228` | `0x56c26202…8a8b` | plain, input = order id | flags | CREDITED 6557491500877157 wei, unstated |
| 1→4663, on 1 | ORDER_ALREADY_FILLED | `0x1790623731841a8d0f93b6698f34083590c7e4da8d35e2deb07b6898a50d3335` | `0xdcda2e318c3ef27352af9d8df345908d25365ee8597c91154b0fac005f626cd3` | `0x202fb1b3cf3924b1acaf721d0612e19d6317f5dfd4778906945ca01bbd1c9de0` | `0x370a7e2d…12cf` | plain, input = order id | flags | CREDITED 749245315537946 wei (72 % of 1046773233556046), unstated |
| 1→4663, on 1 | ORDER_EXPIRED | `0x17906217929135dee5f9e2c11ca9122c3bc39a3241ae1bf477a9ba11786ac2d2` | `0x3b53729e4665923b3b59f1cfff1e33d0b6a11ab99504c8ed3f1568c1431b6475` | `0x46a699fe8ef3430d1b2517185faa49a0ac286b99b72ec65060e73526b58be80e` | `0x56c26202…8a8b` | plain, input = order id | flags | CREDITED 21540846105949373 wei, unstated |
| 1→4663, on 1 | ORDER_ALREADY_FILLED | `0x1790622548921ee69f2f8cc41224294b12f5fdda87df9aa9fcbb880362d24f1f` | `0x5228ef18b05c554431d8f9d0cab4c0371bd5f76509f97fcfb778e0b25f8e8d2b` | `0x1890c09a555fd0056dca5cb1bcd3442445cae9eb715dcf6b8da540283fb6cfa0` | `0x18dd3c14…8e25` | plain, input = order id | flags | **REFUSED: 77671729836796 wei is under half the order's 182123072555596** |
| 1→4663, on 1 | SLIPPAGE | `0x1790632482120bfb8b70a6cab5ac9ff41678e61e12b146edce231e5f05b1d147` | `0x75bc34b591fbd74af11e31f9d6609f8661eb05f54733217e7e57766a7894ad2c` | `0x070bb1ff28517e28f9be9cad8137e009aa217e45612740be369561566ab3484e` | `0x370a7e2d…12cf` | **router** (depositor has 7702 code) | deposit (68 B) | REFUSED |
| 1→4663, on 1 | ORDER_ALREADY_FILLED | `0x1790631545e9148de06cce973ff45308e04c0870f554982224863d95432b5d23` | `0xd80881ef197cd52ef6e3a059af1d814806baa5d240d1dd9f1b16894aa023b04f` | `0x79d0fa76123487499d03b1cf3984e6317acf51ab25e06d685eb98e112deff209` | `0x370a7e2d…12cf` | **router** (depositor has 7702 code) | flags | REFUSED |

\* **trimmed:** in these rows the verifier was also run with the deposit read as a deposit. This is a SYNTHETIC copy of the real
deposit's JSON with only its 8-byte site tag removed, so `relay_verify` accepts `depositNative`. That exercises the "mined after the
deposit's block" rule with the deposit's real block. The result was the same CREDITED line in every row. The 7 runs:
`0xcc50d2c1`, `0xcd4691f1`, `0x207826c1`, `0x10e2ee11` (4663) and `0xeb86d3a3`, `0x214f89bd`, `0x1bba9bc5` (Ethereum).

Plain refunds: **19/20 CREDITED with no amount stated** (10 refunded on 4663, 9 on Ethereum). The 20th was refused only by the existing
at-least-half bound: a 0.00018 ETH Ethereum deposit refunded 0.000078 ETH after gas. Router refunds: 4/4 REFUSED.

## What the survey says beyond the 20

- **540 refunded requests** (all currencies) were read from `/requests/v2`: 300 with origin 4663 (2026-09-28T13:39Z to 21:50Z) and 240
  Ethereum → 4663 (2026-09-27T19:09Z to 2026-09-28T21:55Z). Every one was refunded **on its origin chain**, by exactly one
  out-transaction. None has an unrefunded state (`refundFailReason` `N/A` throughout). No out-transaction is shared by two requests.
- Of the ETH `depositNative` refunds among them:
  - origin 4663: 101 plain to the depositor, 59 via the router;
  - Ethereum → 4663: 88 plain to the depositor, 34 via the router;
  - to the depository, or requiring a claim: **0**.
- **Refund cost** (deposit − refund, ETH refunds):
  - on 4663: median 7.6 × 10¹¹ wei, i.e. ~0.00000076 ETH (the 3.3 × 10¹⁵ outlier is a swap-leg refund);
  - on Ethereum: median 5.7 × 10¹³ wei, ~0.000057 ETH, up to ~0.0003 ETH at the moment's gas.
  - The judge's at-least-half bound therefore refuses only Ethereum refunds of orders under roughly 0.0006 ETH. It never bites on 4663.
- **Depository executes** (topic0 `0x4be10945…c5ea`, the canister's `call_executed` encoding):
  - on 4663, blocks 74,890,328–75,190,328: 1,004 executes in 1,004 transactions, all sent by `0xf70da978…dbef`. 502 pay that
    sender itself (solver withdrawals). 502 pay `0x5fc5360d…d168`, Relay's token-flow helper, which is also the `to` of 91 token refunds.
  - on Ethereum, blocks 26,077,278–26,079,278: 833 executes, all sent by `0xf70da978…dbef`, to token contracts (USDC 401, USDT 140, …)
    or to the sender.
  - On neither chain does any transaction carry more than one record, and none pays ETH to a third party. The two
    executes the reviews cited (`0x86eee61f…`, `0x89635e98…`) are of this kind: payee self-withdrawals.

## Answers to the audit items

- **C5-I-1 / C7-L-1 (shape of a real fill and refund):**
  - Both are **shape 1**: a plain transfer to the payee whose input is the order id.
  - `judge_payout` credits them **as built** with no stated amount. It checks that the order id is in the input, that the payout was
    mined after the deposit's block, and that the value is between half and all of the order's value.
  - A refund of an outbound (4663 → Ethereum) order arrives **on 4663**. A refund of a back order arrives **on Ethereum**.
  - Shape 2 (depository `execute` + `RelayCallExecuted`, amount stated) was **not observed for ETH payouts at all**. The code path is
    harmless but, on this evidence, unused for ETH.
- **A refund as a claimable withdrawal (runbook §5.1 warning):** not observed in 540 refunds or in 1,837 depository executes. The
  "never reaches the canister" case did not occur.
- **B8-I-2 (batched refunds):** not observed. Each refund is its own transaction for one request, so crediting per transaction hash is
  enough for everything seen. The note can stay a note.

## Notes (no judge change required)

1. **The canister's address must stay code-free.** Relay pays through its router whenever the payee has code, including an EIP-7702
   delegation. The judge (rightly) cannot credit that: the ETH arrives through an internal call, and the transaction's `to` is the
   router. The canister's threshold-ECDSA address has no code unless it signs a 7702 authorization, which it never does. If that ever
   changes, late payouts become uncreditable. Worth one line in `phase1-nft.md` §5.1.
2. **`relay_verify` accepts only a 68-byte deposit.** Deposits made through the relay.link site carry an 8-byte suffix
   (`0xdf24fbe5865d8597`). The canister's own deposits come from `/quote` data, which is exactly 68 bytes, and `relayeth.rs` /
   `chains.rs` require that. So only the example tool is affected, and `--order-id` / `--order-value` work around it. Optional: let
   the example ignore a trailing suffix.
3. **`ORDER_ALREADY_FILLED` refunds name an order that was also filled.** Most refunds seen are a second deposit for an order that
   another deposit already filled, so a fill (on the destination) and a refund (on the origin) both carry the same order id. The
   canister makes one deposit per order, so this should not arise for it. If it ever did, both transactions would pass the judge's
   naming rule. The amount bound (≤ the order's value, each) and per-transaction `credited` still hold.
4. **Relay's keyless `/requests/v2`** answered every query here (limit ≤ 20; `limit=50` with `status=refund` returns HTTP 500). It
   still retires on 2026-11-24, as the runbook says.

## What a live test would still add

The shape question is answered from real transactions, so the operator does not need to spend ETH to learn it. A live run would
additionally confirm, end to end:

- that the canister's own `/quote`-built 68-byte deposit gets the same plain-transfer fill;
- the refund on 4663 for the canister's address specifically.

Nothing observed suggests either would differ. The payee-shape rule (code / no code) is the only variable Relay applied. A forced
refund also needs a Relay API key (`debug-force-refund`). If the operator skips the live run, record this file's rows under C5-I-1 in
`phase1-nft.md` §5.1 instead.

## Reproduce

```sh
git worktree add ../Rook-relay-evidence -b relay-evidence origin/main   # f8db5ef
cd ../Rook-relay-evidence
CARGO_TARGET_DIR=/tmp/rv-target OUT=/tmp/rv node ops/rehearsal/relay/verify.mjs \
  --us 0xa3747c66dec0cb61f7bf2b6d22ddd289934b64ca \
  --payout 0xf2fe033e7c3b0889bc89c412295ced781387db8d7dafedbf9580c351cd6e88e2 --payout-chain 4663 \
  --order-id 0xa367e3b17ac81014dbf12ee9e46e1728b9e051e6257548cabe365b8fc569775c --order-value 12075824390118603 \
  --request-id 0x179062684993324a8e949ef58cb7f7351f38f0aabfc754a23f45357bc48be011 --relay-status
# -> shape: from 0xabb2acd3…cd10 to 0xa3747c66…64ca, value 12075107282958603 wei, selector a367e3b1, 32 input bytes
#    a PLAIN TRANSFER to --us …; input carries the order id: true
#    Relay /requests/v2 …: status refund; in [[4663,"0xcc50d2c1…"]]; out [[4663,"0xf2fe033e…"]]; REFUND
#    judge_payout, no amount stated: CREDITED 12075107282958603 wei (0.012075107 ETH)
#    VERDICT: creditable as built (eth_credit_payout(<buy>, <chain>, <tx>, 0))
```

Survey queries: `GET https://api.relay.link/requests/v2?originChainId=4663&status=refund&limit=20` (and
`originChainId=1&destinationChainId=4663`, `status=success`), paged with `continuation`.

## Separately: DFINITY's free cycles faucet (checked 2026-09-28)

- **It still exists.** `https://faucet.dfinity.org` redirects to the canister `anv4y-qiaaa-aaaal-qaqxq-cai.ic0.app` and serves the
  "DFINITY Cycles Faucet" app.
- **Amount:** the app's own text says **"Get 10T Free Cycles!"**, i.e. 10 trillion cycles, about 10 XDR or ≈ $13.
- **Gate:** a coupon is required. The app says "Coupons are only distributed at DFINITY events like hackathons, workshops, or to
  selected groups like grantees". People without one have asked for one on the DFINITY developer Discord or forum; a 2024 forum
  thread shows DFINITY staff still sending codes by Discord DM.
- The bundle also contains an older Twitter-sign-in eligibility path ("account … at least 90 days old", "not used to claim before").
  The landing page now leads with the coupon.

Steps:

1. Get a coupon code: a DFINITY event, a grant, or a request on the DFINITY dev Discord.
2. Open https://faucet.dfinity.org, enter the code ("Coupon Code", NEXT STEP), and follow its "Setup SDK" page (install `dfx`).
3. Run `dfx cycles --network ic redeem-faucet-coupon <code>`. The cycles go to your identity's cycles-ledger account.
4. Run `dfx cycles --network ic balance` and expect `10.000 TC (trillion cycles)`.
5. Then use `dfx canister create/deploy --network ic`, or `dfx cycles top-up <canister> <amount> --network ic`, from that balance.

10T is enough to create and run a small canister for a while. It does not replace the cycles plan's funding.
