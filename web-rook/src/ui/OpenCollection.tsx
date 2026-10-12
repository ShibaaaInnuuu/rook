import type { SolCollection } from "../rook/sol/collections";
import { CANISTER_ON, useOpening } from "../rook/sol/hooksCanister";
import { OPENS_PER_DAY, requestCopy } from "../rook/sol/opening";
import { Pill } from "./bits";

/**
 * A collection that is not open yet (operator decision 2026-10-12, "we don't select, the people do"): "Not open yet: be the first", with
 * "Open it", which asks the canister (`coins_request_open`, anonymous; nothing to sign, Rook pays the ≈ 0.023 SOL); then the page polls
 * `coins_collections` until the collection is Open, saying its place in the queue and, when the canister has waited on it a while, why
 * (`coins_open_queue`, audit C4-12). A refused ask shows the canister's own Err text. Nothing is drawn once it is open, or before the
 * canister has answered. Under `?demo`
 * the demo's canister answers, and an opening runs through its stages in this browser. `compact`: one line, for the launch wizard's
 * review step.
 */
export function OpenCollection({ c, compact = false }: { c: SolCollection; compact?: boolean }) {
  const { opening, queue, request } = useOpening(c.wxSymbol);
  if (!CANISTER_ON || opening.kind === "open" || opening.kind === "unknown") return null;
  const t = requestCopy(opening, c.name, c.chain, queue);
  const waiting = opening.kind === "requested" || opening.kind === "opening" || request.isPending;
  return (
    <div className={`notice ${opening.kind === "refused" ? "notice--bad" : "notice--warn"} small`} role="status" aria-live="polite" style={compact ? { margin: ".5rem 0" } : undefined}>
      <p style={{ margin: 0 }}><strong>{t.title}.</strong> {t.body}</p>
      {t.action ? <p style={{ margin: ".5rem 0 0" }}>
        <button type="button" className="btn btn--small btn--primary" disabled={request.isPending} onClick={() => request.mutate()}>{request.isPending ? "Asking…" : t.action}</button>
        <span className="muted"> At most {OPENS_PER_DAY} open a day, in the order asked.</span>
      </p> : null}
      {waiting ? <p style={{ margin: ".5rem 0 0" }}><Pill tone="warn">{opening.kind === "opening" ? "opening" : "requested"}</Pill> <span className="muted">this page checks every few seconds</span></p> : null}
      {request.error ? <p className="error" style={{ margin: ".5rem 0 0" }}>{String((request.error as Error).message).slice(0, 300)}</p> : null}
    </div>
  );
}
