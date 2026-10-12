import type { RaffleDraw, WrappedCollection } from "../api/types";
import { useEnv, useNow } from "../lib/env";
import { short, when } from "../lib/format";
import { BEACON, RAFFLE_VAULT } from "../rook/abis";
import { claimRefusal, drandSignature, drawSteps, emptyLine, seedLine, type DrawStep } from "../rook/keyless";
import { PHASE1, pinnedAddress } from "../rook/phase";
import { bundledStack } from "../rook/stack";
import { Pill } from "./bits";
import { TxButton } from "./wallet";

/**
 * docs/audit/2026-09-26/fixes-W2-site.md §1: a raffle coin's draws, and the keyless steps anyone may take — pin a round after the
 * 15-minute window, resolve with drand's signature, claim for the winner, void when a draw is stuck. The keeper announces and opens;
 * nothing here needs a key but the visitor's own wallet, and a claim is checked against the draw as the chain holds it first.
 */
const WORD: Record<RaffleDraw["stage"], string> = { announced: "announced", open: "open", pinned: "pinned", resolved: "resolved", claimed: "claimed", cancelled: "cancelled", voided: "voided", empty: "no winner" };
const TONE: Record<RaffleDraw["stage"], "good" | "warn" | "bad" | "live"> = { announced: "warn", open: "warn", pinned: "warn", resolved: "live", claimed: "good", cancelled: "bad", voided: "bad", empty: "warn" };
const LABEL: Record<DrawStep["id"], string> = { pin: "Pin a round", close: "Close (no winner)", resolve: "Resolve", claim: "Claim", void: "Void" };

export function Raffles({ col, coin, fixture }: { col: WrappedCollection; coin: string; fixture: boolean }) {
  const now = useNow();
  const draws = (col.draws ?? []).filter((d) => d.coin.toLowerCase() === coin.toLowerCase());
  const paused = col.keeper?.paused ?? null;
  // E-07: in Phase 1 the vault a draw step is sent to is the stack file's; a draw the indexer places at another vault is not offered
  const pin = PHASE1 ? pinnedAddress(bundledStack.stack, col.slug, "raffleVault", col.contracts.raffleVault) : null;
  if (!draws.length) return <p className="small muted">No draw for this coin yet. The keeper announces one when the coin's fees have bought one whole NFT; the snapshot time is then picked by a drand round that does not exist yet.</p>;
  return (
    <div className="stack">
      {paused ? <p className="notice notice--bad small" role="status"><strong>The keeper is paused.</strong> No draw is opened or pinned until the Safe unpauses it; draws already pinned still resolve and pay.</p> : null}
      {draws.map((d) => <Draw key={`${d.vault}-${d.id}`} d={d} now={now} paused={paused} vault={pin ? (pin.address && pin.address.toLowerCase() === d.vault.toLowerCase() ? pin.address : null) : d.vault} why={pin?.why ?? (pin && pin.address && pin.address.toLowerCase() !== d.vault.toLowerCase() ? `This draw is at ${d.vault}, not the stack file's raffle vault ${pin.address}; nothing is sent to it.` : null)} fixture={fixture} />)}
    </div>
  );
}

function Draw({ d, now, paused, vault, why, fixture }: { d: RaffleDraw; now: number; paused: boolean | null; vault: `0x${string}` | null; why: string | null; fixture: boolean }) {
  const env = useEnv();
  const live = env && env.mode === "live" ? env : null;
  const steps = drawSteps(d, now, paused);
  const request = (s: DrawStep) => async () => {
    const id = BigInt(d.id), v = vault!;
    if (s.id === "pin" || s.id === "close") return { address: v, abi: RAFFLE_VAULT, functionName: "requestDraw", args: [id] };
    if (s.id === "void") return { address: v, abi: RAFFLE_VAULT, functionName: "voidDraw", args: [id] };
    if (s.id === "resolve") {
      let sig: `0x${string}` = "0x";
      if (live) {
        const beacon = await live.client.readContract({ address: v, abi: RAFFLE_VAULT, functionName: "beacon" });
        const r = await live.client.readContract({ address: beacon, abi: BEACON, functionName: "randomnessOf", args: [BigInt(d.round!)] });
        if (/^0x0{64}$/.test(r)) sig = await drandSignature(d.round!);
      }
      return { address: v, abi: RAFFLE_VAULT, functionName: "resolve", args: [id, sig] };
    }
    const c = d.claim!;
    if (live) {
      const onChain = await live.client.readContract({ address: v, abi: RAFFLE_VAULT, functionName: "draw", args: [id] });
      const bad = claimRefusal({ stage: Number(onChain.stage), root: onChain.root, totalTickets: onChain.totalTickets, winningTicket: onChain.winningTicket }, c);
      if (bad) throw new Error(bad);
    }
    return { address: v, abi: RAFFLE_VAULT, functionName: "claim", args: [id, c.holder, BigInt(c.first), BigInt(c.end), c.proof] };
  };
  return (
    <div className="panel panel--tight">
      <div className="row"><strong>Draw {d.id}</strong> <Pill tone={TONE[d.stage]}>{WORD[d.stage]}</Pill></div>
      <dl className="kv small" style={{ marginTop: ".5rem" }}>
        <dt>Seed round</dt><dd>{d.seedRound ?? "—"}{d.announcedAt ? ` (announced ${when(d.announcedAt)})` : ""}</dd>
        <dt>Snapshot time</dt><dd>{d.snapshotAt ? `${when(d.snapshotAt)}: the last block at or before it` : d.seedOrigin !== undefined && d.seedOrigin !== null ? `draw ${d.seedOrigin}'s, fixed by the carried seed round when the draw is opened` : "fixed by the seed round when the draw is opened"}</dd>
        {seedLine(d) ? <><dt>Seed</dt><dd>{seedLine(d)}</dd></> : null}
        {emptyLine(d) ? <><dt>Outcome</dt><dd>{emptyLine(d)}</dd></> : null}
        {d.root ? <><dt>Root</dt><dd className="mono break">{d.root}</dd><dt>Tickets</dt><dd className="num">{d.totalTickets}</dd></> : null}
        {d.uri ? <><dt>Published list</dt><dd className="break">{fixture ? d.uri : <a href={/^https:\/\//.test(d.uri) ? d.uri : undefined} target="_blank" rel="noreferrer noopener">{d.uri}</a>}{d.published ? <> — <span className={d.published.status === "matches" ? "good" : "bad"}>{d.published.status === "matches" ? "rebuilds to the root (with the count and the time)" : d.published.status === "differs" ? "does NOT rebuild to the root" : "could not be read"}</span>{d.published.why ? ` (${d.published.why})` : ""}</> : null}</dd></> : null}
        {d.round !== null ? <><dt>Pinned round</dt><dd>{d.round}</dd></> : null}
        {d.winningTicket !== null ? <><dt>Winning ticket</dt><dd className="num">{d.winningTicket}{d.claim ? ` — ${short(d.claim.holder)}` : ""}</dd></> : null}
        {d.winner ? <><dt>Paid to</dt><dd className="mono break">{d.winner}</dd></> : null}
      </dl>
      {steps.map((s) => s.ok
        ? <div key={s.id} style={{ marginTop: ".5rem" }}><TxButton label={s.id === "claim" && d.claim ? `Claim for ${short(d.claim.holder)}` : LABEL[s.id]} primary={false} disabledWhy={why ?? (!vault ? "No raffle vault to send to." : null)} request={request(s)} /></div>
        : <p key={s.id} className="small muted" style={{ margin: ".5rem 0 0" }}>{LABEL[s.id]}: {s.why ?? ""}{s.at ? ` From about ${when(s.at)}.` : ""}</p>)}
    </div>
  );
}
