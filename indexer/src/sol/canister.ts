/**
 * The canister's public queries the indexer reads (canister-only design §10.1), through @icp-sdk/core as ops/src/ic.ts and the
 * monitor do. Two of them, both anonymous query calls (free, no identity):
 *   - `coins_collections()`: the collection index the memos name ↔ its wX mint ↔ its DBC config ↔ its module `M[i]` and symbol;
 *   - `books_view(coll)`: the accepted floor (lamports per whole wX) the wX/SOL rate falls back to when the desk's pair is absent.
 * Records name only the fields read (Candid record subtyping skips the rest), every variant lists every tag. The types are
 * from autobuyer/canister-rs/etch_autobuyer.did (`CollectionView`, `BooksView`); the decoders are tested against the bytes the
 * canister's own tests write (tests/fixtures/views/*.hex). Nothing here sends an update call.
 */
import { Actor, HttpAgent } from "@icp-sdk/core/agent";
import { IDL } from "@icp-sdk/core/candid";

export const IC_HOST = "https://icp-api.io";

export type CanisterCollection = { index: number; wxMint: string; config: string; module: string; symbol: string; stage: string };
export type BooksFloor = { coll: number; wxMint: string; accepted: bigint; target: bigint; postedAt: number; fresh: boolean; halted: string | null;
  expectedSupply: bigint | null; mintPrice: bigint | null; burnPaysOne: bigint | null };
export type CanisterReader = { id: string; collections(): Promise<CanisterCollection[]>; books(coll: number): Promise<BooksFloor | null> };

const CollectionView = IDL.Record({ coins: IDL.Nat32, backed: IDL.Nat32, wx_mint: IDL.Text, stage: IDL.Text, index: IDL.Nat32, module: IDL.Text, config: IDL.Text, symbol: IDL.Text });
const BooksView = IDL.Record({ fresh: IDL.Bool, expected_supply: IDL.Opt(IDL.Nat64), coll: IDL.Nat16, wx_mint: IDL.Text, target: IDL.Nat64, posted_at: IDL.Int64,
  mint_price: IDL.Opt(IDL.Nat64), burn_pays_one: IDL.Opt(IDL.Nat64), halted: IDL.Opt(IDL.Text), accepted: IDL.Nat64 });
type CollectionV = { coins: number; backed: number; wx_mint: string; stage: string; index: number; module: string; config: string; symbol: string };
type BooksV = { fresh: boolean; expected_supply: [] | [bigint]; coll: number; wx_mint: string; target: bigint; posted_at: bigint; mint_price: [] | [bigint];
  burn_pays_one: [] | [bigint]; halted: [] | [string]; accepted: bigint };
const idl: IDL.InterfaceFactory = () => IDL.Service({
  coins_collections: IDL.Func([], [IDL.Vec(CollectionView)], ["query"]),
  books_view: IDL.Func([IDL.Nat16], [IDL.Opt(BooksView)], ["query"]),
});

const opt = <T>(o: [] | [T]): T | null => (o.length ? o[0] : null);
export const toCollection = (c: CollectionV): CanisterCollection => ({ index: c.index, wxMint: c.wx_mint, config: c.config, module: c.module, symbol: c.symbol, stage: c.stage });
export const toFloor = (v: BooksV): BooksFloor => ({ coll: v.coll, wxMint: v.wx_mint, accepted: v.accepted, target: v.target, postedAt: Number(v.posted_at), fresh: v.fresh,
  halted: opt(v.halted), expectedSupply: opt(v.expected_supply), mintPrice: opt(v.mint_price), burnPaysOne: opt(v.burn_pays_one) });

/** A saved `books_view` reply (the bytes of the candid reply), decoded as the actor would. */
export const decodeBooksView = (bytes: Uint8Array): BooksFloor | null => { const v = opt(IDL.decode([IDL.Opt(BooksView)], bytes)[0] as [] | [BooksV]); return v ? toFloor(v) : null; };
export const decodeCollections = (bytes: Uint8Array): CanisterCollection[] => (IDL.decode([IDL.Vec(CollectionView)], bytes)[0] as CollectionV[]).map(toCollection);

/** The reader over the IC (mainnet: the root key is built in; only a local replica would fetch one, and this never talks to one). */
export async function icCanister(id: string, host = IC_HOST): Promise<CanisterReader> {
  const agent = await HttpAgent.create({ host, shouldFetchRootKey: false });
  const actor = Actor.createActor(idl, { agent, canisterId: id }) as unknown as { coins_collections(): Promise<CollectionV[]>; books_view(coll: number): Promise<[] | [BooksV]> };
  return {
    id,
    collections: async () => (await actor.coins_collections()).map(toCollection),
    books: async (coll) => { const v = opt(await actor.books_view(coll)); return v ? toFloor(v) : null; },
  };
}
