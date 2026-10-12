import { useQuery } from "@tanstack/react-query";
import { href, useNow } from "../lib/env";
import { decodeMint } from "../rook/sol/accounts";
import type { Chain } from "../rook/sol/collections";
import { backingFigures, redemptionValue } from "../rook/sol/booksReserves";
import { DEMO } from "../rook/sol/demo";
import { demoCoinSupply } from "../rook/sol/demoCanister";
import { age, solscan, units } from "../rook/sol/format";
import { rpc } from "../rook/sol/hooksLite";
import { CANISTER_ON, useBooksView, useCoinsVaults } from "../rook/sol/hooksCanister";
import { Fig, Pill } from "./bits";
import { Addr } from "./sol";

/**
 * What stands behind a collection's wX (docs/v9/canister-only-solana.md §4, §10.1 `books`/`ledger`): every number is the canister's
 * `books_view`. Circulating wX = NFTs in custody (less redemptions in flight) × 1 + the SOL-backed units; the SOL side sits in the module
 * `M[i]` as the backing pot and the standing reserve; the accepted floor prices mints and burns.
 */
export function CollectionBacking({ coll, wx, slug, chain }: { coll: number | null; wx: string; slug: string; chain: Chain }) {
  const q = useBooksView(CANISTER_ON ? coll : null);
  const now = useNow();
  const item = chain === "Bitcoin" ? "inscriptions" : "NFTs";
  if (!CANISTER_ON) return <section className="panel panel--tight small"><p className="eyebrow">Backing</p><p className="muted" style={{ margin: 0 }}>This build names no Rook canister yet: the books of {wx} (the {item} in custody, the SOL behind the rest) show here once it is installed.</p></section>;
  if (q.isLoading) return <section className="panel panel--tight small"><p className="eyebrow">Backing</p><p className="muted" style={{ margin: 0 }}>Reading the books…</p></section>;
  if (q.error) return <section className="panel panel--tight small"><p className="eyebrow">Backing</p><p className="error" style={{ margin: 0 }}>The books could not be read: {String((q.error as Error).message).slice(0, 160)}</p></section>;
  const v = q.data;
  if (!v) return <section className="panel panel--tight small"><p className="eyebrow">Backing</p><p className="muted" style={{ margin: 0 }}>The canister keeps no books for this collection yet (it is not registered, or its wX is not minted).</p></section>;
  const f = backingFigures(v);
  const halted = v.halted.length ? v.halted[0] : null;
  return (
    <section className="panel panel--tight" aria-labelledby={`backing-${coll}`}>
      <p className="eyebrow" id={`backing-${coll}`} style={{ margin: 0 }}>Backing · {wx}</p>
      <p className="small" style={{ margin: ".25rem 0 .75rem" }}>
        {halted ? <Pill tone="bad">stopped</Pill> : v.paused ? <Pill tone="warn">mint paused</Pill> : v.fresh ? <Pill tone="good">trades</Pill> : v.escaped ? <Pill tone="warn">stale 7 d: burns at the backing</Pill> : <Pill tone="warn">floor stale</Pill>}
        {v.poster_paused ? <> <Pill tone="warn">poster paused</Pill></> : null}
        <span className="muted"> floor posted {age(Number(v.posted_at), now)} ago</span>
      </p>
      <div className="figs">
        <Fig label={`${wx} in circulation`} value={units(f.circulating, 9, 2)} />
        <Fig label={`${item[0].toUpperCase()}${item.slice(1)} in custody`} value={f.itemsHeld.toString()} unit={f.ticketsOpen > 0n ? `${f.ticketsOpen.toString()} being delivered` : undefined} />
        <Fig label="SOL-backed units" value={units(f.solBacked, 9, 2)} unit={wx} />
        <Fig label="Accepted floor" value={units(f.acceptedFloor, 9, 2)} unit="SOL" />
      </div>
      <dl className="kv small" style={{ marginTop: ".75rem" }}>
        <dt>Mint price now</dt><dd>{f.mintPrice !== null ? `${units(f.mintPrice, 9, 3)} SOL` : "no mint: floor stale or paused"}</dd>
        <dt>Burn pays, per {wx}</dt><dd>{f.burnPaysOne !== null ? `${units(f.burnPaysOne, 9, 3)} SOL` : "—"}</dd>
        <dt>Backing pot</dt><dd>{units(v.backing, 9, 3)} SOL<small className="muted"> converts into {item}</small></dd>
        <dt>Standing reserve</dt><dd>{units(v.reserve, 9, 3)} SOL<small className="muted"> {(f.reserveShare * 100).toFixed(0)} % of the SOL side; never advanced</small></dd>
        <dt>Surplus · revenue</dt><dd>{units(v.surplus, 9, 3)} · {units(v.revenue, 9, 3)} SOL</dd>
        <dt>Module must hold</dt><dd>{units(v.owed, 9, 3)} SOL</dd>
        <dt>In transit</dt><dd>{units(v.advanced, 9, 3)} SOL advanced{v.transit_units > 0n ? `, ${units(v.transit_units, 9, 2)} ${wx} in transit` : ""}</dd>
        <dt>7-day volume</dt><dd>{units(v.volume_7d, 9, 2)} SOL</dd>
        {v.raffle_pot > 0n || v.raffle_prize > 0n ? <><dt>Raffle</dt><dd>{units(v.raffle_pot, 9, 3)} SOL in the pot, {units(v.raffle_prize, 9, 2)} {wx} held as prize</dd></> : null}
      </dl>
      <p className="small muted" style={{ margin: ".5rem 0 0" }}>wX mint <Addr id={v.wx_mint} kind="token" />. The same books against Solana, checked: <a href={href("reserves", slug)}>reserves</a>.</p>
    </section>
  );
}

/** A graduated coin's vault (§7): the wX it holds for the coin's holders, and what one coin redeems for right now. */
export function CoinBacking({ coll, mint, symbol, wx, decimals }: { coll: number; mint: string; symbol: string; wx: string; decimals: number }) {
  const vaults = useCoinsVaults(CANISTER_ON ? coll : null);
  const supply = useQuery({ queryKey: ["sol-mint-supply", mint], enabled: CANISTER_ON, refetchInterval: 60_000,
    queryFn: async () => { if (DEMO) return demoCoinSupply(mint); const a = await rpc().account(mint, "finalized"); return a ? decodeMint(a.data).supply : null; } });
  if (!CANISTER_ON) return null;
  if (vaults.isLoading) return <p className="small muted">Reading the coin's vault from the canister…</p>;
  if (vaults.error) return <p className="small error">The vault could not be read: {String((vaults.error as Error).message).slice(0, 160)}</p>;
  const v = vaults.data?.find((x) => x.coin === mint) ?? null;
  if (!v) return <p className="small muted">The canister keeps no vault for this coin yet: a coin that has not graduated has no backing and no claim.</p>;
  const held = v.wx_in - v.wx_paid;
  const addr = v.vault.length ? v.vault[0] : null;
  const one = 10n ** BigInt(decimals);
  const per = supply.data ? redemptionValue(held, one, supply.data) : null;
  return (
    <div>
      <p style={{ margin: 0 }}>Vault {addr ? <Addr id={addr} /> : <Pill tone="warn">not opened</Pill>}: <strong>{units(held, 9, 4)} {wx}</strong> held for the holders{v.backed_at > 0n ? ` (backed ${new Date(Number(v.backed_at) * 1000).toISOString().slice(0, 10)})` : ""}; {units(v.coins_burned, decimals, 0)} {symbol} redeemed so far for {units(v.wx_paid, 9, 4)} {wx}. <Pill tone={v.stage === "Backed" ? "good" : "warn"}>{v.stage}</Pill></p>
      <dl className="kv small" style={{ marginTop: ".5rem" }}>
        <dt>One {symbol} redeems for</dt><dd>{per !== null ? `${units(per, 9, 6)} ${wx}` : supply.isLoading ? "…" : "— (the supply could not be read)"}<small className="muted"> ⌊vault × coins ÷ live supply⌋, rounded down</small></dd>
        <dt>Pool</dt><dd>{v.pool ? <a className="mono" href={solscan("account", v.pool)} target="_blank" rel="noreferrer noopener">{v.pool.slice(0, 4)}…{v.pool.slice(-4)}</a> : "—"}</dd>
      </dl>
    </div>
  );
}
