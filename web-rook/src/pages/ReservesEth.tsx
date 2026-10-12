import { useMemo, useState } from "react";
import { href, useNow } from "../lib/env";
import { fmtUnits, servedStatus } from "../rook/v8/core/ethReserve";
import { rarityRanks, traitCounts } from "../rook/v8/core/rarity";
import { checkReserves, DEFAULT_ETH_RPCS, DEFAULT_HOME_RPCS } from "../rook/v8/core/selfcheck";
import { looksKeyed } from "../rook/v8/core/jsonrpc";
import type { CallLog, EthItem, EthReserveReport, MetadataSnapshot, Num } from "../rook/v8/core/types";
import { useEthReserves, useRecordedSelfCheck, useSnapshot, useV8Source } from "../rook/v8/source";
import { Status, StatusPill, ago } from "../ui/bits";
import "../styles/v8.css";

/**
 * docs/v8/proof-of-reserves.md: the reserves page's Ethereum section. One status per collection from public reads only, each number
 * with its source, block and age, re-judged at every render (servedStatus: stale is never green; a report past 3 h is red); the
 * canister's own records labelled (P4); and "check it yourself", from the visitor's browser to providers the visitor chooses (P2:
 * the defaults are keyless and independent of the canister's and the monitor's).
 */
/** ETH to four places for reading (the exact wei is in the JSON and in every call the self-check logs). */
const eth = (wei: string | null) => { if (wei === null) return "?"; const v = BigInt(wei), q = 10n ** 14n, r = (v + q / 2n) / q; return `${fmtUnits(r * q)} ETH`; };
const units = (u: string | null) => (u === null ? "?" : fmtUnits(BigInt(u)));
const shortHash = (h: string) => `${h.slice(0, 10)}…${h.slice(-6)}`;

export function EthNests() {
  const q = useEthReserves(), now = useNow();
  if (!q.data?.length) return null;
  return (
    <>
      <p className="eyebrow" style={{ marginTop: "2rem" }}>Ethereum collections</p>
      <ul className="nests rise rise-1">
        {q.data.map((r) => { const s = servedStatus(r, now); return (
          <li key={r.slug}><a className="nest" href={href("reserves", r.slug)}>
            <span className="nest__name">{r.name}<small>{r.symbol} · checked {ago(r.computedAt, now)}{r.lastGreenAt ? `, last green ${ago(r.lastGreenAt, now)}` : ", never green"}</small></span>
            <span className="fig"><span className="fig__label">NFTs</span><span className="fig__value">{r.equation.inCustody}</span></span>
            <span className="fig"><span className="fig__label">In flight</span><span className="fig__value">{r.equation.ticketsInFlight}</span></span>
            <span className="fig"><span className="fig__label">{r.symbol} supply</span><span className="fig__value">{units(r.equation.supplyUnits)}</span></span>
            <span><StatusPill status={s.status} /></span>
          </a></li>); })}
      </ul>
    </>
  );
}

export function EthLedger({ r, fixture }: { r: EthReserveReport; fixture: boolean }) {
  const now = useNow(), snap = useSnapshot(r.slug), src = useV8Source();
  const s = servedStatus(r, now);
  const link = (url: string | null, text: string) => (fixture || !url ? <span className="mono break">{text}</span> : <a className="mono break" href={url} target="_blank" rel="noreferrer noopener">{text}</a>);
  const E = BigInt(r.equation.ethBackedUnits), inFlight = r.equation.ticketsInFlight;
  return (
    <>
      <header className="pagehead rise">
        <p className="eyebrow"><a href={href("reserves")}>Reserves</a> · <a href={href("draw", r.slug)}>{r.name}: redemptions</a></p>
        <h1>{r.name}: the reserves</h1>
        <div className="row"><Status status={s.status} />
          <span className="small muted">computed {ago(r.computedAt, now)} · Robinhood Chain block {r.homeBlock?.toLocaleString("en-US") ?? "?"} ({ago(r.homeAt, now)}) · Ethereum finalized block {r.ethBlock?.toLocaleString("en-US") ?? "?"} ({ago(r.ethAt, now)}) · {r.lastGreenAt ? `last green ${ago(r.lastGreenAt, now)}` : "never green"}</span></div>
        {s.reasons.length ? <ul className="reasons">{s.reasons.map((x, i) => <li key={i}>{x}</li>)}</ul> : null}
        {r.warnings.length ? <ul className="reasons v8-amber">{r.warnings.map((x, i) => <li key={i}>{x}</li>)}</ul> : null}
        {fixture ? <p className="notice notice--warn small" style={{ marginTop: "1rem" }}><strong>Fixture.</strong> {src.data?.note}</p> : null}
        <p className="small muted v8-measure">Green only when every NFT is owned by the vault's custodian at Ethereum's finalized block on two providers that agree, every arrival checks out, every read is under {r.limits.maxAgeS / 60} minutes old, and the sum is exact. A read that failed, disagreed, or came from one provider only is unverified; a report older than {r.limits.redAfterS / 3600} hours is red. Recomputed every {r.limits.cadenceS / 60} minutes.</p>
      </header>

      <div className="grid v8-grid rise rise-1">
        <div className="span-6">
          <p className="eyebrow">The equation</p>
          <dl className="ledger">
            <div className="ledger__row"><dt>NFTs owned by the vault's custodian<small>ownerOf at Ethereum block {r.ethBlock?.toLocaleString("en-US") ?? "?"} (finalized), on {r.providers.eth.join(" and ")}</small></dt><dd className="num">{r.equation.inCustody}{r.equation.unverified ? ` (+${r.equation.unverified} unverified)` : ""}</dd></div>
            <div className="ledger__row"><dt>Less redemptions in flight<small>burned {r.symbol} whose NFT is not delivered yet</small></dt><dd className="num">− {inFlight}</dd></div>
            <div className={`ledger__row${E === 0n ? " ledger__row--muted" : ""}`}><dt>Plus {r.symbol} backed by ETH<small>the floor module's E = C + R + T, on Robinhood Chain</small></dt><dd className="num">+ {units(r.equation.ethBackedUnits)}</dd></div>
            <div className="ledger__row ledger__row--sum"><dt>Backing</dt><dd className="num">{units(r.equation.backingUnits)}</dd></div>
            <div className="ledger__row ledger__row--vs"><dt>{r.symbol} supply<small>{r.symbol} totalSupply at block {r.homeBlock?.toLocaleString("en-US") ?? "?"}, on {r.providers.home.join(" and ")}</small></dt><dd className="num">{r.equation.holds === null ? "?" : r.equation.holds ? "=" : "≠"} {units(r.equation.supplyUnits)}</dd></div>
          </dl>
          <p className="small muted">What green does not prove: that nothing can leave between two reads, that a timelocked upgrade will not move the vault (pending proposals, the 72-hour delay and the guardians are on the trust page), or that the floor price is right.</p>
        </div>
        <div className="span-6"><SelfCheck r={r} fixture={fixture} /></div>
      </div>

      <section className="block rise rise-2">
        <p className="eyebrow">Every number, with its source and age</p>
        <div className="scroll"><table className="v8-nums"><thead><tr><th>Number</th><th className="r">Value</th><th>Sources</th><th>Block</th><th>Age</th><th /></tr></thead>
          <tbody>{r.numbers.map((n) => <NumRow key={n.key} n={n} now={now} limit={r.limits.maxAgeS} />)}</tbody></table></div>
      </section>

      <Inventory r={r} snap={snap.data ?? null} fixture={fixture} now={now} />

      <div className="grid rise rise-3" style={{ marginTop: "3.5rem" }}>
        <section className="span-6">
          <p className="eyebrow">The ETH side</p><h2>The module's books, and the {r.symbol} in transit</h2>
          {r.module ? <dl className="kv kv--books">
            <dt>ETH-backed {r.symbol} (E)</dt><dd>{units(r.module.ethBackedUnits)}<small>= convertible {units(r.module.convertibleUnits)} + reserve {units(r.module.reserveUnits)} + in transit {units(r.module.transitUnits)}</small></dd>
            <dt>ETH in its pots</dt><dd>{eth(r.module.escrowWei)}<small>escrowBalance(); its own balance is {eth(r.module.balanceWei)} (revenue {eth(r.module.revenueWei)})</small></dd>
            <dt>The advance</dt><dd>{r.advance ? <>{eth(r.advance.advancedWei)}{r.advance.advancedAt ? `, ${ago(r.advance.advancedAt, now)}` : ""}<small>{r.advance.where}{r.advance.overdue ? " · open past 24 h" : ""}{r.advance.relay ? <> · Relay order {link(r.advance.relay.url, shortHash(r.advance.relay.orderId))}</> : null}</small></> : "none open"}</dd>
          </dl> : <p className="muted small">No floor module for this collection.</p>}
          {r.gas ? <dl className="kv kv--books" style={{ marginTop: "1rem" }}>
            <dt>The custodian's ETH on Ethereum</dt><dd>{eth(r.gas.custodianWei)}<small>earmarked for arrived buys {eth(r.gas.earmarkedWei)} · leftovers {eth(r.gas.leftoverWei)} · the gas pot {eth(r.gas.potWei)} — it backs no token; the earmarks and leftovers are the canister's records</small></dd>
          </dl> : null}
        </section>
        <section className="span-6">
          <p className="eyebrow">The canister's own records</p><h2>Its word, labelled: it explains, it never turns anything green</h2>
          <p className="small muted">{r.canisterNote}</p>
          {r.canister ? <>
            <ul className="list list--tight small">
              {r.canister.buys.filter((b) => b.phase !== "Settled").map((b) => <li key={b.id}>Buy {b.id}: <strong>{b.phase}</strong>, {eth(b.advancedWei)} · {b.note}</li>)}
              <li>{r.canister.buys.filter((b) => b.phase === "Settled").length} settled buys; {r.canister.deposits.length} holders' deposits minted; {r.canister.deliveries.length} deliveries under way</li>
              {r.canister.caps ? <li>Custody caps: {r.canister.caps.itemsHeld} of {r.canister.caps.maxItems} items ({r.canister.caps.itemsInFlight} in flight), {eth(r.canister.caps.valueWei)} of {eth(r.canister.caps.maxValueWei)}</li> : null}
            </ul></> : null}
          <p className="eyebrow" style={{ marginTop: "1.5rem" }}>Not in the books</p>
          {r.strays.length ? <ul className="list list--tight small">{r.strays.map((x) => <li key={x.tokenId + x.tx}><span className="mono">#{x.tokenId}</span> from <span className="mono">{x.from.slice(0, 10)}…</span>, {ago(x.at, now)}: {x.state}{x.overdue ? <span className="warn"> · past its timeout</span> : null}<br /><span className="muted">never counted as backing</span></li>)}</ul>
            : <p className="small muted">No token of the collection is at the custodian outside the wrapper's books.</p>}
        </section>
      </div>
    </>
  );
}

function NumRow({ n, now, limit }: { n: Num; now: number; limit: number }) {
  const old = n.at !== null && now - n.at > limit;
  const isWei = ["supply", "E", "C", "R", "T", "escrow", "moduleBalance", "custodianEth"].includes(n.key);
  const v = n.value === null ? "?" : isWei ? (["escrow", "moduleBalance", "custodianEth"].includes(n.key) ? eth(n.value) : units(n.value)) : /^\d{5,}$/.test(n.value) ? Number(n.value).toLocaleString("en-US") : n.value;
  return (
    <tr>
      <td>{n.label}{n.note ? <><br /><span className="small muted">{n.note}</span></> : null}</td>
      <td className="r num">{v}</td>
      <td className="small">{n.sources.join(", ")}</td>
      <td className="small num">{n.block?.toLocaleString("en-US") ?? "—"}</td>
      <td className={`small num${old ? " warn" : ""}`}>{n.at === null ? "—" : ago(n.at, now)}</td>
      <td>{n.ok === true && !old ? <span className="good">✓</span> : n.ok === false ? <span className="bad">✗</span> : <span className="warn">?</span>}</td>
    </tr>
  );
}

function Inventory({ r, snap, fixture, now }: { r: EthReserveReport; snap: MetadataSnapshot | null; fixture: boolean; now: number }) {
  const ranks = useMemo(() => (snap ? rarityRanks(snap.tokens) : null), [snap]);
  const counts = useMemo(() => (snap ? traitCounts(snap.tokens) : null), [snap]);
  const tokens = useMemo(() => new Map(snap?.tokens.map((t) => [t.tokenId, t]) ?? []), [snap]);
  return (
    <section className="block rise rise-3">
      <p className="eyebrow">The inventory</p>
      <h2>{r.items.length} NFT{r.items.length === 1 ? "" : "s"} in the vault{fixture ? " (invented ids; the links are text)" : ""}</h2>
      <p className="small muted v8-measure">Each NFT's owner at the finalized block on every provider, and the transaction that brought it in: final, successful, the transfer to the custodian, and a Seaport fill (a purchase) or the holder's own claim naming Robinhood Chain and this wrapper (a deposit). {snap ? `Rarity ranks are OpenRarity's method over the metadata snapshot (${snap.tokens.length.toLocaleString("en-US")} tokens), reproducible from it.` : ""}</p>
      <div className="scroll"><table className="v8-inv">
        <thead><tr><th>NFT</th><th>Rank</th><th>Traits</th><th>Arrived</th><th>Owner at the finalized block</th></tr></thead>
        <tbody>{r.items.map((it) => <InvRow key={it.itemId} it={it} rank={ranks?.get(it.tokenId)?.rank ?? null} of={snap?.tokens.length ?? 0} traits={tokens.get(it.tokenId)?.traits ?? null} counts={counts} custodian={r.custodian} fixture={fixture} now={now} />)}</tbody>
      </table></div>
    </section>
  );
}

function InvRow({ it, rank, of, traits, counts, custodian, fixture, now }: { it: EthItem; rank: number | null; of: number; traits: Record<string, string> | null; counts: Map<string, Map<string, number>> | null; custodian: string; fixture: boolean; now: number }) {
  const link = (url: string | null, text: string) => (fixture || !url ? <span className="mono break">{text}</span> : <a className="mono break" href={url} target="_blank" rel="noreferrer noopener">{text}</a>);
  return (
    <tr>
      <td>{link(it.nftUrl, `#${it.tokenId}`)}{it.promisedTo !== null ? <><br /><span className="small muted">drawn by ticket {it.promisedTo}, awaiting delivery</span></> : null}</td>
      <td className="num small">{rank ? `${rank} / ${of}` : "—"}</td>
      <td className="small">{traits ? Object.entries(traits).sort(([a], [b]) => a.localeCompare(b)).map(([k, v]) => <span key={k} className="v8-trait" title={counts ? `${counts.get(k)?.get(v) ?? 0} of ${of} tokens` : undefined}>{v}</span>) : "—"}</td>
      <td className="small">{it.how === "purchase" ? "bought" : "a holder's deposit"}{it.mintedAt ? `, ${ago(it.mintedAt, now)}` : ""}<br />{link(it.arrivalUrl, shortHash(it.arrivalTx))}<br />
        <span className={it.arrival.verdict === "ok" ? "good" : it.arrival.verdict === "bad" ? "bad" : "warn"}>{it.arrival.verdict === "ok" ? "checks out" : it.arrival.verdict === "bad" ? "does not check out" : "not checked"}</span>{it.arrival.pricePaidWei ? <span className="muted"> · paid {eth(it.arrival.pricePaidWei)}</span> : null}{it.cost ? <span className="muted"> · cost {eth(it.cost.costWei)}</span> : null}</td>
      <td className="small"><span className={`verdict verdict--${it.verdict}`}>{it.verdict.replace("-", " ")}</span>{it.reason ? <><br /><span className="muted">{it.reason}</span></> : null}
        {it.owners.map((o, i) => <span key={i} style={{ display: "block" }} className="muted">{o.source}: {o.owner ? (o.owner.toLowerCase() === custodian.toLowerCase() ? "the custodian" : o.owner) : o.reverted ? "reverts" : "no answer"}</span>)}</td>
    </tr>
  );
}

/** §4.1: "Check it yourself" — the same reads and the same verdict, from this browser, to providers the visitor picks. */
function SelfCheck({ r, fixture }: { r: EthReserveReport; fixture: boolean }) {
  const [ethRpcs, setEth] = useState<string>([...DEFAULT_ETH_RPCS].join("\n")), [homeRpcs, setHome] = useState<string>([...DEFAULT_HOME_RPCS].join("\n"));
  const [run, setRun] = useState<{ state: "idle" | "running" | "done" | "error"; report?: EthReserveReport; calls?: CallLog[]; error?: string }>({ state: "idle" });
  const recorded = useRecordedSelfCheck(r.slug, fixture), now = useNow(), src = useV8Source();
  // a fixture shows the providers its recording was made with (they do not exist), not the live defaults
  const hosts = recorded.data?.hosts;
  const ethShown = fixture && hosts ? hosts.eth.join("\n") : ethRpcs, homeShown = fixture && hosts ? hosts.home.join("\n") : homeRpcs;
  const urls = (s: string) => s.split(/\s+/).map((x) => x.trim()).filter(Boolean);
  const keyed = [...urls(ethRpcs), ...urls(homeRpcs)].filter(looksKeyed);
  const go = async () => {
    setRun({ state: "running" });
    try {
      const out = await checkReserves({ slug: r.slug, name: r.name, symbol: r.symbol, wrapper: r.wrapper, collection: r.collection, custodian: r.custodian, module: r.moduleAddress,
        homeRpcs: urls(homeRpcs), ethRpcs: urls(ethRpcs), fetch });
      setRun({ state: "done", report: out.report, calls: out.calls });
    } catch (e) { setRun({ state: "error", error: String((e as Error)?.message ?? e).slice(0, 200) }); }
  };
  const shown = fixture ? recorded.data : run.state === "done" ? { report: run.report!, calls: run.calls! } : null;
  const cli = `npm run -s verify -- reserves --wrapper ${r.wrapper} --collection ${r.collection} --custodian ${r.custodian}${r.moduleAddress ? ` --module ${r.moduleAddress}` : ""}`;
  return (
    <div className="panel v8-self">
      <p className="eyebrow">Check it yourself</p>
      <p className="small" style={{ marginTop: 0 }}>Re-runs every vault read from this browser — the wrapper's books and the module on Robinhood Chain, each NFT's owner and arrival on Ethereum — and the same verdict. No Rook server is involved beyond serving this page. Paste your own node's or wallet's RPC URLs to use them instead (keyless URLs only: a URL with a key in it is refused).</p>
      <div className="v8-rpcs">
        <label className="small">Ethereum providers<textarea rows={2} value={ethShown} onChange={(e) => setEth(e.target.value)} spellCheck={false} disabled={fixture} /></label>
        <label className="small">Robinhood Chain providers<textarea rows={2} value={homeShown} onChange={(e) => setHome(e.target.value)} spellCheck={false} disabled={fixture} /></label>
      </div>
      {keyed.length ? <p className="small bad">Refused (looks like it carries a key): {keyed.join(", ")}</p> : null}
      <div className="row"><button type="button" className="btn btn--primary btn--small" onClick={go} disabled={fixture || run.state === "running" || keyed.length > 0 || src.data?.kind !== "live"}>{run.state === "running" ? "Reading…" : "Check it yourself"}</button>
        {fixture ? <span className="small muted">Fixture: its chain does not exist, so below is a recording of this check run over the fixture's own providers.</span> : null}</div>
      {run.state === "error" ? <p className="small bad">{run.error}</p> : null}
      {shown ? <SelfResult report={shown.report} calls={shown.calls} now={now} /> : null}
      <p className="small muted" style={{ marginBottom: 0 }}>From a terminal, in the repository's indexer/: <code className="break">{cli}</code> (exits 0 green, 1 red, 2 unverified). By hand: every NFT links to its token page and its arrival transaction.</p>
    </div>
  );
}

function SelfResult({ report, calls, now }: { report: EthReserveReport; calls: CallLog[]; now: number }) {
  const s = servedStatus(report, now);
  return (
    <div className="v8-selfresult">
      <div className="row"><StatusPill status={s.status} /><span className="small">{report.equation.inCustody} in custody − {report.equation.ticketsInFlight} in flight + {units(report.equation.ethBackedUnits)} = {units(report.equation.backingUnits)} {report.equation.holds ? "=" : "vs"} {units(report.equation.supplyUnits)} supply</span></div>
      {s.reasons.length ? <ul className="reasons small">{s.reasons.slice(0, 4).map((x, i) => <li key={i}>{x}</li>)}</ul> : null}
      <details className="small"><summary>{calls.length} calls made ({calls.filter((c) => !c.ok).length} failed), each one repeatable</summary>
        <div className="scroll v8-calls"><table><thead><tr><th>Chain</th><th>Host</th><th>Call</th><th>Answer</th></tr></thead>
          <tbody>{calls.slice(0, 400).map((c, i) => <tr key={i} className={c.ok ? "" : "warn"}><td>{c.chain}</td><td className="mono">{c.host}</td><td className="mono break">{c.method} {c.params}</td><td className="mono break">{c.result}</td></tr>)}</tbody></table></div>
      </details>
      <p className="small muted" style={{ margin: 0 }}>Ethereum: {report.providers.eth.join(", ")} · Robinhood Chain: {report.providers.home.join(", ")}</p>
    </div>
  );
}
