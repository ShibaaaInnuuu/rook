import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { WrappedCollection } from "../api/types";
import { href, useEnv } from "../lib/env";
import { type WSource } from "../lib/wrapped";
import { eth } from "../lib/format";
import { pct } from "../lib/quote";
import { CURVE_FACTORY, FLOOR_MODULE_FRESH, WAKE_DESK } from "../rook/abis";
import { bondOf, capacityFrom, capacityText, capacityUrl, launchWait, lifeOf, livesFrom, wakeView } from "../rook/dormancy";
import { useCatalog, type CatalogSrc } from "../rook/catalogData";
import { STATUS_TONE, STATUS_WORD, entryOfNest, floorText, launchable, wizardOrder } from "../rook/cataloglogic";
import type { CatalogEntry } from "../rook/catalogTypes";
import { DEFAULT_FEE_BPS, DEFAULT_LAUNCH_FEE_BPS, DEFAULT_LAUNCH_FEE_DECAY_S, MAX_FEE_BPS, MIN_FEE_BPS, POLICIES, POLICY_INDEX, VAULT_SHARE_BPS, volumePerItem, type Policy } from "../rook/tiers";
import { PHASE1, policiesFor, stackCollection } from "../rook/phase";
import { LAUNCHES_PAUSED, launchBlocked } from "../rook/keyless";
import { bundledStack } from "../rook/stack";
import { Gate } from "../ui/Gate";
import { TxButton } from "../ui/wallet";
import { Loading, Pill } from "../ui/bits";
import { Picture } from "../ui/Picture";

const STEPS = ["The collection", "The policy", "The fee", "The coin"] as const;
type Step = 0 | 1 | 2 | 3;

/** docs/v6/site-design.md §6: four steps — collection → policy → fee tier with the ETH-per-item preview → target and summary. The nests are the catalogue's (docs/v6/collections-catalog.md §4): a live one launches, a listable one walks the wizard and waits for its listing, the rest are greyed with their status. */
export function Launch({ slug }: { slug?: string }) {
  return <Gate page="launch">{(src) => <Wizard src={src} slug={slug} />}</Gate>;
}
function Wizard({ src, slug }: { src: WSource; slug?: string }) {
  const cat = useCatalog();
  const nests = useMemo(() => wizardOrder(cat.data?.entries ?? []), [cat.data]);
  const [step, setStep] = useState<Step>(slug ? 1 : 0);
  const [form, setForm] = useState<{ slug: string; policy: Policy; feeBps: number; name: string; symbol: string; sweepOnly: boolean; image: string | null; x: string; telegram: string; website: string }>({ slug: slug ?? "", policy: "raffle", feeBps: DEFAULT_FEE_BPS, name: "", symbol: "", sweepOnly: false, image: null, x: "", telegram: "", website: "" });
  const pickImage = (f: File | undefined) => { if (!f || !f.type.startsWith("image/") || f.size > 512 * 1024) { setForm((x) => ({ ...x, image: null })); return; } const r = new FileReader(); r.onload = () => setForm((x) => ({ ...x, image: String(r.result) })); r.readAsDataURL(f); };
  // the chosen nest: by its Rook slug when it is live, by its catalogue slug otherwise
  const nest = useMemo(() => (form.slug ? entryOfNest(nests, form.slug) ?? nests.find((e) => e.slug.toLowerCase() === form.slug.toLowerCase()) ?? null : null), [nests, form.slug]);
  const rookSlug = nest?.rook.kind === "live" ? nest.rook.slug : null;
  const detail = useQuery({ queryKey: ["launch-nest", src.kind, rookSlug], enabled: !!rookSlug, queryFn: () => src.get<WrappedCollection>(`/v1/wrapped/${rookSlug}`) });
  // the floor the preview prices against: the keeper's cap for a live nest, the catalogue's floor otherwise (said as such)
  const keeperFloor = detail.data?.floorWei ? BigInt(detail.data.floorWei) : null;
  const catFloor = nest?.floor?.eth !== null && nest?.floor?.eth !== undefined ? BigInt(Math.round(nest.floor.eth * 1e6)) * 10n ** 12n : null;
  const floor = keeperFloor ?? catFloor;
  const perItem = floor && floor > 0n ? volumePerItem(floor, form.feeBps, VAULT_SHARE_BPS) : null;
  const problem: (string | null)[] = [
    !nest ? "Choose the collection whose floor this coin buys." : !launchable(nest) ? `${nest.name} is ${STATUS_WORD[nest.rook.kind].toLowerCase()}: ${nest.rook.why}` : null,
    null,
    form.feeBps < MIN_FEE_BPS || form.feeBps > MAX_FEE_BPS ? "The fee tier is between 1% and 8%." : null,
    !form.name.trim() ? "Give the coin a name." : !/^[A-Za-z0-9]{2,10}$/.test(form.symbol) ? "A symbol is 2 to 10 letters or digits." : null,
  ];
  const blocked = problem.slice(0, step + 1).find((p) => p) ?? null;
  const policies = policiesFor(POLICIES, PHASE1);
  const policy = policies.find((p) => p.id === form.policy) ?? policies[0];
  // Phase 1 (docs/v6/phase1-scope.md §8.3): launch through the bundled stack file's factory, never one the indexer alone names
  const served = detail.data?.contracts.factory ?? null;
  const pinned = PHASE1 ? stackCollection(bundledStack.stack, rookSlug)?.factory ?? null : null;
  const factory = PHASE1 ? pinned : served;
  const factoryWhy = PHASE1 && rookSlug && !pinned ? `This build's stack file has no ${nest?.name ?? rookSlug} collection, so there is nothing to launch through.`
    : PHASE1 && pinned && served && served.toLowerCase() !== pinned.toLowerCase() ? `The indexer names factory ${served} for ${nest?.name ?? rookSlug}, but this build's stack file names ${pinned}. Nothing is sent while they differ.` : null;
  // A-03: the factory refuses a launch that buys items while the queue has no room for its budget (`launchesOpen()`, read by the indexer)
  const paused = detail.data?.launchesOpen === false;
  const notYet = nest && nest.rook.kind === "listable" ? `${nest.name} meets the listing rule but is not listed yet: there is no curve factory to launch through until it is (the collection page says what listing takes).` : null;
  // docs/v7/dormant-collections.md §4.2: a dormant collection is woken first (a bonded WakeDesk request); while it warms up the page says
  // so with the canister's estimate. The lives come from the runner's GET /collections (VITE_COLLECTIONS_URL); without it, nothing changes
  const livesUrl = (import.meta.env as Record<string, string | undefined>).VITE_COLLECTIONS_URL;
  const lives = useQuery({ queryKey: ["collection-lives", livesUrl], enabled: !!livesUrl, refetchInterval: 60_000,
    queryFn: () => fetch(livesUrl!).then((r) => r.json()).then(livesFrom) });
  // docs/v7/dormant-collections.md §11: "N of M collections can be awake right now" (the runner's GET /capacity, next to GET /collections)
  const capUrl = capacityUrl(livesUrl);
  const capacity = useQuery({ queryKey: ["collection-capacity", capUrl], enabled: !!capUrl, refetchInterval: 60_000,
    queryFn: () => fetch(capUrl!).then((r) => r.json()).then(capacityFrom) });
  const cap = capacityText(capacity.data ?? null);
  const wrapperAddr = PHASE1 ? stackCollection(bundledStack.stack, rookSlug)?.wrapper ?? null : detail.data?.contracts.wrapper ?? null;
  const bond = bondOf(bundledStack.stack);
  // dormant-review DR-M-4: under the dormancy the page reads the chain itself (the module's freshness, the desk's open request) and
  // fails closed; the runner's lives only say how to wait
  const env = useEnv();
  const live = env && env.mode === "live" ? env : null;
  const dormancy = bond !== null;
  const moduleAddr = stackCollection(bundledStack.stack, rookSlug)?.floorModule ?? null;
  const fresh = useQuery({ queryKey: ["module-fresh", moduleAddr], enabled: dormancy && !!live && !!moduleAddr, refetchInterval: 60_000,
    queryFn: () => live!.client.readContract({ address: moduleAddr as `0x${string}`, abi: FLOOR_MODULE_FRESH, functionName: "fresh" }) });
  // independent review 2 (D2-I-1): when the floor was last posted (with no lives served, the wake is offered only after a post cadence)
  const postedAt = useQuery({ queryKey: ["module-posted-at", moduleAddr], enabled: dormancy && !!live && !!moduleAddr, refetchInterval: 60_000,
    queryFn: () => live!.client.readContract({ address: moduleAddr as `0x${string}`, abi: FLOOR_MODULE_FRESH, functionName: "postedAt" }) });
  const open = useQuery({ queryKey: ["wake-open", wrapperAddr], enabled: dormancy && !!live && !!wrapperAddr, refetchInterval: 60_000,
    queryFn: () => live!.client.readContract({ address: bundledStack.stack!.wakeDesk as `0x${string}`, abi: WAKE_DESK, functionName: "openOf", args: [wrapperAddr as `0x${string}`] }) });
  // independent review DI-M-2 / DI-L-1: the open request's time and the desk's window say what an open request on a dormant collection
  // means; with no lives served, a module read not fresh offers the wake (gated on the openOf read)
  const openId = open.data?.[0] === true ? open.data[1] : undefined;
  const openAt = useQuery({ queryKey: ["wake-open-at", openId?.toString()], enabled: dormancy && !!live && openId !== undefined, refetchInterval: 60_000,
    queryFn: () => live!.client.readContract({ address: bundledStack.stack!.wakeDesk as `0x${string}`, abi: WAKE_DESK, functionName: "requestAt", args: [openId!] }) });
  const deskWindow = useQuery({ queryKey: ["wake-window"], enabled: dormancy && !!live, staleTime: Infinity,
    queryFn: () => live!.client.readContract({ address: bundledStack.stack!.wakeDesk as `0x${string}`, abi: WAKE_DESK, functionName: "window" }) });
  const openArg = open.data === undefined ? null : openId !== undefined ? { id: openId, at: openAt.data ? Number(openAt.data[2]) : undefined } : false;
  const wake = wakeView(lifeOf(lives.data ?? null, wrapperAddr), openArg,
    { dormancy, fresh: fresh.data, postedAt: postedAt.data !== undefined ? Number(postedAt.data) : undefined, windowSecs: deskWindow.data !== undefined ? Number(deskWindow.data) : undefined, nowS: Math.floor(Date.now() / 1000) });
  const wakeWait = launchWait({ dormancy, sweepOnly: form.sweepOnly, fresh: fresh.data, view: wake });
  // what the collection's state means for this launch, shown whenever its floor is not live (the launch itself waits only for step 1)
  const wakeNote = wakeWait ?? (dormancy && !form.sweepOnly && fresh.data !== true ? wake.launchWait : null);

  return (
    <>
      <header className="pagehead rise"><p className="eyebrow">Launch a coin</p><h1>Launch a coin that buys the floor</h1>
        <p className="lede">Four choices, none of them changeable afterwards by you, by us, or by the coin's holders. The coin starts on a curve; when its raise is met it graduates into a locked pool and its budget buys NFTs from the collection's floor into the vault.</p></header>
      <div className="wizard rise rise-1">
        <div>
          <ol className="steps">{STEPS.map((s, i) => <li key={s} aria-current={i === step ? "step" : undefined} className={i < step ? "done" : ""}><button type="button" onClick={() => setStep(i as Step)} disabled={i > step && !!blocked}>{s}</button></li>)}</ol>
          {step === 0 ? (
            <div className="stack">
              <p className="muted">The collection whose floor this coin's fees buy. Its floor sets what one NFT costs, and so how much trading buys one. Every collection in the list is here: a live collection launches now; a listable one can be walked through and waits for its listing; the rest say why not.</p>
              {cat.sourceLoading || cat.isLoading ? <Loading what="the collections" /> : null}
              {cat.source ? nests.map((e) => <NestChoice key={e.slug} e={e} src={cat.source!} on={!!nest && nest.slug === e.slug} pick={() => setForm((f) => ({ ...f, slug: e.rook.kind === "live" ? e.rook.slug : e.slug }))} />) : null}
              {!nests.length && !cat.isLoading && !cat.sourceLoading ? <p className="notice notice--warn small">No collection list is served, so there is nothing to launch on.</p> : null}
            </div>
          ) : step === 1 ? (
            <fieldset className="stack" style={{ border: 0, padding: 0, margin: 0 }}>
              <legend className="muted" style={{ marginBottom: "1rem" }}>What happens to each NFT the fees buy. Fixed for the coin's life.</legend>
              {policies.map((p) => (
                <label key={p.id} className={`choice${form.policy === p.id ? " choice--on" : ""}`}>
                  <input type="radio" name="policy" value={p.id} checked={form.policy === p.id} onChange={() => setForm((f) => ({ ...f, policy: p.id }))} />
                  <span><strong>{p.title}</strong><small>{p.line}</small></span>
                </label>
              ))}
            </fieldset>
          ) : step === 2 ? (
            <div className="stack tier">
              <p className="muted">The trade fee, 1% to 8%. A higher fee buys an NFT with less trading and makes the coin dearer to trade. It is public on the coin's page and cannot be changed.</p>
              <p className="tier__pick"><span className="num">{pct(form.feeBps, form.feeBps % 100 ? 1 : 0)}</span><small>of every trade</small></p>
              <input type="range" min={MIN_FEE_BPS} max={MAX_FEE_BPS} step={50} value={form.feeBps} aria-label="Fee tier" onChange={(e) => setForm((f) => ({ ...f, feeBps: Number(e.target.value) }))} />
              <div className="tier__scale"><span>1%</span><span>4%</span><span>8%</span></div>
              <div className="panel panel--tight">
                <p className="eyebrow">ETH of trading per NFT</p>
                {perItem ? <p style={{ margin: 0 }}><strong className="num" style={{ fontSize: "var(--t3)" }}>{eth(perItem, 1)} ETH</strong> <span className="muted small">of volume buys one {nest?.name.replace(/s$/, "").toLowerCase()} at {keeperFloor ? "today's keeper floor" : `the collection list's ${nest?.floor?.stale ? "dated " : ""}floor`} of {eth(floor!, 3)} ETH, with {pct(VAULT_SHARE_BPS, 0)} of the fee going to the vault.</span></p>
                  : <p className="muted small" style={{ margin: 0 }}>{nest ? "The collection has no floor to preview against." : "Choose a collection first."}</p>}
              </div>
              <p className="small muted">For the first {DEFAULT_LAUNCH_FEE_DECAY_S / 60} minutes a buy also pays an extra fee that starts at {pct(DEFAULT_LAUNCH_FEE_BPS, 0)} and falls to nothing; all of it goes to the coin's holders, none to you and none to us.</p>
            </div>
          ) : (
            <div className="stack">
              <label className="field">Name<input value={form.name} maxLength={40} autoComplete="off" onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))} /></label>
              <label className="field">Symbol<input className="mono" value={form.symbol} maxLength={10} autoComplete="off" onChange={(e) => setForm((f) => ({ ...f, symbol: e.target.value.toUpperCase() }))} /></label>
              <div className="panel panel--tight stack">
                <p className="eyebrow" style={{ margin: 0 }}>Image and links · stored off-chain</p>
                <div className="row" style={{ alignItems: "center" }}>
                  {form.image ? <img src={form.image} alt="" width={56} height={56} style={{ borderRadius: "22%", objectFit: "cover" }} /> : <span className="face" style={{ width: 56, height: 56, borderRadius: "22%", background: "var(--raised)", display: "block" }} />}
                  <label className="field" style={{ flex: 1 }}>Image (PNG, JPG or SVG, up to 512 kB)<input type="file" accept="image/*" onChange={(e) => pickImage(e.target.files?.[0])} /></label>
                </div>
                <label className="field">X<input value={form.x} placeholder="@handle" onChange={(e) => setForm((f) => ({ ...f, x: e.target.value }))} /></label>
                <label className="field">Telegram<input value={form.telegram} placeholder="t.me/…" onChange={(e) => setForm((f) => ({ ...f, telegram: e.target.value }))} /></label>
                <label className="field">Website<input value={form.website} placeholder="https://…" onChange={(e) => setForm((f) => ({ ...f, website: e.target.value }))} /></label>
                <p className="small muted" style={{ margin: 0 }}><strong>Not on chain, and not uploaded yet.</strong> <span className="mono">CurveFactory.launch</span> carries a name, a symbol, a policy and a fee tier and nothing else. The image and links are shown here only; the indexer's coin-metadata registry that will hold them (signed by the creator's key, served beside the board) is specified in <a href={href("docs", "site-design")}>the design, §6.3</a> and is not built, so nothing here is saved. No upload succeeded because none was attempted.</p>
              </div>
              <label className="choice" style={{ marginTop: ".5rem" }}><input type="checkbox" checked={form.sweepOnly} onChange={(e) => setForm((f) => ({ ...f, sweepOnly: e.target.checked }))} /><span><strong>Floor buys only</strong> <span className="muted small">(sweep-only)</span><small>The coin pairs with ETH instead of the collection's wrapped token and skips the NFT budget at graduation; fees still buy the floor. Leave off unless you know why.</small></span></label>
            </div>
          )}
          {blocked ? <p className="small warn" role="status" style={{ marginTop: "1rem" }}>{blocked}</p> : notYet && step === 0 ? <p className="small muted" role="status" style={{ marginTop: "1rem" }}>{notYet}</p> : null}
          <div className="row" style={{ marginTop: "1.5rem" }}>
            {step > 0 ? <button type="button" className="btn btn--quiet" onClick={() => setStep((step - 1) as Step)}>Back</button> : null}
            {step < 3 ? <button type="button" className="btn" disabled={!!blocked} onClick={() => setStep((step + 1) as Step)}>Next</button> : null}
          </div>
        </div>
        <aside className="panel summary">
          <p className="eyebrow">Summary</p>
          <dl className="kv">
            <dt>Collection</dt><dd>{nest ? nest.name : "—"}</dd>
            <dt>Status</dt><dd>{nest ? <Pill tone={STATUS_TONE[nest.rook.kind]}>{STATUS_WORD[nest.rook.kind]}</Pill> : "—"}</dd>
            {rookSlug && lives.data ? <><dt>Floor watch</dt><dd><Pill tone={wake.tone === "good" ? "good" : wake.tone === "warn" ? "warn" : undefined}>{wake.word}</Pill></dd></> : null}
            {rookSlug && cap ? <><dt>Awake now</dt><dd title={cap.note}>{cap.line}</dd></> : null}
            <dt>Policy</dt><dd>{policy.title}</dd>
            <dt>Fee tier</dt><dd>{pct(form.feeBps, form.feeBps % 100 ? 1 : 0)}</dd>
            <dt>To the vault</dt><dd>{pct((form.feeBps * VAULT_SHARE_BPS) / 10_000, 2)} of volume</dd>
            <dt>Trading per NFT</dt><dd>{perItem ? `${eth(perItem, 1)} ETH` : "—"}</dd>
            <dt>Coin</dt><dd>{form.name ? `${form.name} (${form.symbol || "…"})` : "—"}</dd>
          </dl>
          {paused ? <p className="notice notice--warn small" role="status" style={{ marginTop: "1rem" }}><strong>{LAUNCHES_PAUSED}</strong> The vault at stage 0 can convert only a few NFTs, and the floor module holds at most 40 wrapped tokens backed by ETH; a new coin's NFT budget has no room until some are burned or converted. A floor-buys-only launch has no NFT budget and is not affected.</p> : null}
          {rookSlug && wakeNote ? <div className="notice small" role="status" style={{ marginTop: "1rem" }}><p style={{ margin: 0 }}>{wakeNote}</p>
            {wakeWait && wake.launchWait ? <p style={{ margin: ".4rem 0 0" }}>{wake.launchWait}</p> : null}
            {wake.detail ? <p className="muted" style={{ margin: ".4rem 0 0" }}>{wake.detail}</p> : null}
            {cap && wake.word !== "Awake" ? <p className="muted" style={{ margin: ".4rem 0 0" }}>{cap.line}. {cap.note}</p> : null}
            {wake.canWake ? <div style={{ marginTop: ".75rem" }}><TxButton label={bond !== null ? `Step 1: wake it (bond ${eth(bond, 4)} ETH, back when your coin graduates)` : "Step 1: wake it"} block
              disabledWhy={bond === null ? "This build's stack file names no wake desk or bond, so a wake cannot be asked from here." : !wrapperAddr ? "This collection has no wrapper in the stack file." : null}
              request={() => ({ address: bundledStack.stack!.wakeDesk!, abi: WAKE_DESK, functionName: "request", args: [wrapperAddr!], value: bond! })} /></div> : null}</div> : null}
          <div style={{ marginTop: "1.25rem" }}>
            <TxButton label={rookSlug && !form.sweepOnly && wake.word !== "Awake" && fresh.data !== true ? "Step 2: launch your coin" : "Launch a coin"} block disabledWhy={blocked ?? problem[3] ?? notYet ?? factoryWhy ?? wakeWait ?? (launchBlocked(detail.data?.launchesOpen, form.sweepOnly) ? LAUNCHES_PAUSED : null) ?? (!factory ? "This collection has no curve factory configured, so there is nothing to launch through." : null)}
              request={() => ({ address: factory!, abi: CURVE_FACTORY, functionName: "launch", args: [form.name.trim(), form.symbol, POLICY_INDEX[policy.id], form.sweepOnly, form.feeBps] })} />
          </div>
          <p className="small muted" style={{ marginTop: "1rem", marginBottom: 0 }}>One transaction: <span className="mono">CurveFactory.launch</span>. The curve, its pool key and its vault are created together. <a href={href("docs", "policies-and-fees")}>The fee and policy design</a>.</p>
        </aside>
      </div>
    </>
  );
}

function NestChoice({ e, src, on, pick }: { e: CatalogEntry; src: CatalogSrc; on: boolean; pick: () => void }) {
  const can = launchable(e);
  return (
    <label className={`choice choice--nest${on ? " choice--on" : ""}${can ? "" : " choice--off"}`} title={can ? undefined : e.rook.why}>
      <input type="radio" name="nest" checked={on} disabled={!can} onChange={pick} />
      <Picture src={src.image(e)} seed={e.slug} name={e.name} size={40} />
      <span><strong>{e.name} <span className="muted" style={{ fontWeight: 400 }}>{e.chainName}</span></strong>
        <small>Floor {floorText(e)}{e.floor?.stale ? " (dated)" : ""} · {e.items === null ? "?" : e.items.toLocaleString("en-US")} {e.chain === "bitcoin" ? "ordinals" : "NFTs"} · {e.heldByRook ?? 0} in Rook's vault · <Pill tone={STATUS_TONE[e.rook.kind]}>{STATUS_WORD[e.rook.kind]}{e.rook.kind === "live" && e.rook.coins ? ` · ${e.rook.coins} coins` : ""}</Pill></small>
        {!can ? <small className="muted">{e.rook.why.length > 140 ? `${e.rook.why.slice(0, 140)}…` : e.rook.why}</small> : null}</span>
    </label>
  );
}
