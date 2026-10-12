import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { href, useNow, useRoute } from "../lib/env";
import { collectionBySlug, COLLECTIONS, type SolCollection } from "../rook/sol/collections";
import { readConfigs } from "../rook/sol/board";
import { CANISTER_ON, useCoinsCollections, useOpening } from "../rook/sol/hooksCanister";
import { configOf, launchableOf, openingOf } from "../rook/sol/opening";
import { OpenCollection } from "../ui/OpenCollection";
import { thresholdOf, thresholdText, wxText } from "../rook/sol/graduation";
import { priceSol } from "../rook/sol/dbc";
import { DEMO } from "../rook/sol/demo";
import { loadFloors } from "../rook/sol/floors";
import { rpc, useBalances, useConfigs, useSolUsd } from "../rook/sol/hooks";
import {
  IMAGE_ACCEPT, STEPS, buyUnits, clearDraft, cleanTicker, coinProblems, cropHint, DESCRIPTION_MAX, draftBuyUnits, imageFileProblem, launchCosts,
  launchBuyFeeShort, launchBuyNote, launchBuyQuote, launchErrorText, launchHash, loadDraft, normalizeLink, parseLaunchRoute, prefillLinks, reachableStep, saveDraft, stepProblem, switchBuyUnit, withCollection, type Draft, type PickedImage, type Step,
} from "../rook/sol/launchWizard";
import { coinsText, floorLamportsOf, friendlyWx, graduateText, holdText, otherUnitText, progressLabel, solWithTokens, tokensText, tokOf, wxUnits } from "../rook/sol/firstBuy";
import { NATIVE_MINT } from "../rook/sol/keys";
import { ata, localKeypair } from "../rook/sol/pda";
import { SOL } from "../rook/sol/siteConfig";
import { solNum, solText, usd } from "../rook/sol/format";
import { boundBySimulation, launchIxs } from "../rook/sol/trade";
import { simulateAndSend } from "../rook/sol/send";
import { uploadCoin, uploadReady, UploadError, type Uploaded } from "../rook/sol/upload";
import { SolWalletButton, useSolWallet } from "../rook/sol/wallet";
import { firstBuyNote, LaunchCosts, LaunchGraduation } from "../ui/LaunchCosts";
import { Picture } from "../ui/Picture";
import { CollectionLogo } from "../ui/sol";
import "../styles/launch.css";

// every collection the canister has pinned (operator decision 2026-10-12; `launchableOf`: its `coins_collections` once read, the 38 the
// ceremony pins without one): a coin can be prepared on any; one that is not open yet is opened from the review step ("Open it":
// coins_request_open), since the launch transaction needs its DBC config. While the canister is still answering, any collection's slug
// is kept, so a link to a collection pinned after the ceremony (a Robinhood Chain one) is not dropped before the list arrives.
function useLaunchable(): { list: readonly SolCollection[]; known: (slug: string) => boolean } {
  const cols = useCoinsCollections();
  const list = launchableOf(cols.data);
  const waiting = CANISTER_ON && cols.isLoading;
  return { list, known: (slug: string) => (waiting ? COLLECTIONS : list).some((c) => c.slug === slug) };
}
const store = (): Storage | null => { try { return window.sessionStorage; } catch { return null; } };
const SLIPPAGE_BPS = 300;

type Phase = "upload" | "create" | "confirm" | "done";
const PHASES: [Phase, string][] = [["upload", "Uploading image"], ["create", "Creating coin"], ["confirm", "Confirming"]];

/**
 * Launch a coin on Solana in three steps: pick the collection, describe the coin (name, ticker, image, optional description, links and
 * first buy), review and launch. The image and the metadata JSON are hosted by the site's server (POST /api/upload); the launch is ONE
 * transaction: Meteora DBC `initialize_virtual_pool_with_spl_token` under the collection's Rook config (creator = your wallet, mint = a
 * fresh key this page signs with after your wallet, never stored) and the optional first buy. Simulated before your wallet is asked.
 * Before day 1, and in demo mode, every step works and the Launch button says why it can't send. A collection opens when someone first
 * launches on it (operator decision 2026-10-12): until it is open the review step offers "Open it" and waits for the canister.
 */
export function SolLaunch(_props: { slug?: string }) {
  const route = useRoute();
  const { known } = useLaunchable();
  const parsed = parseLaunchRoute(route, known);
  const initial = useMemo(() => loadDraft(store()), []);
  const [draft, setDraft] = useState<Draft>(() => (parsed.slug ? withCollection(initial.draft, parsed.slug) : initial.draft));
  const [image, setImage] = useState<PickedImage | null>(initial.image);
  const step = reachableStep(parsed.step, { ...draft, collection: parsed.slug ?? draft.collection }, image, known);
  const headRef = useRef<HTMLHeadingElement>(null);
  const firstRender = useRef(true);

  // the URL names the collection: it wins over a stored draft
  useEffect(() => { if (parsed.slug && parsed.slug !== draft.collection) setDraft((d) => withCollection(d, parsed.slug!)); }, [parsed.slug]); // eslint-disable-line react-hooks/exhaustive-deps
  // a URL past a missing field lands on the furthest step the draft allows
  useEffect(() => { if (step !== parsed.step && (parsed.slug || draft.collection)) location.replace(launchHash(parsed.slug ?? draft.collection, step)); }, [step, parsed.step]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { saveDraft(store(), draft, image); }, [draft, image]);
  useEffect(() => { if (firstRender.current) { firstRender.current = false; return; } headRef.current?.focus(); }, [step]);

  const go = (s: Step, slug = draft.collection) => { location.hash = launchHash(slug, s); };
  const update = (patch: Partial<Draft>) => setDraft((d) => ({ ...d, ...patch }));
  const problem = stepProblem(step, draft, image, known);
  const col = collectionBySlug(draft.collection);

  return (
    <>
      <header className="pagehead rise launch__head"><p className="eyebrow">Launch a coin</p><h1>Launch a coin</h1>
        <p className="lede">Pick a collection, describe your coin, launch. One transaction on Solana; half of what your coin raises at graduation backs the collection.</p></header>
      {DEMO ? <p className="notice small launch__note" role="note"><strong>Demo.</strong> Every step works; nothing is uploaded and nothing is sent.</p>
        : !SOL.ready && !CANISTER_ON ? <p className="notice notice--warn small launch__note" role="note"><strong>Before day 1.</strong> You can prepare your coin now; launching opens once Rook's canister is live on Solana.</p> : null}
      <nav aria-label="Launch steps" className="launch__steps rise rise-1">
        <ol>{STEPS.map(([n, label]) => {
          const state = n === step ? "current" : n < step ? "done" : "todo";
          return <li key={n} className={`launch__stepi launch__stepi--${state}`} aria-current={n === step ? "step" : undefined}>
            {state === "done" ? <button type="button" onClick={() => go(n)}><span className="launch__num" aria-hidden="true">✓</span>{label}<span className="vh"> (done, go back)</span></button>
              : <span><span className="launch__num" aria-hidden="true">{n}</span>{label}</span>}
          </li>;
        })}</ol>
      </nav>
      <section className="launch__body rise rise-1" aria-labelledby="launch-step-title">
        <h2 id="launch-step-title" ref={headRef} tabIndex={-1} className="launch__title">{step === 1 ? "Pick a collection" : step === 2 ? `Your coin${col ? ` on ${col.name}` : ""}` : "Review & launch"}</h2>
        {step === 1 ? <CollectionStep value={draft.collection} onPick={(slug) => { setDraft((d) => withCollection(d, slug)); go(2, slug); }} />
          : step === 2 ? <CoinStep draft={draft} update={update} image={image} setImage={setImage} />
          : <ReviewStep draft={draft} image={image} onEdit={() => go(2)} />}
        {step < 3 ? <div className="launch__nav">
          {step > 1 ? <button type="button" className="btn" onClick={() => go((step - 1) as Step)}>Back</button> : <span />}
          <span className="launch__navright">
            {problem && step === 2 ? <span className="small muted" aria-live="polite">{problem}</span> : null}
            <button type="button" className="btn btn--primary" disabled={!!problem} onClick={() => go((step + 1) as Step)}>Next</button>
          </span>
        </div> : <div className="launch__nav"><button type="button" className="btn" onClick={() => go(2)}>Back</button><span /></div>}
      </section>
    </>
  );
}

// ── 1 · Collection ─────────────────────────────────────────────────────────────────────────────────────────────────────────────────
function CollectionStep({ value, onPick }: { value: string; onPick: (slug: string) => void }) {
  const [q, setQ] = useState("");
  const { list } = useLaunchable();
  const floors = useQuery({ queryKey: ["launch-floors", list.length], staleTime: Infinity, retry: false, queryFn: () => loadFloors(list) });
  const cols = useCoinsCollections();
  const t = q.trim().toLowerCase();
  const shown = list.filter((c) => !t || c.name.toLowerCase().includes(t) || c.slug.includes(t) || c.wxSymbol.toLowerCase().includes(t));
  return (
    <div className="stack">
      <label className="launch__search"><span className="vh">Search collections</span>
        <input type="search" placeholder="Search collections" value={q} autoComplete="off" spellCheck={false} onChange={(e) => setQ(e.target.value)} /></label>
      <div className="launch__grid" role="list">{shown.map((c) => {
        const f = floors.data?.[c.slug];
        return <div role="listitem" key={c.slug}>
          <button type="button" className={`launch__card${value === c.slug ? " launch__card--on" : ""}`} aria-pressed={value === c.slug} onClick={() => onPick(c.slug)}>
            <CollectionLogo c={c} size={56} />
            <span className="launch__cardname">{c.name}</span>
            <span className="launch__cardtier" title={`Graduates at ${thresholdText(c.slug)}; half of it (${wxText(thresholdOf(c.slug)!.backingWx)} ${c.wxSymbol}) goes into the coin's vault: more than one ${c.name} NFT's worth`}>{(() => { const g = graduateText(tokOf(c.slug), thresholdOf(c.slug)!.wx); return <>{g.main} to graduate{g.label ? <small className="launch__cardwx">{g.label}</small> : null}</>; })()}</span>
            <span className="launch__cardfloor" title={f ? `Recorded floor, ${new Date(f.at * 1000).toLocaleDateString()}: not a live price` : undefined}>{f ? `Floor ~${f.text}` : floors.isLoading ? "Floor …" : c.chain}</span>
            {CANISTER_ON && cols.data && openingOf(cols.data, c.wxSymbol).kind !== "open" ? <span className="small muted">Not open yet: be the first</span> : null}
          </button>
        </div>;
      })}</div>
      {!shown.length ? <p className="muted small">No collection matches “{q}”.</p> : null}
      <p className="small muted">Every collection Rook can buy, hold and price today is here: Ethereum, Solana, Bitcoin Frogs and the other Ordinals (the Robinhood Chain collections appear once they are added: see <a href={href("collections")}>the collections</a>). One nobody has launched on yet opens the first time someone asks: you can ask from the last step. Rook pays for it and opens at most 8 a day, in the order asked. Floors are recorded marketplace figures, not live prices. Each collection's coins are priced in its wrapped NFT token and graduate at the collection's own threshold: a whole number of those tokens, at least 3, chosen so the raise was worth about 85 SOL at the day-1 floor (more for the priciest collections). Half of it goes into the coin's vault: more than one NFT's worth from the first graduation.</p>
    </div>
  );
}

// ── 2 · Your coin ──────────────────────────────────────────────────────────────────────────────────────────────────────────────────
function CoinStep({ draft, update, image, setImage }: { draft: Draft; update: (p: Partial<Draft>) => void; image: PickedImage | null; setImage: (i: PickedImage | null) => void }) {
  const [touched, setTouched] = useState<Record<string, boolean>>({});
  const [imgErr, setImgErr] = useState<string | null>(null);
  const [drag, setDrag] = useState(false);
  const pre = prefillLinks(draft.collection);
  const hasPrefill = !!(pre.website || pre.twitter);
  const p = coinProblems(draft, image);
  const show = (k: keyof typeof p) => (touched[k] && p[k] ? <span className="launch__err" id={`err-${k}`}>{p[k]}</span> : null);
  const touch = (k: string) => () => setTouched((t) => ({ ...t, [k]: true }));
  const aria = (k: keyof typeof p) => (touched[k] && p[k] ? { "aria-invalid": true as const, "aria-describedby": `err-${k}` } : {});

  const pick = async (file: File | undefined) => {
    if (!file) return;
    setImgErr(null);
    const head = new Uint8Array(await file.slice(0, 16).arrayBuffer());
    const bad = imageFileProblem({ type: file.type, size: file.size }, head);
    if (bad) { setImgErr(bad); return; }
    const dataUrl = await new Promise<string>((res, rej) => { const r = new FileReader(); r.onload = () => res(String(r.result)); r.onerror = () => rej(r.error); r.readAsDataURL(file); });
    const dims = await new Promise<{ w: number | null; h: number | null }>((res) => { const i = new Image(); i.onload = () => res({ w: i.naturalWidth, h: i.naturalHeight }); i.onerror = () => res({ w: null, h: null }); i.src = dataUrl; });
    // the stored data URL carries the sniffed type, whatever the file claimed
    const sniffedType = dataUrl.slice(5, dataUrl.indexOf(";"));
    setImage({ dataUrl, type: sniffedType, size: file.size, width: dims.w, height: dims.h, name: file.name });
    setTouched((t) => ({ ...t, image: true }));
  };
  // the first buy as typed (in SOL by default, or in wX) and in wX, the amount the transaction uses (firstBuy.ts)
  const typed = buyUnits(draft.buy);
  const lamports = draftBuyUnits(draft);
  const quoteSym = collectionBySlug(draft.collection)?.wxSymbol ?? "SOL";
  const floor = floorLamportsOf(draft.collection);
  const unit = floor ? draft.buyUnit : "wX";
  const tok = tokOf(draft.collection);
  const other = typed ? otherUnitText(typed, unit, tok) : null;
  const tokName = tok.name ? `${tok.name} tokens` : quoteSym;

  return (
    <div className="launch__two">
      <div className="stack">
        <div className="launch__row2">
          <label className="field">Name<input value={draft.name} maxLength={32} autoComplete="off" placeholder="Frog Coin" onChange={(e) => update({ name: e.target.value })} onBlur={touch("name")} {...aria("name")} />{show("name")}</label>
          <label className="field">Ticker<span className="launch__ticker"><span aria-hidden="true">$</span><input className="mono" value={draft.symbol} maxLength={10} autoComplete="off" autoCapitalize="characters" placeholder="FROG" onChange={(e) => update({ symbol: cleanTicker(e.target.value) })} onBlur={touch("symbol")} {...aria("symbol")} /></span>{show("symbol")}</label>
        </div>

        <div className="field">
          <span id="img-label">Image</span>
          <label className={`launch__drop${drag ? " launch__drop--over" : ""}${image ? " launch__drop--has" : ""}`}
            onDragOver={(e) => { e.preventDefault(); setDrag(true); }} onDragLeave={() => setDrag(false)}
            onDrop={(e) => { e.preventDefault(); setDrag(false); void pick(e.dataTransfer.files?.[0]); }}>
            <input className="vh" type="file" accept={IMAGE_ACCEPT} aria-labelledby="img-label" aria-describedby="img-hint" onChange={(e) => { void pick(e.target.files?.[0]); e.target.value = ""; }} />
            {image ? <img src={image.dataUrl} alt="Your coin's image" className="launch__dropimg" /> : <span className="launch__dropicon" aria-hidden="true">+</span>}
            <span className="launch__droptext">
              <strong>{image ? "Change image" : "Drop an image here, or choose a file"}</strong>
              <small id="img-hint">PNG, JPG, GIF or WebP, up to 2 MB. Square works best.</small>
            </span>
          </label>
          {imgErr ? <span className="launch__err" role="alert">{imgErr}</span> : image ? (cropHint(image.width, image.height) ? <span className="small warn">{cropHint(image.width, image.height)}</span> : null) : show("image")}
        </div>

        <label className="field">Description <small>(optional)</small>
          <textarea rows={3} maxLength={DESCRIPTION_MAX} value={draft.description} placeholder="What's your coin about?" onChange={(e) => update({ description: e.target.value })} />
          <small className="launch__count">{draft.description.length}/{DESCRIPTION_MAX}</small></label>

        <div className="launch__links" role="group" aria-labelledby="links-label">
          <span id="links-label">Links <small>(optional)</small></span>
          {/* Website and X start as the collection's official links (launchWizard.ts withCollection); the creator can change or clear them */}
          {hasPrefill ? <small className="muted" style={{ display: "block", marginTop: ".2rem" }}>Prefilled with the collection's official links. You can change them.</small> : null}
          <label className="field">Website<input value={draft.website} inputMode="url" placeholder="example.com" autoComplete="off" onChange={(e) => update({ website: e.target.value })} onBlur={touch("website")} {...aria("website")} />{show("website")}</label>
          <label className="field">X<input value={draft.twitter} placeholder="@handle" autoComplete="off" onChange={(e) => update({ twitter: e.target.value })} onBlur={touch("twitter")} {...aria("twitter")} />{show("twitter")}</label>
          <label className="field">Telegram<input value={draft.telegram} placeholder="t.me/yourgroup" autoComplete="off" onChange={(e) => update({ telegram: e.target.value })} onBlur={touch("telegram")} {...aria("telegram")} />{show("telegram")}</label>
        </div>

        <div className="field">
          <span><label htmlFor="launch-buy">Buy first <small>(optional)</small></label>
            {floor ? <span className="launch__buyunit" role="group" aria-label="Type the amount in">
              {(["SOL", "wX"] as const).map((u) => <button key={u} type="button" aria-pressed={unit === u} onClick={() => update(switchBuyUnit(draft, u))}>{u === "SOL" ? "SOL" : quoteSym}</button>)}
            </span> : null}</span>
          <span className="ticket__in"><input id="launch-buy" inputMode="decimal" placeholder="0" value={draft.buy} autoComplete="off" onChange={(e) => update({ buy: e.target.value })} onBlur={touch("buy")} aria-describedby="launch-buy-other" {...aria("buy")} /><span>{unit === "SOL" ? "SOL" : quoteSym}</span></span>
          <span id="launch-buy-other" aria-live="polite">{other && !p.buy ? <span className="launch__other">{other}</span> : null}
            {lamports && lamports > 0n && !p.buy ? <FirstBuyGets slug={draft.collection} amount={lamports} /> : null}</span>
          {show("buy")}
          {lamports && lamports > 0n ? <LaunchBuyNotice slug={draft.collection} /> : <small>Be your coin's first buyer in the same transaction, paying in {tokName}{tok.name ? ` (${quoteSym})` : ""} from your wallet{floor ? `. Type the amount in SOL or in ${quoteSym}: 1 token is 1 ${tok.name} NFT, about the recorded floor` : ""}. Leave empty to skip: after launch anyone can buy with SOL on the coin's page.</small>}
          {lamports && lamports > 0n && floor ? <small className="muted">Paid as {tokensText(friendlyWx(tok, lamports))} from your wallet. The SOL figure is an estimate at the recorded floor, not a live price.</small> : null}
        </div>
      </div>
      <aside className="launch__side" aria-label="Preview">
        <p className="eyebrow">Preview</p>
        <BoardPreview draft={draft} image={image} />
      </aside>
    </div>
  );
}

/** The collection's DBC config: the site config's, else the canister's once the collection is Open (`configOf`); read from the chain. */
function useCfg(slug: string) {
  const cfgs = useConfigs();
  const row = SOL.collections.find((c) => c.slug === slug);
  const col = collectionBySlug(slug);
  const { opening } = useOpening(col?.wxSymbol);
  const config = configOf(row?.config, opening);
  const known = config ? cfgs.data?.get(config) ?? null : null;
  const own = useQuery({ queryKey: ["launch-config", config], enabled: !!config && !known && !DEMO, staleTime: Infinity, retry: 1,
    queryFn: async () => (await readConfigs(rpc(), [{ config: config!, slug, label: col?.name ?? slug }])).get(config!) ?? null });
  return { row, config, opening, dbc: known ?? own.data ?? null };
}

/** What a first buy of `amount` (the curve's quote, wX) gets on the fresh curve: its starting price, at the fee the launch's own buy
 *  pays (launchWizard.ts `launchBuyQuote`: the normal fee on a config with the creator-first-buy flag). */
function FirstBuyGets({ slug, amount }: { slug: string; amount: bigint }) {
  const { dbc } = useCfg(slug);
  const now = useNow();
  const est = useMemo(() => (dbc ? launchBuyQuote(dbc, amount, BigInt(now)) : null), [dbc, amount, now]);
  if (!dbc || !est) return null;
  const fee = friendlyWx(tokOf(slug), est.fee);
  return <small className="launch__gets">{`Gets ${coinsText(est.amountOut, dbc.tokenDecimal, dbc.fixedSupply ? dbc.preMigrationTokenSupply : null)}, after a fee of ${fee.sol ?? fee.label}`}</small>;
}

/** The note on the first buy's fee (launchWizard.ts `launchBuyNote`): informational when the config gives the launch's buy the normal fee. */
function LaunchBuyNotice({ slug }: { slug: string }) {
  const { dbc } = useCfg(slug);
  if (!dbc) return null;
  const note = launchBuyNote(dbc);
  return <p className={`notice ${dbc.firstSwapMinFee ? "notice--info" : "notice--warn"} small launch__warn`} role="note"><strong>{note.headline}</strong> {note.detail}</p>;
}

/** The coin as the board will list it: picture, ticker, name, collection, market cap (in the collection's wX, and in dollars with
 *  the wX valued at the day-1 floor), progress to its collection's threshold, age. */
function BoardPreview({ draft, image }: { draft: Draft; image: PickedImage | null }) {
  const col = collectionBySlug(draft.collection);
  const t = thresholdOf(draft.collection);
  const { dbc } = useCfg(draft.collection);
  const px = useSolUsd();
  const mcap = dbc && dbc.fixedSupply ? priceSol(dbc.sqrtStartPrice, dbc.tokenDecimal) * (Number(dbc.preMigrationTokenSupply) / 10 ** dbc.tokenDecimal) : null;
  const name = draft.name.trim() || "Your coin", sym = draft.symbol.trim() || "TICKER";
  return (
    <div className="launch__preview">
      <table className="board__table">
        <thead><tr><th>Coin</th><th className="col-nest">Collection</th><th className="r">Market cap</th><th title={progressLabel(draft.collection)}>Progress to {graduateText(tokOf(draft.collection), thresholdOf(draft.collection)?.wx ?? 0).main}</th><th className="r">Age</th></tr></thead>
        <tbody><tr className="board__row">
          <td><span className="board__coin"><Picture src={image?.dataUrl ?? null} seed={sym} name={name} size={32} round="50%" /><span><strong>{sym}</strong><small>{name}</small></span></span></td>
          <td className="small col-nest">{col?.name ?? "—"}</td>
          <td className="r num">{mcap !== null ? `${solNum(mcap, 2)} ${col?.wxSymbol ?? "SOL"}` : "—"}<br /><small className="muted" title={t ? `${col?.wxSymbol} valued at the day-1 floor, ${t.floorSol} SOL` : undefined}>{mcap !== null && px.data ? `≈ ${usd(mcap * (t?.floorSol ?? 1) * px.data.price)}` : ""}</small></td>
          <td><span className="progress"><i style={{ width: "0%" }} /></span><small className="small muted">0%</small></td>
          <td className="r num muted">now</td>
        </tr></tbody>
      </table>
      {draft.description.trim() ? <p className="small muted launch__desc">{draft.description.trim()}</p> : null}
    </div>
  );
}

// ── 3 · Review & launch ────────────────────────────────────────────────────────────────────────────────────────────────────────────
function ReviewStep({ draft, image, onEdit }: { draft: Draft; image: PickedImage | null; onEdit: () => void }) {
  const w = useSolWallet();
  const { config, dbc } = useCfg(draft.collection);
  const col = collectionBySlug(draft.collection)!;
  const t = thresholdOf(draft.collection)!;
  // the first buy is in the curve's quote: the collection's wX, from the wallet's wX account (a SOL-quoted curve would take SOL)
  const quoteIsSol = !!dbc && dbc.quoteMint === NATIVE_MINT;
  const lamports = draftBuyUnits(draft) ?? 0n;
  const tok = tokOf(draft.collection);
  const costs = launchCosts(lamports, quoteIsSol ? "SOL" : "wX");
  const bal = useBalances(DEMO ? null : w.address, undefined, quoteIsSol ? null : dbc?.quoteMint);
  const ready = useQuery({ queryKey: ["upload-ready"], enabled: !DEMO, staleTime: 60_000, retry: false, queryFn: () => uploadReady() });
  const [phase, setPhase] = useState<Phase | null>(null);
  const [signing, setSigning] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [demoDone, setDemoDone] = useState(false);
  const cache = useRef<{ key: string; up: Uploaded } | null>(null);

  const balance = bal.data?.sol ?? null;
  const short = balance !== null && balance < costs.total;
  const shortWx = bal.data && costs.firstBuyWx > 0n && bal.data.wx < costs.firstBuyWx;
  const why: string | null = DEMO ? null
    : !config && !CANISTER_ON ? "Launching opens once Rook's canister is live on Solana. Everything above is ready to go."
    : !config ? `${col.name} isn't open yet: open it above, then launch once it is.`
    : ready.data === false ? "Image upload isn't set up yet, so coins can't be launched from this site right now."
    : !w.address ? null
    : short ? `Your wallet has ${solText(balance!, 4)} SOL; this launch needs about ${solText(costs.total, 4)} SOL. Add SOL${quoteIsSol ? " or lower the first buy" : ""}.`
    : shortWx ? `The first buy needs ${solWithTokens(tok, costs.firstBuyWx)} in your wallet. ${holdText(tok, bal.data!.wx)}. Lower it or leave it empty (you can buy with SOL on your coin's page right after launch), or mint ${col.wxSymbol} first.`
    : null;
  const busy = phase !== null && phase !== "done";

  const fields = () => ({
    name: draft.name.trim(), symbol: draft.symbol.trim(), description: draft.description.trim(), collection: draft.collection,
    website: normalizeLink("website", draft.website) ?? "", twitter: normalizeLink("twitter", draft.twitter) ?? "", telegram: normalizeLink("telegram", draft.telegram) ?? "",
  });

  const launchDemo = async () => {
    setErr(null); setDemoDone(false);
    for (const p of ["upload", "create", "confirm"] as Phase[]) { setPhase(p); await new Promise((r) => setTimeout(r, 700)); }
    setPhase("done"); setDemoDone(true);
  };
  const launch = async () => {
    if (DEMO) return void launchDemo();
    if (!image || !w.address || !config) return;
    setErr(null);
    try {
      setPhase("upload");
      const f = fields();
      const key = JSON.stringify([f, image.dataUrl.length, image.dataUrl.slice(-64)]);
      const up = cache.current?.key === key ? cache.current.up : await uploadCoin(image.dataUrl, f);
      cache.current = { key, up };
      setPhase("create");
      const mint = localKeypair();
      const ixs = (minOut: bigint, unwrap: boolean) => launchIxs({ user: w.address!, config: config!, mint: mint.address, quoteMint: dbc?.quoteMint, name: f.name, symbol: f.symbol, uri: up.uri, firstBuy: lamports, minOut }, unwrap);
      const minOut = lamports > 0n ? (await boundBySimulation(rpc(), w.address, ata(w.address, mint.address), ixs, SLIPPAGE_BPS)).minOut : 0n;
      await simulateAndSend(rpc(), { ...w, address: w.address }, ixs(minOut, true), { extra: [mint], onStep: (s) => { setSigning(s === "signing"); setPhase(s === "confirming" ? "confirm" : "create"); } });
      setPhase("done");
      clearDraft(store());
      location.hash = `#/coin/${mint.address}`;
    } catch (e) {
      setPhase(null); setSigning(false);
      setErr(e instanceof UploadError ? e.message : launchErrorText(e, { total: costs.total, balance }));
    }
  };

  return (
    <div className="launch__two">
      <div className="stack">
        <div>
          <p className="eyebrow">On the board</p>
          <BoardPreview draft={draft} image={image} />
          <p className="small"><button type="button" className="launch__link" onClick={onEdit}>Edit your coin</button></p>
        </div>
        <section className="panel panel--tight launch__plain" aria-labelledby="plain-h">
          <h3 id="plain-h">What happens</h3>
          <ul>
            <li>Your coin trades against {col.name} tokens ({col.wxSymbol}: 1 token is 1 wrapped {col.name} NFT) on a bonding curve; people can pay in SOL through this site. Once the curve holds <strong>{solWithTokens(tok, wxUnits(t.wx))}</strong> ({col.name}'s threshold, SOL at the day-1 floor), it graduates to a Meteora pool.</li>
            <li>At graduation, <strong>50 % of the raise ({solWithTokens(tok, wxUnits(t.backingWx))})</strong> goes straight into your coin's own vault, held by the custodian for its holders: wrapped {col.name} NFTs, more than one NFT's worth. Any holder can burn coins for their share.</li>
            <li>The pool's liquidity is <strong>locked forever</strong>. Nobody can pull it, not you and not Rook.</li>
            <li><strong>What you earn:</strong> nothing on the curve (the creator's share of curve fees is 0 %); after graduation, your locked LP position earns its share of the pool's 1 % fee (about 0.4 % of volume), in {col.wxSymbol}, claimable on Meteora.</li>
            <li>The name, ticker, image and collection can't be changed after launch.</li>
          </ul>
        </section>
      </div>
      <aside className="launch__side panel launch__pay" aria-label="Costs and launch">
        <OpenCollection c={col} compact />
        <p className="eyebrow">Graduation</p>
        <LaunchGraduation tok={tok} wx={t.wx} backingWx={t.backingWx} />
        <p className="eyebrow">Cost</p>
        <LaunchCosts costs={costs} firstBuy={lamports} quote={{ isSol: quoteIsSol, tok: quoteIsSol ? { ...tok, floor: null } : tok }} />
        {lamports > 0n && dbc ? <p className="lcost__note">{firstBuyNote(launchBuyFeeShort(dbc), SLIPPAGE_BPS)}</p> : null}
        {lamports > 0n && !quoteIsSol && bal.data ? <p className="lcost__note">{holdText(tok, bal.data.wx)}.</p> : null}

        {phase ? <ol className="launch__progress" aria-live="polite">{PHASES.map(([p, label], i) => {
          const at = phase === "done" ? 3 : PHASES.findIndex(([q]) => q === phase);
          const st = i < at ? "done" : i === at ? "now" : "todo";
          return <li key={p} className={`launch__ph launch__ph--${st}`}><span className="launch__phdot" aria-hidden="true">{st === "done" ? "✓" : ""}</span>{label}
            {st === "now" && p === "create" && signing ? <small> — approve in your wallet</small> : null}<span className="vh">{st === "done" ? " (done)" : st === "now" ? " (in progress)" : ""}</span></li>;
        })}</ol> : null}

        {DEMO ? <p className="notice small launch__demo" role="note"><strong>Demo: nothing is sent.</strong> {demoDone ? "On the live site your wallet would have signed one transaction and you'd now be on your coin's page." : "Launch shows the steps the live site would go through: upload the image, create the coin (your wallet signs), confirm."}</p> : null}

        {!DEMO && config && !w.address ? <div className="launch__connect"><p className="small" style={{ margin: 0 }}>Connect a wallet to launch.</p><SolWalletButton /></div>
          : <button type="button" className="btn btn--primary btn--block launch__go" disabled={!!why || busy || (DEMO && demoDone)} aria-describedby={why ? "launch-why" : undefined} onClick={() => void launch()}>
            {busy ? "Launching…" : DEMO ? (demoDone ? "Demo complete" : "Launch (demo)") : "Launch"}</button>}
        {why ? <p id="launch-why" className="small muted">{why}</p> : null}
        {err ? <p className="small error" role="alert">{err}</p> : null}
        <p className="small muted">One transaction: your wallet signs first, then the page signs with your new coin's key (made for this launch, never stored). It is simulated before your wallet is asked. <a href={href("risk")}>Risk</a></p>
      </aside>
    </div>
  );
}

