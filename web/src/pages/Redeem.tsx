import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useAccount } from "wagmi";
import { href, useEnv, useNow } from "../lib/env";
import { useTicket, useWrappedCollection, type WSource } from "../lib/wrapped";
import { redeemDestinationProblem } from "../lib/redeem";
import { NFT_WRAPPER_REDEEM, WRAPPER_REDEEM } from "../lib/abisV3";
import { countdown, eth, when } from "../lib/format";
import { DRAND } from "../lib/chain";
import { Loading, Section } from "../components/ui";
import { TxButton } from "../components/wallet";
import { WxGate } from "../components/WxGate";
import { Proof, short, ticketStage } from "../components/wx";
import type { Ticket, WrappedCollection } from "../api/types";

/**
 * docs/v5/ux-and-reserves.md §1, "Redeem an item". Optional and separate from trading: burn one whole token for one random item,
 * delivered to an address of yours on the item's own chain; follow the ticket to delivery, or refund it if it is late.
 */
export function RedeemPage({ slug, ticket }: { slug?: string; ticket?: string }) {
  return <WxGate title="Redeem an item">{(src) => slug ? <Redeem slug={slug} ticket={ticket} src={src} /> : <p>Pick a collection on the <a href={href("collection")}>collections page</a>.</p>}</WxGate>;
}

function Redeem({ slug, ticket, src }: { slug: string; ticket?: string; src: WSource }) {
  const q = useWrappedCollection(slug);
  if (q.isLoading) return <Loading what="the collection" />;
  if (!q.data) return <div className="notice"><h2>No such collection</h2><p>Nothing is configured under “{slug}”.</p></div>;
  const c = q.data;
  return ticket !== undefined ? <TicketPage c={c} id={Number(ticket)} src={src} /> : <Form c={c} src={src} />;
}

function Form({ c, src }: { c: WrappedCollection; src: WSource }) {
  const env = useEnv();
  const { address } = useAccount();
  const [dest, setDest] = useState("");
  const [track, setTrack] = useState("");
  const live = env && (env.mode === "live" || env.mode === "demo") ? env : null;
  const abi = c.kind === "nft" ? NFT_WRAPPER_REDEEM : WRAPPER_REDEEM;
  const balance = useQuery({ queryKey: ["wx-balance", c.wrapper, address], enabled: !!live && !!address,
    queryFn: () => live!.client.readContract({ address: c.wrapper, abi, functionName: "balanceOf", args: [address!] }) as Promise<bigint> });
  const problem = redeemDestinationProblem(c, dest);
  const short1 = balance.data !== undefined && balance.data < 10n ** 18n ? `You hold ${eth(balance.data, 4)} ${c.symbol}; a redemption burns exactly one whole token.` : null;
  const where = c.custody.kind === "evm" ? c.custody.chainName : "Bitcoin";
  return (
    <>
      <header className="pagehead">
        <h1>Redeem an item</h1>
        <p>Burn one whole {c.symbol} for one real item from <a href={href("collection", c.slug)}>{c.name}</a>, sent to your own {c.kind === "nft" ? `address on ${where}` : "Bitcoin taproot address"}. You do not need this to trade.</p>
      </header>
      <Section title="How the item is chosen">
        <p>At random, by a <a href={DRAND} target="_blank" rel="noreferrer noopener">drand</a> round that does not exist yet when you burn, so nobody — including us — can choose which item you get.</p>
        <p className="quiet">Delivery is due within 7 days of the draw. If it has not arrived by then, anyone can refund the ticket: one whole {c.symbol} goes back to you and the item goes back into the set.</p>
      </Section>
      <Section title="Where to send it">
        <label className="tkt__amount wx-dest">
          <span className="tkt__label">{c.kind === "nft" ? `Your address on ${where} (0x…)` : "Your Bitcoin taproot address (bc1p…)"}</span>
          <input className="mono" autoComplete="off" spellCheck={false} value={dest} onChange={(e) => setDest(e.target.value)} placeholder={c.kind === "nft" ? "0x…" : "bc1p…"} />
        </label>
        {dest && problem ? <p className="error">{problem}</p> : null}
        <p className="quiet small">It cannot be changed after you burn. {c.kind === "nft" ? `Make sure you control this address on ${where}; it can be a different one from the wallet you burn from.` : "A wallet that is not taproot-aware may not show an inscription."}</p>
        <TxButton label={`Burn 1 ${c.symbol} for a random item`} doing="Burning…" disabledWhy={problem ?? short1}
          request={() => ({ address: c.wrapper, abi, functionName: "redeem", args: [dest.trim()] as never })} />
      </Section>
      <Section title="Track a ticket">
        <div className="field-inline">
          <input className="mono" inputMode="numeric" value={track} onChange={(e) => setTrack(e.target.value.replace(/\D/g, ""))} placeholder="ticket number" aria-label="Ticket number" />
          <a className="btn btn--quiet" href={track ? href("redeem", c.slug, track) : undefined} aria-disabled={!track}>Track</a>
        </div>
        {c.openTickets.length ? <ul className="wx-activity">{c.openTickets.map((t) => <li key={t.id}><a href={href("redeem", c.slug, t.id)}>Ticket {t.id}</a> <span className="quiet small">{ticketStage(t, Math.floor(Date.now() / 1000))}</span></li>)}</ul> : null}
        {src.kind === "fixture" ? <p className="quiet small">In this fixture, tickets 0, 1 and 2 exist on the frogs.</p> : null}
      </Section>
    </>
  );
}

const STEPS: { stage: Ticket["stage"] | "burned"; say: string }[] = [
  { stage: "burned", say: "Burned — the drand round that will choose your item is pinned" },
  { stage: "resolved", say: "Drawn — your item is chosen and the 7-day delivery clock runs" },
  { stage: "delivered", say: "Delivered — the item is at your address" },
];

function TicketPage({ c, id, src }: { c: WrappedCollection; id: number; src: WSource }) {
  const t = useTicket(c.slug, id), now = useNow(), fixture = src.kind === "fixture";
  const abi = c.kind === "nft" ? NFT_WRAPPER_REDEEM : WRAPPER_REDEEM;
  if (t.isLoading) return <Loading what="the ticket" />;
  if (!t.data) return <div className="notice"><h2>No ticket {id}</h2><p>{c.name} has no redemption with that number. <a href={href("redeem", c.slug)}>Back</a>.</p></div>;
  const k = t.data, reached = k.stage === "pinned" ? 0 : k.stage === "resolved" ? 1 : k.stage === "delivered" ? 2 : -1;
  const overdue = k.stage === "resolved" && k.deliverBy !== null && k.deliverBy <= now;
  const stuck = k.stage === "pinned" && k.burnedAt !== null && now - k.burnedAt > 24 * 3600;
  return (
    <>
      <header className="pagehead">
        <h1>Ticket {k.id}</h1>
        <p><a href={href("collection", c.slug)}>{c.name}</a> · {ticketStage(k, now)}</p>
      </header>
      {k.stage === "refunded" ? <div className="notice notice--plain"><p>Refunded: it was not delivered in time, so one whole {c.symbol} went back to its owner and the item went back into the set.</p></div> : (
        <ol className="wx-steps">
          {STEPS.map((s, i) => <li key={s.stage} className={i <= reached ? "on" : ""}>{s.say}{i === 1 && k.itemRef ? <> — <span className="mono">{c.kind === "nft" ? `#${k.itemRef}` : short(k.itemRef)}</span></> : null}</li>)}
        </ol>
      )}
      <dl className="kv kv--wide">
        <dt>Destination</dt><dd className="mono">{k.toBurn ? "burned for good (no one can move it)" : k.destination}</dd>
        <dt>drand round</dt><dd className="num">{k.round}</dd>
        <dt>Burned</dt><dd>{k.burnedAt ? when(k.burnedAt) : "—"} · <Proof href={null} fixture={fixture}>{short(k.burnTx)}</Proof></dd>
        {k.deliverBy ? <><dt>Deliver by</dt><dd className={overdue ? "warn-text" : undefined}>{when(k.deliverBy)}{k.stage === "resolved" ? ` (${overdue ? "overdue" : `in ${countdown(k.deliverBy, now)}`})` : ""}</dd></> : null}
        {k.deliveryTx ? <><dt>Delivery</dt><dd><Proof href={k.deliveryUrl} fixture={fixture}>{short(k.deliveryTx)}</Proof></dd></> : null}
      </dl>
      {overdue ? (
        <Section title="Refund">
          <p>The deadline has passed without a delivery. Anyone may press this; one whole {c.symbol} goes back to the ticket's owner. The contract also waits for Robinhood Chain's own block count, so it may refuse for a little longer than the clock above.</p>
          <TxButton label={`Refund ticket ${k.id}`} doing="Refunding…" request={() => ({ address: c.wrapper, abi, functionName: "refund", args: [BigInt(k.id)] })} />
        </Section>
      ) : null}
      {stuck ? (
        <Section title="Redraw">
          <p>drand has not published this ticket's round for a day. Anyone may pin a fresh one; the contract refuses if the round did appear.</p>
          <TxButton label={`Pin a new round for ticket ${k.id}`} doing="Pinning…" request={() => ({ address: c.wrapper, abi, functionName: "redraw", args: [BigInt(k.id)] })} />
        </Section>
      ) : null}
    </>
  );
}
