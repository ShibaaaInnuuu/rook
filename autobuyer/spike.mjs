// Runs the three proofs against the LOCAL replica and prints what each showed. Nothing here touches a real key or a real chain write.
import * as btc from "@scure/btc-signer";
import { canisterKey, canisterSign, taprootOutputKey, verifyBip340, bytesToHex, hexToBytes, call } from "./lib.mjs";

const out = {};
// 1. the threshold key, its taproot address, and a key-path spend signed by the subnet and verified by noble + finalised by scure
const sec1 = canisterKey();                       // 33-byte compressed
const internal = sec1.slice(1);                   // x-only
const outputKey = taprootOutputKey(internal);
const net = btc.TEST_NETWORK;
const pay = btc.p2tr(internal, undefined, net);   // scure applies the same tweak; its address must match ours
out.address = pay.address;
out.outputKeyMatchesLibrary = bytesToHex(pay.tweakedPubkey) === bytesToHex(outputKey);

const tx = new btc.Transaction();
tx.addInput({ txid: hexToBytes("11".repeat(32)), index: 0, witnessUtxo: { script: pay.script, amount: 100_000n }, tapInternalKey: internal });
tx.addOutputAddress("tb1pqqqqp399et2xygdj5xreqhjjvcmzhxw4aywxecjdzew6hylgvsesf3hn0c", 60_000n, net);   // any valid testnet taproot recipient (the winner's, in the real flow)
tx.addOutputAddress(pay.address, 39_000n, net);                                                     // change back to us
const sighash = tx.preimageWitnessV1(0, [pay.script], btc.SigHash.DEFAULT, [100_000n]);
const sig = canisterSign(sighash);
out.signatureBytes = sig.length;
out.verifiesUnderTweakedKey = verifyBip340(sig, sighash, outputKey);
out.verifiesUnderUntweakedKey = verifyBip340(sig, sighash, internal);   // must be false: the subnet applied the tweak
tx.updateInput(0, { tapKeySig: sig });
tx.finalize();
out.finalisedTxBytes = tx.extract().length;
out.txid = tx.id;

// 2. an HTTPS outcall to a public Bitcoin TESTNET API (UniSat's testnet host answers 401/403 without a key — recorded, then a keyless one)
const grab = (s) => ({ status: Number((s.match(/status = ([\d_]+)/) || [])[1]?.replaceAll("_", "")), body: ((s.match(/body = "((?:[^"\\]|\\.)*)"/s) || [])[1] || "").slice(0, 160) });
try { out.unisatTestnet = grab(call("get", '("https://open-api-testnet.unisat.io/v1/indexer/blockchain/info", false)')); } catch (e) { out.unisatTestnet = { error: String(e.message).split("\n").slice(-3).join(" ").slice(0, 200) }; }
try { out.mempoolTestnet = grab(call("get", '("https://mempool.space/testnet4/api/blocks/tip/height", true)')); } catch (e) { out.mempoolTestnet = { error: String(e.message).split("\n").slice(-3).join(" ").slice(0, 200) }; }

// 3. a JSON-RPC READ of Robinhood Chain through an outcall (chain id, then the latest Pons factory logs)
const rpcUrl = "https://rpc.mainnet.chain.robinhood.com";
const q = (json) => call("rpc", `("${rpcUrl}", "${json.replaceAll('"', '\\"')}")`);
try {
  out.chainId = grab(q('{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}'));
  const unescape = (b) => b.replaceAll('\\"', '"').replaceAll("\\n", "").trim();
  const head = JSON.parse(unescape(grab(q('{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}')).body)).result;
  const from = "0x" + (parseInt(head, 16) - 200).toString(16);
  const logs = q(`{"jsonrpc":"2.0","id":1,"method":"eth_getLogs","params":[{"address":"0x7eD598BcEf8bd9Edd8C97A195C6d13f40801EC7e","fromBlock":"${from}","toBlock":"${head}"}]}`);
  out.ponsFactoryLogs = { status: grab(logs).status, count: (logs.match(/transactionHash/g) || []).length, head };
} catch (e) { out.rpc = { error: String(e.message).split("\n").slice(-3).join(" ").slice(0, 300) }; }
console.log(JSON.stringify(out, null, 2));
