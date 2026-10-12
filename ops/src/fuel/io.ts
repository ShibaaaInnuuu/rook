import { Actor, HttpAgent, type Identity } from "@icp-sdk/core/agent";
import { b58 } from "./sol.js";
import { IDL } from "@icp-sdk/core/candid";
import { Principal } from "@icp-sdk/core/principal";
import type { Hex } from "viem";
import { accountId, IC, ICP_FEE_E8S, CKETH_FEE_WEI, TPUP_MEMO } from "./pinned.js";

/**
 * docs/v7/cycles-from-fees.md §4.2. Everything the fueler reads or calls, behind one interface (the engine is tested against a fake of
 * it). This file signs nothing: a raw EVM transaction arrives already signed and checked by the engine's allowlist, and the IC calls
 * here are only the fixed ones of the route (approve to the pinned pool, the pool's deposit / swap / withdraw for the caller itself,
 * the ICP transfer to the CMC's top-up account, `notify_top_up`).
 */
export type Receipt = { ok: boolean; block: bigint };
export type Unused = { icp: bigint; ck: bigint };
export type Notify = { ok: bigint } | { processing: true } | { refunded: string } | { err: string };

export interface FuelIo {
  evmBalance(chain: number, addr: Hex): Promise<bigint>;
  evmNonce(chain: number, addr: Hex): Promise<number>;
  evmReceipt(chain: number, hash: Hex): Promise<Receipt | null>;
  evmCall(chain: number, to: Hex, data: Hex): Promise<Hex>;
  evmCode(chain: number, addr: Hex): Promise<Hex>;
  evmBaseFee(chain: number): Promise<bigint>;
  evmEstimate(chain: number, tx: { from: Hex; to: Hex; data: Hex; value: bigint }): Promise<bigint>;
  evmBroadcast(chain: number, raw: Hex): Promise<void>;
  relayQuote(body: Record<string, unknown>): Promise<any>; // eslint-disable-line @typescript-eslint/no-explicit-any
  prices(): Promise<{ cg: { eth?: number; icp?: number; sol?: number } | null; cb: { eth?: number; icp?: number; sol?: number } | null }>;
  curves(): Promise<Hex[]>;
  cycles(canister: string): Promise<bigint>;
  /** The books home (canister-only Solana): the canister's `treasury_status().treasury`: its ops key (base58, or null while unset) and the lamports it paid ops so far. */
  booksTreasury(canister: string): Promise<{ ops: string | null; totalToOps: bigint }>;
  xdrRate(): Promise<bigint>;
  minterHelper(): Promise<string | null>;
  poolTokens(): Promise<{ token0: string; token1: string }>;
  ckBalance(): Promise<bigint>;
  ckApprove(amount: bigint): Promise<void>;
  poolUnused(): Promise<Unused>;
  poolDeposit(amount: bigint): Promise<void>;
  poolQuote(ckIn: bigint): Promise<bigint>;
  poolSwap(ckIn: bigint, minOut: bigint): Promise<bigint>;
  poolWithdraw(icpE8s: bigint): Promise<void>;
  icpBalance(): Promise<bigint>;
  /** The legacy ICP transfer with the TPUP memo and a fixed created-at time: a retry of the same transfer answers its block. */
  icpTransfer(to: Uint8Array, e8s: bigint, createdAtNs: bigint): Promise<bigint>;
  notifyTopUp(block: bigint, canister: string): Promise<Notify>;
  principal(): string;
}

// ---- EVM over JSON-RPC ---------------------------------------------------------------------------------------------------------

async function rpc(url: string, method: string, params: unknown[], timeoutMs = 20_000): Promise<any> { // eslint-disable-line @typescript-eslint/no-explicit-any
  const r = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
    signal: AbortSignal.timeout(timeoutMs) });
  if (!r.ok) throw new Error(`${method}: HTTP ${r.status}`);
  const j = await r.json();
  if (j.error) throw new Error(`${method}: ${j.error.code} ${String(j.error.message).slice(0, 160)}`);
  return j.result;
}
const all = async <X>(urls: string[], f: (u: string) => Promise<X>): Promise<X[]> => {
  const rs = await Promise.allSettled(urls.map(f));
  const ok = rs.filter((r): r is PromiseFulfilledResult<Awaited<X>> => r.status === "fulfilled").map((r) => r.value);
  if (!ok.length) throw new Error(`every provider failed: ${rs.map((r) => (r.status === "rejected" ? String(r.reason?.message ?? r.reason).slice(0, 80) : "")).join("; ")}`);
  return ok;
};

// ---- the IC's interfaces, as far as they are called -------------------------------------------------------------------------------

const Account = IDL.Record({ owner: IDL.Principal, subaccount: IDL.Opt(IDL.Vec(IDL.Nat8)) });
const ApproveError = IDL.Variant({ GenericError: IDL.Record({ message: IDL.Text, error_code: IDL.Nat }), TemporarilyUnavailable: IDL.Null,
  Duplicate: IDL.Record({ duplicate_of: IDL.Nat }), BadFee: IDL.Record({ expected_fee: IDL.Nat }), AllowanceChanged: IDL.Record({ current_allowance: IDL.Nat }),
  CreatedInFuture: IDL.Record({ ledger_time: IDL.Nat64 }), TooOld: IDL.Null, Expired: IDL.Record({ ledger_time: IDL.Nat64 }), InsufficientFunds: IDL.Record({ balance: IDL.Nat }) });
const ckLedgerIdl: IDL.InterfaceFactory = () => IDL.Service({
  icrc1_balance_of: IDL.Func([Account], [IDL.Nat], ["query"]),
  icrc2_approve: IDL.Func([IDL.Record({ fee: IDL.Opt(IDL.Nat), memo: IDL.Opt(IDL.Vec(IDL.Nat8)), from_subaccount: IDL.Opt(IDL.Vec(IDL.Nat8)),
    created_at_time: IDL.Opt(IDL.Nat64), amount: IDL.Nat, expected_allowance: IDL.Opt(IDL.Nat), expires_at: IDL.Opt(IDL.Nat64), spender: Account })],
  [IDL.Variant({ Ok: IDL.Nat, Err: ApproveError })], []),
});
const Tokens = IDL.Record({ e8s: IDL.Nat64 });
const icpLedgerIdl: IDL.InterfaceFactory = () => IDL.Service({
  account_balance: IDL.Func([IDL.Record({ account: IDL.Vec(IDL.Nat8) })], [Tokens], ["query"]),
  transfer: IDL.Func([IDL.Record({ memo: IDL.Nat64, amount: Tokens, fee: Tokens, from_subaccount: IDL.Opt(IDL.Vec(IDL.Nat8)), to: IDL.Vec(IDL.Nat8),
    created_at_time: IDL.Opt(IDL.Record({ timestamp_nanos: IDL.Nat64 })) })],
  [IDL.Variant({ Ok: IDL.Nat64, Err: IDL.Variant({ BadFee: IDL.Record({ expected_fee: Tokens }), InsufficientFunds: IDL.Record({ balance: Tokens }),
    TxTooOld: IDL.Record({ allowed_window_nanos: IDL.Nat64 }), TxCreatedInFuture: IDL.Null, TxDuplicate: IDL.Record({ duplicate_of: IDL.Nat64 }) }) })], []),
});
const PoolError = IDL.Variant({ CommonError: IDL.Null, InsufficientFunds: IDL.Null, InternalError: IDL.Text, UnsupportedToken: IDL.Text });
const PoolResult = IDL.Variant({ ok: IDL.Nat, err: PoolError });
const SwapArgs = IDL.Record({ amountIn: IDL.Text, zeroForOne: IDL.Bool, amountOutMinimum: IDL.Text });
const MoveArgs = IDL.Record({ fee: IDL.Nat, token: IDL.Text, amount: IDL.Nat });
const Token = IDL.Record({ address: IDL.Text, standard: IDL.Text });
const poolIdl: IDL.InterfaceFactory = () => IDL.Service({
  metadata: IDL.Func([], [IDL.Variant({ ok: IDL.Record({ token0: Token, token1: Token, fee: IDL.Nat }), err: PoolError })], ["query"]),
  getUserUnusedBalance: IDL.Func([IDL.Principal], [IDL.Variant({ ok: IDL.Record({ balance0: IDL.Nat, balance1: IDL.Nat }), err: PoolError })], ["query"]),
  quote: IDL.Func([SwapArgs], [PoolResult], ["query"]),
  depositFrom: IDL.Func([MoveArgs], [PoolResult], []),
  swap: IDL.Func([SwapArgs], [PoolResult], []),
  withdraw: IDL.Func([MoveArgs], [PoolResult], []),
});
const NotifyError = IDL.Variant({ Refunded: IDL.Record({ reason: IDL.Text, block_index: IDL.Opt(IDL.Nat64) }), Processing: IDL.Null,
  TransactionTooOld: IDL.Nat64, InvalidTransaction: IDL.Text, Other: IDL.Record({ error_code: IDL.Nat64, error_message: IDL.Text }) });
const cmcIdl: IDL.InterfaceFactory = () => IDL.Service({
  notify_top_up: IDL.Func([IDL.Record({ block_index: IDL.Nat64, canister_id: IDL.Principal })], [IDL.Variant({ Ok: IDL.Nat, Err: NotifyError })], []),
  get_icp_xdr_conversion_rate: IDL.Func([], [IDL.Record({ data: IDL.Record({ timestamp_seconds: IDL.Nat64, xdr_permyriad_per_icp: IDL.Nat64 }),
    hash_tree: IDL.Vec(IDL.Nat8), certificate: IDL.Vec(IDL.Nat8) })], ["query"]),
});
const minterIdl: IDL.InterfaceFactory = () => IDL.Service({
  get_minter_info: IDL.Func([], [IDL.Record({ deposit_with_subaccount_helper_contract_address: IDL.Opt(IDL.Text) })], ["query"]),
});
const cyclesIdl: IDL.InterfaceFactory = () => IDL.Service({ cycles: IDL.Func([], [IDL.Record({ balance: IDL.Nat, low_threshold: IDL.Nat })], ["query"]) });
// the two fields of `treasury_status` the fueler reads (a Candid record decodes with the fields it names; the rest of the reply is skipped)
const treasuryStatusIdl: IDL.InterfaceFactory = () => IDL.Service({ treasury_status: IDL.Func([], [IDL.Record({ treasury: IDL.Record({ ops: IDL.Opt(IDL.Vec(IDL.Nat8)), total_to_ops: IDL.Nat64 }) })], ["query"]) });

const poolErr = (e: Record<string, unknown>) => Object.entries(e).map(([k, v]) => (v === null ? k : `${k}: ${String(v)}`)).join(", ");
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const okOf = (r: any, what: string): bigint => { if ("ok" in r) return BigInt(r.ok); throw new Error(`${what}: ${poolErr(r.err)}`); };

export type IoConfig = { rpcs: Record<number, string[]>; relayApi: string; relayKey?: string; indexer: string; icHost: string; identity: Identity | null };

/** The real world. */
export async function realIo(c: IoConfig): Promise<FuelIo> {
  const agent = await HttpAgent.create({ host: c.icHost, identity: c.identity ?? undefined });
  const me = c.identity ? c.identity.getPrincipal() : Principal.anonymous();
  type A = Record<string, (...a: unknown[]) => Promise<any>>; // eslint-disable-line @typescript-eslint/no-explicit-any
  const actor = (idl: IDL.InterfaceFactory, id: string) => Actor.createActor(idl, { agent, canisterId: id }) as unknown as A;
  const ck = actor(ckLedgerIdl, IC.CKETH_LEDGER), icp = actor(icpLedgerIdl, IC.ICP_LEDGER), pool = actor(poolIdl, IC.POOL_ICP_CKETH);
  const cmc = actor(cmcIdl, IC.CMC), minter = actor(minterIdl, IC.CKETH_MINTER);
  const urls = (chain: number) => { const u = c.rpcs[chain]; if (!u?.length) throw new Error(`no RPC for chain ${chain}`); return u; };
  const getJson = async (u: string, init?: RequestInit) => {
    const r = await fetch(u, { ...init, signal: AbortSignal.timeout(20_000) });
    if (!r.ok) throw new Error(`${new URL(u).host}: HTTP ${r.status}`);
    return r.json();
  };
  return {
    principal: () => me.toText(),
    async evmBalance(chain, addr) { return (await all(urls(chain), async (u) => BigInt(await rpc(u, "eth_getBalance", [addr, "latest"])))).reduce((m, x) => (x < m ? x : m)); },
    async evmNonce(chain, addr) { return Math.max(...(await all(urls(chain), async (u) => Number(BigInt(await rpc(u, "eth_getTransactionCount", [addr, "latest"])))))); },
    async evmReceipt(chain, hash) {
      const rs = (await all(urls(chain), (u) => rpc(u, "eth_getTransactionReceipt", [hash]))).filter(Boolean);
      if (!rs.length) return null;
      const st = new Set(rs.map((r) => r.status));
      if (st.size > 1) throw new Error(`the providers disagree on the receipt of ${hash}`);
      return { ok: rs[0].status === "0x1", block: BigInt(rs[0].blockNumber) };
    },
    async evmCall(chain, to, data) { return (await all(urls(chain).slice(0, 1), (u) => rpc(u, "eth_call", [{ to, data }, "latest"])))[0]; },
    async evmCode(chain, addr) { return (await all(urls(chain), (u) => rpc(u, "eth_getCode", [addr, "latest"]))).find((x) => x !== "0x") ?? "0x"; },
    async evmBaseFee(chain) {
      const fees = await all(urls(chain), async (u) => BigInt((await rpc(u, "eth_getBlockByNumber", ["latest", false]))?.baseFeePerGas ?? "0x0"));
      return fees.reduce((m, x) => (x > m ? x : m), 0n);
    },
    async evmEstimate(chain, tx) {
      return BigInt(await rpc(urls(chain)[0], "eth_estimateGas", [{ from: tx.from, to: tx.to, data: tx.data, value: `0x${tx.value.toString(16)}` }]));
    },
    async evmBroadcast(chain, raw) {
      // every provider; "already known" / "nonce too low" are answers to a rebroadcast, not failures: the receipt decides
      await Promise.allSettled(urls(chain).map((u) => rpc(u, "eth_sendRawTransaction", [raw])));
    },
    async relayQuote(body) {
      const r = await fetch(`${c.relayApi}/quote/v2`, { method: "POST", signal: AbortSignal.timeout(20_000),
        headers: { "content-type": "application/json", ...(c.relayKey ? { "x-api-key": c.relayKey } : {}) }, body: JSON.stringify(body) });
      const j = await r.json();
      if (!r.ok) throw new Error(`Relay refused the quote: HTTP ${r.status} ${String(j?.message ?? "").slice(0, 200)}`);
      return j;
    },
    async prices() {
      const cg = await getJson("https://api.coingecko.com/api/v3/simple/price?ids=ethereum,internet-computer,solana&vs_currencies=usd")
        .then((j) => ({ eth: Number(j?.ethereum?.usd), icp: Number(j?.["internet-computer"]?.usd), sol: Number(j?.solana?.usd) })).catch(() => null);
      const spot = (p: string) => getJson(`https://api.coinbase.com/v2/prices/${p}/spot`).then((j) => Number(j?.data?.amount));
      const cb = await Promise.all([spot("ETH-USD"), spot("ICP-USD"), spot("SOL-USD").catch(() => undefined)])
        .then(([eth, icp, sol]) => ({ eth, icp, sol })).catch(() => null);
      return { cg, cb };
    },
    async curves() {
      const out: Hex[] = [];
      for (let offset = 0; ; offset += 200) {
        const p = await getJson(`${c.indexer}/v1/coins?limit=200&offset=${offset}`);
        for (const x of p.items ?? []) if (/^0x[0-9a-fA-F]{40}$/.test(x.curve)) out.push(String(x.curve).toLowerCase() as Hex);
        if (!p.items?.length || offset + 200 >= Number(p.total)) break;
      }
      return out;
    },
    async cycles(canister) { return BigInt((await actor(cyclesIdl, canister).cycles()).balance); },
    async booksTreasury(canister) {
      const t = (await actor(treasuryStatusIdl, canister).treasury_status()).treasury;
      return { ops: t.ops.length ? b58(Uint8Array.from(t.ops[0])) : null, totalToOps: BigInt(t.total_to_ops) };
    },
    async xdrRate() { return BigInt((await cmc.get_icp_xdr_conversion_rate()).data.xdr_permyriad_per_icp); },
    async minterHelper() { const v = (await minter.get_minter_info()).deposit_with_subaccount_helper_contract_address; return v.length ? String(v[0]).toLowerCase() : null; },
    async poolTokens() { const r = await pool.metadata(); if (!("ok" in r)) throw new Error(`pool metadata: ${poolErr(r.err)}`); return { token0: r.ok.token0.address, token1: r.ok.token1.address }; },
    async ckBalance() { return BigInt(await ck.icrc1_balance_of({ owner: me, subaccount: [] })); },
    async ckApprove(amount) {
      const r = await ck.icrc2_approve({ fee: [CKETH_FEE_WEI], memo: [], from_subaccount: [], created_at_time: [], amount, expected_allowance: [], expires_at: [],
        spender: { owner: Principal.fromText(IC.POOL_ICP_CKETH), subaccount: [] } });
      if (!("Ok" in r)) throw new Error(`ckETH approve: ${Object.keys(r.Err)[0]}`);
    },
    async poolUnused() { const r = await pool.getUserUnusedBalance(me); if (!("ok" in r)) throw new Error(`unused balance: ${poolErr(r.err)}`); return { icp: BigInt(r.ok.balance0), ck: BigInt(r.ok.balance1) }; },
    async poolDeposit(amount) { okOf(await pool.depositFrom({ token: IC.CKETH_LEDGER, amount, fee: CKETH_FEE_WEI }), "pool depositFrom"); },
    async poolQuote(ckIn) { return okOf(await pool.quote({ amountIn: ckIn.toString(), zeroForOne: false, amountOutMinimum: "0" }), "pool quote"); },
    async poolSwap(ckIn, minOut) { return okOf(await pool.swap({ amountIn: ckIn.toString(), zeroForOne: false, amountOutMinimum: minOut.toString() }), "pool swap"); },
    async poolWithdraw(icpE8s) { okOf(await pool.withdraw({ token: IC.ICP_LEDGER, amount: icpE8s, fee: ICP_FEE_E8S }), "pool withdraw"); },
    async icpBalance() { return BigInt((await icp.account_balance({ account: Array.from(accountId(me)) })).e8s); },
    async icpTransfer(to, e8s, createdAtNs) {
      const r = await icp.transfer({ memo: TPUP_MEMO, amount: { e8s }, fee: { e8s: ICP_FEE_E8S }, from_subaccount: [], to: Array.from(to),
        created_at_time: [{ timestamp_nanos: createdAtNs }] });
      if ("Ok" in r) return BigInt(r.Ok);
      if ("TxDuplicate" in r.Err) return BigInt(r.Err.TxDuplicate.duplicate_of);
      throw new Error(`ICP transfer: ${Object.keys(r.Err)[0]}`);
    },
    async notifyTopUp(block, canister) {
      const r = await cmc.notify_top_up({ block_index: block, canister_id: Principal.fromText(canister) });
      if ("Ok" in r) return { ok: BigInt(r.Ok) };
      if ("Processing" in r.Err) return { processing: true };
      if ("Refunded" in r.Err) return { refunded: r.Err.Refunded.reason };
      return { err: Object.keys(r.Err)[0] };
    },
  };
}
