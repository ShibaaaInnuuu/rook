// The only way the monitor talks to the IC: as the ANONYMOUS identity, query calls (the replica's answer is signed by the
// node, and the agent checks that signature) and `read_state` for controllers and module hashes (certified by the subnet;
// the agent verifies the certificate against the IC root key). No update call is ever made: this holds no key and can change
// nothing.
import { Actor, CanisterStatus, HttpAgent } from "@icp-sdk/core/agent";
import { Principal } from "@icp-sdk/core/principal";
import { buyerIdl, plain, timelockIdl } from "./candid.mjs";
import { EMPTY_RECORDS } from "./records.mjs";

const toHex = (x) => (x == null ? null : typeof x === "string" ? x.replace(/^0x/, "").toLowerCase() : Buffer.from(x).toString("hex"));

export async function icClient({ host = "https://icp-api.io", timelock, local = false }) {
  const agent = await HttpAgent.create({ host, shouldFetchRootKey: local });
  const tl = Actor.createActor(timelockIdl, { agent, canisterId: timelock });
  const buyers = new Map();
  const buyer = (id) => { if (!buyers.has(id)) buyers.set(id, Actor.createActor(buyerIdl, { agent, canisterId: id })); return buyers.get(id); };
  const q = async (what, p) => { try { return plain(await p); } catch (err) { throw new Error(`${what}: ${String(err.message).split("\n")[0]}`); } };
  return {
    status: () => q("timelock status", tl.status()),
    history: () => q("timelock history", tl.history()),
    cycles: (id) => q(`cycles of ${id}`, id === timelock ? tl.cycles() : buyer(id).cycles()),
    custodyCaps: (id) => q("custody_caps", buyer(id).custody_caps()),
    custodyOutflow: (id) => q("custody_outflow", buyer(id).custody_outflow()),
    /** audit D, H-1 and E-08: the ticket-log scanner, the canister's log, and which ordinals providers it asks. */
    ticketLogs: (id) => q("ticket_logs", buyer(id).ticket_logs()),
    log: (id) => q("log", buyer(id).log()),
    ordProviders: (id) => q("ord_providers", buyer(id).ord_providers()),
    holderDeposits: (id) => q("holder_deposits", buyer(id).holder_deposits()),
    /** canister review2 N-4 / N-2: the custody fee pool (quarantined entries) and the redemptions' markUndeliverable. */
    feePool: (id) => q("custody_fee_pool", buyer(id).custody_fee_pool()),
    redemptions: (id) => q("redemptions", buyer(id).redemptions()),
    /** docs/v6/relay-conversion.md §6: the Relay conversions (advance → quote → deposit → sats → frog). */
    conversions: (id) => q("frog_conversions", buyer(id).frog_conversions([])),
    /** docs/v7/phase1-nft.md §6: the Ethereum NFT side (buys, deliveries, floor readings, the Ethereum line). */
    ethState: (id) => q("eth_nft_state", buyer(id).eth_nft_state()),
    /** Audit C3-05: the configuration (which chains its collections live on) and the side of each other chain (`eth_nft_state_of`). */
    ethConfig: (id) => q("eth_nft_config", buyer(id).eth_nft_config()),
    ethStateOf: (id, chain) => q(`eth_nft_state_of(${chain})`, buyer(id).eth_nft_state_of(BigInt(chain))),
    /** docs/v9/solana-migration.md §2.4 (phase B2): the canister-wide Solana line (null: none, or an EVM-home buyer). */
    solLine: (id) => q("sol_line", buyer(id).sol_line()),
    /** W3: the four Solana send lines and the canister's own timer (a buyer built before W3 has no sol_lines: the caller falls back). */
    solLines: (id) => q("sol_lines", buyer(id).sol_lines()),
    /** W11 (canister-only): the treasury and cranks, the pool desks, the coins' log, the intent desk (a buyer without them: the caller falls back). */
    treasuryStatus: (id) => q("treasury_status", buyer(id).treasury_status()),
    poolDesks: (id) => q("pool_desks", buyer(id).pool_desks()),
    coinsLog: (id) => q("coins_log", buyer(id).coins_log()),
    deskStatus: (id) => q("desk_status", buyer(id).desk_status()),
    /** W15 (the canister-only rehearsal): one collection's books, the listed collections (id, wX mint) and the pinned ones. */
    booksView: (id, coll) => q(`books_view(${coll})`, buyer(id).books_view(coll)),
    booksCollections: (id) => q("books_collections", buyer(id).books_collections()),
    coinsCollections: (id) => q("coins_collections", buyer(id).coins_collections()),
    /** The records the canaries match against, all public queries, read together. */
    records: async (id) => {
      const b = buyer(id);
      // docs/v6/relay-conversion.md §6: a buyer built before the relay conversion has no frog_conversions; read as none, which
      // only makes the checks louder (an Advanced event with no record pages), never quieter
      const [redemptions, holder_deposits, frog_buys, sale_state, purchases, conversions, eth] = await Promise.all([
        q("redemptions", b.redemptions()), q("holder_deposits", b.holder_deposits()), q("frog_buys", b.frog_buys([])),
        q("sale_state", b.sale_state()), q("purchases", b.purchases()), q("frog_conversions", b.frog_conversions([])).catch(() => []),
        // docs/v7/phase1-nft.md: a buyer without the Ethereum side reads as none (louder, never quieter)
        q("eth_nft_state", b.eth_nft_state()).catch(() => null)]);
      return { ...EMPTY_RECORDS, redemptions, holder_deposits, frog_buys, sale_state, purchases, conversions, eth };
    },
    /** Controllers and module hash, from the IC's certified state (what `dfx canister info` shows). */
    certified: async (id) => {
      try {
        const m = await CanisterStatus.request({ canisterId: Principal.fromText(id), agent, paths: ["controllers", "module_hash"] });
        return { controllers: (m.get("controllers") ?? []).map((p) => p.toText()), module_hash: toHex(m.get("module_hash")) };
      } catch (err) {
        throw new Error(`certified read_state of ${id}: ${String(err.message).split("\n")[0]}`);
      }
    },
  };
}
