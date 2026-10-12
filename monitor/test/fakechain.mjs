// A fake 4663 JSON-RPC for the tests: eth_call is answered from a table of view results (encoded with the real ABI), and
// everything else from handlers the test gives. No network.
import { decodeFunctionData, encodeFunctionResult } from "viem";
import { ERC20, MODULE, REGISTRY, VAULT, WRAPPER } from "../src/abi.mjs";

const ABIS = [REGISTRY, MODULE, VAULT, WRAPPER, ERC20, [{ type: "function", name: "routerOf", stateMutability: "view", inputs: [{ type: "address" }], outputs: [{ type: "address" }] }]];

/**
 * `views(url, to, fn, args)` returns the decoded value of a view (or throws). `other(url, method, params)` answers the rest.
 * Returns an `http` for the monitor's `rpc()`, and `calls`, every request seen.
 */
export function fakeChain({ views = () => { throw new Error("no view"); }, other = () => { throw new Error("no handler"); } }) {
  const calls = [];
  const http = async (url, init) => {
    const { method, params } = JSON.parse(init.body);
    calls.push({ url, method, params });
    let result, error = null;
    try {
      if (method === "eth_call" && params[0].data && !params[0].from) {
        const { to, data } = params[0];
        let hit = null;
        for (const abi of ABIS) { try { hit = { abi, ...decodeFunctionData({ abi, data }) }; break; } catch { /* next */ } }
        if (!hit) throw new Error(`unknown call ${data.slice(0, 10)}`);
        const v = await views(url, to.toLowerCase(), hit.functionName, hit.args ?? []);
        result = encodeFunctionResult({ abi: hit.abi, functionName: hit.functionName, result: v });
      } else result = await other(url, method, params);
    } catch (err) { error = { code: -32000, message: String(err.message) }; }
    return { ok: true, json: async () => (error ? { jsonrpc: "2.0", id: 1, error } : { jsonrpc: "2.0", id: 1, result }) };
  };
  return { http, calls };
}

export const hex = (n) => `0x${BigInt(n).toString(16)}`;
