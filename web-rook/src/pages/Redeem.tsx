import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useAccount } from "wagmi";
import type { Ticket, WrappedCollection } from "../api/types";
import { DRAND } from "../lib/chain";
import { href, useEnv, useNow } from "../lib/env";
import { useTicket, useWrappedCollection, type WSource } from "../lib/wrapped";
import { redeemDestinationProblem } from "../lib/redeem";
import { NFT_WRAPPER_REDEEM } from "../lib/abisV3";
import { BEACON, WRAPPER_P1 } from "../rook/abis";
import { drandSignature, ticketSteps } from "../rook/keyless";
import { PHASE1, pinnedAddress } from "../rook/phase";
import { bundledStack } from "../rook/stack";
import { countdown, eth, when } from "../lib/format";
import { Gate } from "../ui/Gate";
import { TxButton } from "../ui/wallet";
import { Loading, Proof, Wrapped, ago } from "../ui/bits";
import { nftWord, wrappedLine } from "../rook/words";
import { useV8Collections } from "../rook/v8/source";
import { ticketStage } from "./Reserves";

/** docs/v5/ux-and-reserves.md §1, "Redeem an item": one whole wrapped token (a unit) for one random NFT, drawn by a drand round that does not exist yet. */
export function RedeemPage({ slug, ticket }: { slug?: string; ticket?: string }) {
  return <Gate page={`redeem/${slug ?? ""}${ticket ? `/${ticket}` : ""}`}>{(src) => <Redeem src={src} slug={slug} ticket={ticket} />}</Gate>;
}
function Redeem({ src, slug, ticket }: { src: WSource; slug?: string; ticket?: string }) {
  const q = useWrappedCollection(slug);
  if (q.isLoading) return <Loading what="the collection" />;
  if (!q.data) return <div className="pagehead"><h1>No such collection</h1><p className="lede"><a href={href("collections")}>All collections</a>.</p></div>;
  const c = q.data;
  const id = ticket !== undefined && /^\d+$/.test(ticket) ? Number(ticket) : undefined;
  return id !== undefined ? <Track src={src} c={c} id={id} /> : <Burn src={src} c={c} />;
}

function Burn({ src, c }: { src: WSource; c: WrappedCollection }) {
  const env = useEnv(), { address } = useAccount();
  const live = env && env.mode === "live" ? env : null;
  const [dest, setDest] = useState("");
  const abi = c.kind === "nft" ? NFT_WRAPPER_REDEEM : WRAPPER_P1;
  // E-07: in Phase 1 the wrapper a redemption burns at is the stack file's, never the indexer's alone
  const pin = PHASE1 ? pinnedAddress(bundledStack.stack, c.slug, "wrapper", c.wrapper) : null;
  const wrapper = pin ? pin.address : c.wrapper;
  const balance = useQuery({ queryKey: ["wx-balance", wrapper, address], enabled: !!live && !!address && !!wrapper, queryFn: () => live!.client.readContract({ address: wrapper!, abi, functionName: "balanceOf", args: [address!] }) });
  const b = c.kind === "inscriptions", one1 = nftWord(b, 1);
  const problem = dest ? redeemDestinationProblem(c, dest) : `Enter where the ${one1} should go.`;
  const v8 = useV8Collections();
  const drawTo = v8.data?.collections.includes(c.slug) ? href("draw", c.slug) : href("draw");
  const one = 10n ** 18n;
  return (
    <>
      <header className="pagehead rise"><p className="eyebrow"><a href={href("collection", c.slug)}>{c.name}</a> · Redeem</p><h1>One {c.symbol} for one random {one1}</h1>
        <p className="lede">Burn one whole <Wrapped symbol={c.symbol} bitcoin={b} /> and receive one {one1} from the vault. It is chosen by a drand round that does not exist yet when you burn, so nobody — including us — can pick which one you get.</p>
        <p className="small muted" style={{ margin: 0 }}>{wrappedLine(c.symbol, b)}</p></header>
      <div className="grid rise rise-1">
        <div className="span-6 panel stack">
          <label className="field">{c.kind === "inscriptions" ? "Your Bitcoin taproot address (bc1p…)" : `Your address on ${c.custody.kind === "evm" ? c.custody.chainName : "the collection's chain"}`}
            <input className="mono" value={dest} autoComplete="off" spellCheck={false} onChange={(e) => setDest(e.target.value)} placeholder={c.kind === "inscriptions" ? "bc1p…" : "0x…"} />
            <small>{problem && dest ? <span className="warn">{problem}</span> : c.kind === "inscriptions" ? `Checked as the contract checks it: a taproot address, nothing else. An ${one1} sent to a wrong address you gave cannot be recovered.` : "Not the zero address, not the custodian."}</small></label>
          <p className="small muted" style={{ margin: 0 }}>{balance.data !== undefined ? <>You hold <span className="num">{eth(balance.data, 4)}</span> {c.symbol}; a redemption burns exactly one.</> : live ? "Connect a wallet to see what you hold." : "Your balance is read when a deployment exists."}</p>
          <TxButton label={`Burn one ${c.symbol} for a random ${one1}`} block disabledWhy={pin?.why ?? problem ?? (balance.data !== undefined && balance.data < one ? `You need one whole ${c.symbol}.` : null)}
            request={() => ({ address: wrapper!, abi, functionName: "redeem", args: [dest.trim()] })} />
        </div>
        <div className="span-6">
          <p className="eyebrow">How a redemption goes</p>
          <ol className="timeline">
            <li><strong>You burn one {c.symbol}.</strong> <span className="muted">Your burn pins a ticket to a future drand round.</span></li>
            <li><strong>The round is published.</strong> <span className="muted">Anyone can then prove it on chain (tickets resolve in order), and the round is final: nobody can swap it for another.</span></li>
            <li><strong>One {one1} is chosen.</strong> <span className="muted">The ticket resolves to one {one1} in the vault, chosen by that round. <a href={drawTo}>See the odds and every past redemption's proof</a>.</span></li>
            <li><strong>It is delivered.</strong> <span className="muted">The vault's custodian delivers it to your address within 7 days.</span></li>
          </ol>
          <div className="timeline__branch small">
            <p style={{ margin: 0 }}><strong>If it goes wrong, you get your {c.symbol} back.</strong></p>
            <ul>
              <li><span className="muted">Nobody proves the round within <strong>7 days</strong> of your burn:</span> anyone can refund it then.</li>
              <li><span className="muted">The {one1} cannot be delivered:</span> the custodian attests it on chain, and anyone can press refund <strong>72 hours</strong> after the delivery deadline.</li>
              <li><span className="muted">No word from the custodian (a custodian that is gone):</span> the refund opens <strong>30 days</strong> after the deadline. Until then a late delivery is still recorded, so no refund can race it.</li>
            </ul>
            <p className="muted" style={{ margin: 0 }}>A refund returns one whole {c.symbol} to the ticket's owner.</p>
          </div>
          <p className="small muted">Track a ticket by its number: {c.openTickets.length ? c.openTickets.map((t) => <a key={t.id} href={href("redeem", c.slug, t.id)} style={{ marginRight: ".5rem" }}>#{t.id}</a>) : "none in flight now"}. The randomness: <Proof href={DRAND} fixture={src.kind === "fixture"}>drand, evmnet</Proof>.</p>
        </div>
      </div>
    </>
  );
}

function Track({ src, c, id }: { src: WSource; c: WrappedCollection; id: number }) {
  const now = useNow(), env = useEnv();
  const live = env && env.mode === "live" ? env : null;
  const q = useTicket(c.slug, id);
  const pin = PHASE1 ? pinnedAddress(bundledStack.stack, c.slug, "wrapper", c.wrapper) : null;
  const wrapper = pin ? pin.address : c.wrapper;
  const pinned = q.data?.stage === "pinned" && c.kind === "inscriptions";
  // tickets resolve strictly in order (Wrapper.resolve reverts OutOfOrder): the next one is read on chain
  const next = useQuery({ queryKey: ["next-to-resolve", wrapper], enabled: !!live && !!wrapper && pinned, refetchInterval: 15_000,
    queryFn: () => live!.client.readContract({ address: wrapper!, abi: WRAPPER_P1, functionName: "nextToResolve" }) });
  if (q.isLoading) return <Loading what={`ticket ${id}`} />;
  if (!q.data) return <div className="pagehead"><h1>No ticket {id}</h1><p className="lede"><a href={href("redeem", c.slug)}>Redeem</a></p></div>;
  const t: Ticket = q.data, fixture = src.kind === "fixture";
  const abi = c.kind === "nft" ? NFT_WRAPPER_REDEEM : WRAPPER_P1;
  const steps = ticketSteps(t, now, next.data !== undefined ? Number(next.data) : null);
  const stepsList: { id: Ticket["stage"]; word: string }[] = [{ id: "pinned", word: "Pinned" }, { id: "resolved", word: "Resolved" }, { id: "delivered", word: "Delivered" }];
  const at = t.stage === "refunded" ? -1 : stepsList.findIndex((s) => s.id === t.stage);
  // Resolve: the round's randomness is read from the beacon first; only a round nobody proved yet needs drand's signature
  const resolveRequest = async () => {
    let sig: `0x${string}` = "0x";
    if (live) {
      const beacon = await live.client.readContract({ address: wrapper!, abi: WRAPPER_P1, functionName: "beacon" });
      const r = await live.client.readContract({ address: beacon, abi: BEACON, functionName: "randomnessOf", args: [BigInt(t.round)] });
      if (/^0x0{64}$/.test(r)) sig = await drandSignature(t.round);
    }
    return { address: wrapper!, abi: WRAPPER_P1, functionName: "resolve", args: [BigInt(t.id), sig] };
  };
  return (
    <>
      <header className="pagehead rise"><p className="eyebrow"><a href={href("collection", c.slug)}>{c.name}</a> · <a href={href("redeem", c.slug)}>Redeem</a></p><h1>Ticket {t.id}</h1><p className="lede">{ticketStage(t, now)}</p></header>
      <div className="grid rise rise-1">
        <div className="span-6 panel">
          <ul className="phase__steps" aria-hidden="true">{stepsList.map((s, i) => <li key={s.id} className={i === at ? "on" : i < at ? "done" : ""} />)}</ul>
          <dl className="kv" style={{ marginTop: "1rem" }}>
            <dt>Owner</dt><dd className="break">{t.owner}</dd>
            <dt>Destination</dt><dd className="break">{t.destination}</dd>
            <dt>Burnt</dt><dd>{t.burnedAt ? `${when(t.burnedAt)} (${ago(t.burnedAt, now)})` : "—"}</dd>
            <dt>drand round</dt><dd>{t.round}</dd>
            <dt>{c.kind === "inscriptions" ? "Ordinal" : "NFT"}</dt><dd className="break">{t.itemRef ?? (t.refundedUnresolved ? "none: refunded before it was resolved" : "not drawn yet")}</dd>
            <dt>Deliver by</dt><dd>{t.deliverBy ? `${when(t.deliverBy)}${now < t.deliverBy ? ` (in ${countdown(t.deliverBy, now)})` : ""}` : "—"}</dd>
            {t.stage === "resolved" ? <><dt>The custodian's word</dt><dd>{t.undeliverable ? "attested undeliverable (Wrapper.markUndeliverable): refundable 72 hours after the deadline" : "none: delivering, or refundable 30 days after the deadline"}</dd></> : null}
            <dt>Delivery</dt><dd>{t.deliveryTx ? <Proof href={t.deliveryUrl} fixture={fixture}>{t.deliveryTx.slice(0, 14)}…</Proof> : "—"}</dd>
          </dl>
        </div>
        <div className="span-6 stack">
          {steps.resolve && c.kind === "inscriptions" ? (
            <div className="panel panel--tight">
              <p className="small" style={{ marginBottom: ".75rem" }}><strong className="accent">Resolve.</strong> Anyone may prove the round on chain; the ticket then draws its {nftWord(c.kind === "inscriptions", 1)}. The round is final: there is no redraw.</p>
              <TxButton label="Resolve the ticket" primary={false} disabledWhy={pin?.why ?? steps.resolve.why} request={resolveRequest} />
            </div>) : null}
          {steps.refund ? (steps.refund.ok
            ? <div className="panel panel--tight"><p className="small" style={{ marginBottom: ".75rem" }}><strong className="bad">{t.stage === "pinned" ? "Unresolved for 7 days." : t.undeliverable ? "Undelivered 72 hours past the deadline, and the vault's custodian attested it cannot be delivered." : "Undelivered 30 days past the deadline."}</strong> Anyone may press this; one whole {c.symbol} goes back to the ticket's owner.</p><TxButton label="Refund the ticket" primary={false} disabledWhy={pin?.why ?? null} request={() => ({ address: wrapper!, abi, functionName: "refund", args: [BigInt(t.id)] })} /></div>
            : steps.refund.at ? <p className="small muted">{t.stage === "pinned" ? "If nobody resolves it" : "If it is not delivered"}, it is refundable by anyone from about {when(steps.refund.at)}.{t.stage === "resolved" ? ` ${steps.refund.why ?? ""}` : ""}</p> : null) : null}
          <p className="small muted">The round: <Proof href={`${DRAND}/public/${t.round}`} fixture={fixture}>drand round {t.round}</Proof>. The burn: <span className="mono break">{t.burnTx}</span>.</p>
        </div>
      </div>
    </>
  );
}
