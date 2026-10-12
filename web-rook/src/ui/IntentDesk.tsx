import { useEffect, useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useNow } from "../lib/env";
import { canister, stateOf, type IntentViewV, type OpenedV } from "../rook/sol/canister";
import { CANISTER_ON, useDeskStatus, useIntent, useQuoteBurn, useQuoteMint } from "../rook/sol/hooksCanister";
import { rpc, useBalances } from "../rook/sol/hooks";
import { DEMO } from "../rook/sol/demo";
import { DEMO_INTENT_IDS } from "../rook/sol/demoCanister";
import { amountProblem, depositInstructions, depositOf, intentsJson, INTENTS_KEY, KIND_WORD, mintMinOut, openedSummary, openKindFor, refusalCopy, rememberedIntents, statusCopy, STEPS, type IntentKind, type Tone } from "../rook/sol/intent";
import { parseUnits, short, solscan, units } from "../rook/sol/format";
import { simulateAndSend, type SendStep } from "../rook/sol/send";
import { SolWalletButton, useSolWallet } from "../rook/sol/wallet";
import { Pill } from "./bits";
import { Addr } from "./sol";

/**
 * The intent desk (docs/v9/canister-only-solana.md §6.1): open a mint, burn or redemption intent with the connected wallet as recipient,
 * get its deposit address, send the deposit from the wallet (any wallet can: the address is shown too), then watch the canister act on
 * it: finalized → signed → landed, or returned. Every number shown is the canister's (`open_intent`, `quote_*`, `intent_status`);
 * the page only draws them. Nothing is sent before the wallet signs; the canister never holds a key of yours.
 */
export function IntentDesk({ coll, wx, wxMint, kinds, coin, compact = false, destinationText = "an Ethereum address, a Robinhood Chain address, a taproot address or a Solana wallet, as the collection's chain needs" }: { coll: number; wx: string; wxMint: string | null; kinds: IntentKind[]; coin?: { mint: string; symbol: string; decimals: number }; compact?: boolean; destinationText?: string }) {
  const w = useSolWallet();
  const now = useNow();
  const qc = useQueryClient();
  const desk = useDeskStatus();
  const [kind, setKind] = useState<IntentKind>(kinds[0]);
  const [amount, setAmount] = useState("");
  const [destination, setDestination] = useState("");
  const [opened, setOpened] = useState<OpenedV | null>(null);
  const [id, setId] = useState<bigint | null>(null);
  const [busy, setBusy] = useState<"opening" | SendStep | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [sent, setSent] = useState<string | null>(null);
  const [mine, setMine] = useState<bigint[]>([]);
  useEffect(() => { setMine(w.address ? rememberedIntents(w.address, (k) => { try { return localStorage.getItem(k); } catch { return null; } }) : []); }, [w.address]);
  const remember = (i: bigint) => { if (!w.address) return; const next = [...mine.filter((x) => x !== i), i]; setMine(next); try { localStorage.setItem(INTENTS_KEY(w.address), intentsJson(next)); } catch { /* no storage */ } };

  const d = depositOf(kind, wx, coin?.symbol);
  const decimals = kind === "redeem_coin" ? (coin?.decimals ?? 6) : d.decimals;
  const amt = kind === "redeem_nft" ? 1_000_000_000n : parseUnits(amount, decimals);
  const mintOf = kind === "mint" ? null : kind === "redeem_coin" ? coin?.mint ?? null : wxMint;
  const bal = useBalances(w.address, mintOf ?? undefined);
  const balance = bal.data ? (kind === "mint" ? bal.data.sol : bal.data.coin) : null;
  const qm = useQuoteMint(kind === "mint" ? coll : null, amt);
  const qb = useQuoteBurn(kind === "burn" ? coll : null, amt);
  const problem = amountProblem(kind, amt, balance, wx);
  const view = useIntent(id);

  if (DEMO) return <DemoDesk kinds={kinds} kind={kind} setKind={setKind} wx={wx} destinationText={destinationText} now={now} />;
  if (!CANISTER_ON) return <section className="panel"><p className="eyebrow">Mint, burn, redeem</p><p className="small muted" style={{ margin: 0 }}>This build names no Rook canister yet (<span className="mono">src/config/canister.ts</span>): the desk opens once it is installed. Minting {wx} with SOL, burning it for SOL and redeeming it for an NFT all go through the canister's intents: your own deposit address per request, acted on once the deposit is final, or returned.</p></section>;

  const reset = () => { setOpened(null); setId(null); setErr(null); setSent(null); setBusy(null); };
  const paused = desk.data?.paused ?? false;
  const halted = desk.data?.halted.find(([c]) => c === coll)?.[1] ?? null;
  const coinRedeemOff = kind === "redeem_coin" && desk.data && !desk.data.coin_redeem;
  const open = async () => {
    if (!w.address) return;
    setErr(null); setBusy("opening");
    try {
      const minOut = kind === "mint" ? mintMinOut(qm.data ? { Mint: qm.data } : null, amt ?? 0n) : kind === "burn" && qb.data ? (qb.data.net * 95n) / 100n : null;
      const k = openKindFor(kind, { coll, wallet: w.address, minOut, destination, coin: coin?.mint });
      const r = await (await canister()).open_intent(k);
      if ("Err" in r) { setErr(refusalCopy(r.Err)); setBusy(null); return; }
      setOpened(r.Ok); setId(r.Ok.id); remember(r.Ok.id);
    } catch (e) { setErr(String((e as Error).message ?? e).slice(0, 240)); }
    setBusy(null);
  };
  const send = async () => {
    if (!w.address || !opened || amt === null) return;
    setErr(null);
    try {
      const ixs = depositInstructions(kind, { wallet: w.address, address: opened.address, tokenAccount: opened.token_account.length ? opened.token_account[0] : null, mint: mintOf, amount: amt });
      const sig = await simulateAndSend(rpc(), { address: w.address, signTransaction: w.signTransaction, signAndSend: w.signAndSend }, ixs, { onStep: setBusy });
      setSent(sig);
      void qc.invalidateQueries({ queryKey: ["ic-intent", opened.id.toString()] });
      void qc.invalidateQueries({ queryKey: ["sol-balances"] });
    } catch (e) { setErr(String((e as Error).message ?? e).slice(0, 240)); }
    setBusy(null);
  };

  return (
    <section className={`panel ticket${compact ? " panel--tight" : ""}`} aria-labelledby={`desk-${coll}`}>
      <p className="eyebrow" id={`desk-${coll}`}>Mint, burn, redeem · with the canister</p>
      {kinds.length > 1 ? <div className="seg" role="tablist" aria-label="What to do" style={{ marginBottom: ".75rem" }}>{kinds.map((k) => <button key={k} type="button" role="tab" aria-pressed={kind === k} aria-selected={kind === k} onClick={() => { setKind(k); setAmount(""); reset(); }}>{KIND_WORD[k]}</button>)}</div> : null}
      {!w.address ? <p className="small">Connect a wallet: it is the recipient of what the intent pays, and where a deposit goes back. <SolWalletButton /></p> : null}
      {paused ? <p className="notice notice--warn small">The desk is paused by the operator: no new intent for now. Open deposits are still returned.</p> : null}
      {halted ? <p className="notice notice--bad small">This collection is stopped by an incident in the canister: {halted}. No intent until the operator clears it.</p> : null}
      {coinRedeemOff ? <p className="notice notice--warn small">Coin redemption intents are not enabled on the canister yet.</p> : null}
      {!opened ? (
        <>
          <p className="small muted" style={{ marginTop: 0 }}>{kind === "mint" ? <>SOL in, {wx} out at the module's price (the accepted floor plus the mint spread). Minimum {d.minText}; fee 0.0015 SOL per operation.</>
            : kind === "burn" ? <>{wx} in, SOL out at the burn price. Minimum {d.minText}; fee 0.0015 SOL.</>
            : kind === "redeem_nft" ? <>Exactly 1 {wx} in; one NFT of the collection delivered to the destination ({destinationText}, or <span className="mono">burn</span>). Fee 0.0015 SOL.</>
            : <>{coin?.symbol ?? "Coins"} in; their share of the coin's {wx} vault out, rounded down. Fee 0.0015 SOL.</>}</p>
          {kind === "redeem_nft" ? <div className="ticket__in" style={{ marginBottom: ".5rem" }}><input aria-label="Destination" placeholder="destination address, or burn" value={destination} onChange={(e) => setDestination(e.target.value)} style={{ textAlign: "left" }} /><span>to</span></div> : null}
          {kind !== "redeem_nft" ? <div className="ticket__in"><input inputMode="decimal" aria-label={`Amount of ${d.asset}`} placeholder="0.0" value={amount} onChange={(e) => setAmount(e.target.value)} /><span>{d.asset}</span></div> : null}
          {balance !== null && kind !== "redeem_nft" ? <p className="small muted" style={{ margin: ".25rem 0 0" }}>You hold {units(balance, decimals, 4)} {d.asset}.</p> : null}
          <div className="ticket__line"><span className="ticket__label">You get{kind === "mint" || kind === "burn" ? " (quote now)" : ""}</span><span className="ticket__value">
            {kind === "mint" ? (qm.data ? <>≈ {units(qm.data.units, 9, 4)} {wx}<small> at {units(qm.data.unit_price, 9, 3)} SOL per {wx}</small></> : qm.error ? <small className="warn">{String((qm.error as Error).message).slice(0, 80)}</small> : "—")
              : kind === "burn" ? (qb.data ? <>≈ {units(qb.data.net, 9, 4)} SOL<small> after the fee</small></> : qb.error ? <small className="warn">{String((qb.error as Error).message).slice(0, 80)}</small> : "—")
              : kind === "redeem_nft" ? <>1 NFT<small> by ticket</small></> : <>{wx}<small> pro rata of the vault</small></>}</span></div>
          {kind === "mint" && qm.data ? <div className="ticket__line"><span className="ticket__label">Price bound</span><span className="ticket__value">≥ 95 % of the quote<small> else your SOL comes back, less the fee</small></span></div> : null}
          <div className="ticket__go"><button type="button" className="btn btn--primary btn--block" disabled={!w.address || !!problem || !!busy || paused || !!halted || !!coinRedeemOff || (kind === "redeem_nft" && !destination.trim())} onClick={() => void open()}>{busy === "opening" ? "Opening…" : `Open a ${KIND_WORD[kind].toLowerCase()} intent`}</button></div>
          {w.address && problem && (kind !== "redeem_nft" || amount) ? <p className="small muted" style={{ margin: ".5rem 0 0" }}>{problem}</p> : null}
          {err ? <p className="small error" role="alert" style={{ margin: ".5rem 0 0" }}>{err}</p> : null}
          <p className="small muted" style={{ margin: ".75rem 0 0" }}>Opening is free and needs no signature. The canister gives this request its own deposit address; at most 3 open per wallet, 5 an hour.</p>
        </>
      ) : (
        <Opened opened={opened} kind={kind} wx={wx} amount={amt} sent={sent} busy={busy} err={err} canSend={!!w.address && amt !== null && !problem} onSend={() => void send()} onNew={() => { setAmount(""); reset(); }} view={view.data ?? null} now={now} />
      )}
      {mine.length > 0 && !opened ? <MyIntents ids={mine} current={id} wx={wx} now={now} onPick={(i) => { setId(i); setOpened(null); }} /> : null}
      {id && !opened && view.data ? <LiveStatus v={view.data} wx={wx} now={now} onClose={() => setId(null)} /> : null}
    </section>
  );
}

function Opened({ opened, kind, wx, amount, sent, busy, err, canSend, onSend, onNew, view, now }: { opened: OpenedV; kind: IntentKind; wx: string; amount: bigint | null; sent: string | null; busy: string | null; err: string | null; canSend: boolean; onSend: () => void; onNew: () => void; view: IntentViewV | null; now: number }) {
  const token = opened.token_account.length ? opened.token_account[0] : null;
  const to = kind === "mint" ? opened.address : token ?? opened.address;
  const [copied, setCopied] = useState(false);
  const copy = async () => { try { await navigator.clipboard.writeText(to); setCopied(true); setTimeout(() => setCopied(false), 1_500); } catch { /* no clipboard */ } };
  const final = view ? statusCopy(view, now, wx).final : false;
  return (
    <>
      <p className="small" style={{ marginTop: 0 }}>Intent <strong>#{opened.id.toString()}</strong> is open. {openedSummary(opened, kind, wx)}</p>
      <div className="ticket__line"><span className="ticket__label">{kind === "mint" ? "Send SOL to" : "Send to the token account"}</span><span className="ticket__value"><Addr id={to} /> <button type="button" className="btn btn--small btn--quiet" onClick={() => void copy()}>{copied ? "Copied" : "Copy"}</button></span></div>
      {kind !== "mint" ? <div className="ticket__line"><span className="ticket__label">Its owner (the intent's address)</span><span className="ticket__value"><Addr id={opened.address} /></span></div> : null}
      <div className="ticket__line"><span className="ticket__label">Minimum</span><span className="ticket__value">{kind === "mint" ? `${units(opened.min_deposit, 9, 4)} SOL` : kind === "redeem_coin" ? "any amount" : `${units(opened.min_deposit, 9, 4)} ${wx}`}</span></div>
      <div className="ticket__line"><span className="ticket__label">Expires</span><span className="ticket__value">{new Date(Number(opened.expires_at) * 1000).toISOString().replace("T", " ").slice(0, 16)} UTC</span></div>
      {!sent && !final ? (
        <div className="ticket__go">
          <button type="button" className="btn btn--primary btn--block" disabled={!canSend || !!busy} onClick={onSend}>{busy === "simulating" ? "Simulating…" : busy === "signing" ? "Sign in your wallet…" : busy === "sending" ? "Sending…" : busy === "confirming" ? "Confirming…" : `Send ${amount !== null ? (kind === "mint" ? `${units(amount, 9, 4)} SOL` : kind === "redeem_coin" ? "the coins" : `${units(amount, 9, 4)} ${wx}`) : ""} from this wallet`}</button>
          <p className="small muted" style={{ margin: ".5rem 0 0" }}>Or send from any wallet or exchange to the address above (no memo needed). The transaction is simulated before your wallet is asked.</p>
        </div>
      ) : null}
      {sent ? <p className="small" style={{ margin: ".75rem 0 0" }}>Deposit sent: <a className="mono" href={solscan("tx", sent)} target="_blank" rel="noreferrer noopener">{short(sent)} ↗</a>. The canister reads it once finalized.</p> : null}
      {err ? <p className="small error" role="alert" style={{ margin: ".5rem 0 0" }}>{err}</p> : null}
      {view ? <LiveStatus v={view} wx={wx} now={now} /> : <p className="small muted" style={{ margin: ".75rem 0 0" }}>Reading the intent…</p>}
      <p style={{ margin: ".75rem 0 0" }}><button type="button" className="btn btn--small btn--quiet" onClick={onNew}>{final ? "Open another" : "Start over (the intent stays open until it expires)"}</button></p>
    </>
  );
}

const TONE: Record<Tone, "good" | "warn" | "bad" | "live" | undefined> = { wait: undefined, work: "live", good: "good", warn: "warn", bad: "bad" };
/** The intent as the canister reports it: the step reached, the headline and detail for its state, every transaction with its explorer link. */
export function LiveStatus({ v, wx, now, onClose }: { v: IntentViewV; wx: string; now: number; onClose?: () => void }) {
  const s = statusCopy(v, now, wx);
  return (
    <div style={{ marginTop: "1rem" }} aria-live="polite">
      <ol className="steps" aria-label="Where the intent is">{STEPS.map((name, i) => <li key={name} aria-current={i + 1 === s.step ? "step" : undefined} className={i + 1 < s.step ? "done" : undefined}><button type="button" disabled>{name}</button></li>)}</ol>
      <p style={{ margin: 0 }}><span className="status" style={{ fontSize: "var(--t1)" }}>{s.headline}</span> <Pill tone={TONE[s.tone]}>{s.state}</Pill>{v.attention ? <> <Pill tone="bad">needs the operator</Pill></> : null}</p>
      <p className="small" style={{ margin: ".35rem 0 0" }}>{s.detail}</p>
      {v.txs.length ? (
        <div className="scroll" style={{ marginTop: ".5rem" }}><table className="tape small">
          <thead><tr><th>Step</th><th>Result</th><th>Signature</th></tr></thead>
          <tbody>{v.txs.map((t, i) => <tr key={i}><td>{t.op}{t.memo ? <small className="muted"> · {t.memo}</small> : null}</td><td className={/^landed/.test(t.result) ? "good" : "warn"}>{t.result}</td><td className="num">{t.signature ? <a className="mono" href={solscan("tx", t.signature)} target="_blank" rel="noreferrer noopener">{short(t.signature)} ↗</a> : "—"}</td></tr>)}</tbody>
        </table></div>
      ) : null}
      <p className="small muted" style={{ margin: ".5rem 0 0" }}>Intent #{v.id.toString()} · address <Addr id={v.address} /> · recipient <Addr id={v.recipient} />{onClose ? <> · <button type="button" className="btn btn--small btn--quiet" onClick={onClose}>Hide</button></> : null}</p>
    </div>
  );
}

function MyIntents({ ids, current, wx, now, onPick }: { ids: bigint[]; current: bigint | null; wx: string; now: number; onPick: (i: bigint) => void }) {
  return (
    <details style={{ marginTop: "1rem" }} open={false}>
      <summary className="small muted">Your intents in this browser ({ids.length})</summary>
      <ul className="small" style={{ margin: ".5rem 0 0", paddingLeft: "1.1rem" }}>{[...ids].reverse().map((i) => <li key={i.toString()}><IntentLine id={i} wx={wx} now={now} active={current === i} onPick={() => onPick(i)} /></li>)}</ul>
    </details>
  );
}
function IntentLine({ id, wx, now, active, onPick }: { id: bigint; wx: string; now: number; active: boolean; onPick: () => void }) {
  const v = useIntent(id);
  const s = v.data ? statusCopy(v.data, now, wx) : null;
  return <><button type="button" className="btn btn--small btn--quiet" onClick={onPick} aria-pressed={active}>#{id.toString()}</button> {v.data ? <>{KIND_WORD[v.data.kind as IntentKind] ?? v.data.kind} · {s?.headline} <Pill tone={TONE[s!.tone]}>{stateOf(v.data)}</Pill></> : v.isLoading ? "reading…" : <span className="muted">not on the canister (dropped after its grace)</span>}</>;
}

/** The desk under `?demo`: the same rules in words, and a sample intent of the chosen kind as the demo's canister reports it (a mint done,
 *  a burn landing, a redemption waiting on its deposit, a coin redemption done), drawn by the live desk's own status view. Opens nothing. */
function DemoDesk({ kinds, kind, setKind, wx, destinationText, now }: { kinds: IntentKind[]; kind: IntentKind; setKind: (k: IntentKind) => void; wx: string; destinationText: string; now: number }) {
  const sample = useIntent(DEMO_INTENT_IDS[kind]);
  const d = depositOf(kind, wx);
  return (
    <section className="panel ticket" aria-label="Mint, burn, redeem (demo)">
      <p className="eyebrow">Mint, burn, redeem · with the canister</p>
      {kinds.length > 1 ? <div className="seg" role="tablist" aria-label="What to do" style={{ marginBottom: ".75rem" }}>{kinds.map((k) => <button key={k} type="button" role="tab" aria-pressed={kind === k} aria-selected={kind === k} onClick={() => setKind(k)}>{KIND_WORD[k]}</button>)}</div> : null}
      <p className="small" style={{ marginTop: 0 }}><strong>Demo: nothing is opened or sent.</strong> On the live site you open an intent (free, no signature), get a deposit address of its own, send {kind === "mint" ? "SOL" : kind === "redeem_coin" ? "the coins" : wx} to it from any wallet ({d.minText}{kind === "redeem_nft" ? `; the NFT goes to ${destinationText}` : ""}; fee 0.0015 SOL), and watch the canister act on it once the deposit is final, or return it, less the fee, after 6 hours.</p>
      <p className="eyebrow" style={{ marginTop: "1rem" }}>A sample intent</p>
      {sample.data ? <LiveStatus v={sample.data} wx={wx} now={now} /> : <p className="small muted">Reading the sample…</p>}
    </section>
  );
}
