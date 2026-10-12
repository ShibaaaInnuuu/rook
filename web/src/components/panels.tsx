import { useEffect, useMemo, useRef, useState } from "react";
import { href } from "../lib/env";
import { units } from "../lib/format";
import { Ext } from "./ui";
import { AssetArt, EntryArt } from "./Plate";
import { asOf, isRune, rankingLine, sats, vaultOf, vaultsByRef, vaultTag, type Catalogue, type CatalogueEntry,
  type VaultRef } from "../lib/catalogue";
import { freshKeys, keyOf, newestFirst } from "../lib/feed";
import type * as Api from "../api/types";

/** The five panels on the front page, each a working miniature of the page it opens. docs/phase9.md, section 2. */

const reduced = () => typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

/** How long ago, in the fewest words that are still true. */
export const ago = (s: number | null): string => {
  if (!s) return "time not known";
  const d = Math.floor(Date.now() / 1000) - s;
  if (d < 60) return "just now";
  if (d < 3600) return `${Math.round(d / 60)}m ago`;
  if (d < 172_800) return `${Math.round(d / 3600)}h ago`;
  return `${Math.round(d / 86_400)}d ago`;
};

/**
 * A figure that counts to its value when it changes, so a number that moved looks like it moved.
 * Tabular figures mean the count cannot jitter the layout, and with reduced motion it simply prints.
 */
export function Counter({ value, format }: { value: number; format?: (n: number) => string }) {
  const show = format ?? ((n: number) => Math.round(n).toLocaleString("en-US"));
  const [shown, setShown] = useState(value);
  const from = useRef(value);

  useEffect(() => {
    if (reduced() || value === from.current) {
      from.current = value;
      setShown(value);
      return;
    }
    const start = performance.now();
    const a = from.current;
    const span = value - a;
    let raf = 0;
    const tick = (t: number) => {
      const k = Math.min(1, (t - start) / 620);
      const eased = 1 - Math.pow(1 - k, 3);
      setShown(a + span * eased);
      if (k < 1) raf = requestAnimationFrame(tick);
      else from.current = value;
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [value]);

  return <span className="num">{show(shown)}</span>;
}

/** A clock that ticks, for a window that is closing. It ticks under reduced motion too: a countdown that does not tick is wrong. */
export function Ticking({ until, done }: { until: number; done?: string }) {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  useEffect(() => {
    const t = setInterval(() => setNow(Math.floor(Date.now() / 1000)), 1000);
    return () => clearInterval(t);
  }, []);
  const left = until - now;
  if (left <= 0) return <span className="num">{done ?? "closed"}</span>;
  const h = Math.floor(left / 3600);
  const m = Math.floor((left % 3600) / 60);
  const s = left % 60;
  return (
    <span className="num" role="timer">
      {h ? `${h}h ${String(m).padStart(2, "0")}m` : `${m}m ${String(s).padStart(2, "0")}s`}
    </span>
  );
}

// ------------------------------------------------------------------ 1. the strip

/**
 * The catalogue, drifting past: the real Bitcoin collections and runes a vault can hold.
 *
 * Every plate is stamped with that entry's OWN vault status — "vault", "paused", or "no vault yet" — rather than with a
 * claim about the strip it sits in, because everything here is meant to have a vault and most of it now does. A card
 * lifted out of the strip still tells the truth about itself. It stops on hover, on focus, and under reduced motion.
 * docs/phase9.md §8, §11.3.
 */
export function CatalogueStrip({ catalogue, vaults, limit = 16 }: {
  catalogue: Catalogue | undefined; vaults?: readonly VaultRef[]; limit?: number;
}) {
  const items = useMemo<CatalogueEntry[]>(() => [
    ...(catalogue?.collections ?? []).slice(0, limit),
    ...(catalogue?.runes ?? []).slice(0, Math.max(4, Math.round(limit / 3))),
  ], [catalogue, limit]);

  if (!items.length) {
    return <p className="empty">The catalogue could not be read, so there is nothing to show here. It is a file in this site, so this normally means it is missing from the build.</p>;
  }

  // Two copies make the loop seamless; the second is hidden from assistive tech so nothing is announced twice.
  const run = [...items, ...items];
  const by = vaultsByRef(vaults);
  const seconds = Math.max(40, items.length * 3.4);

  return (
    <div className="strip-rail" aria-label="Collections and runes">
      <ul className="strip-rail__track" style={{ "--run": `${seconds}s`, "--count": items.length } as React.CSSProperties}>
        {run.map((e, i) => (
          <li className="strip-rail__item" key={`${e.id}-${i}`} aria-hidden={i >= items.length ? "true" : undefined}>
            <a className="strip-rail__in" href={href("vaults", e.slug)} tabIndex={i >= items.length ? -1 : undefined}>
              <EntryArt entry={e} tag={vaultTag(vaultOf(e, by))} />
              <span className="strip-rail__name">{e.name}</span>
              <span className="strip-rail__note quiet">{isRune(e) ? `rune ${e.runeId}` : sats(e.floorSats)}</span>
            </a>
          </li>
        ))}
      </ul>
    </div>
  );
}

// ---------------------------------------------------------------- 2. the receipts

/**
 * The newest settled purchases, newest first, polled. Each row says what was paid, what it bought and when, and opens the
 * Bitcoin transaction that proves it. A row that was not here last time is cut in rather than appearing.
 */
export function ReceiptFeed({ rows, limit = 5 }: { rows: Api.Acquisition[]; limit?: number }) {
  const seen = useRef<Set<string> | null>(null);
  const newest = useMemo(() => newestFirst(rows, limit), [rows, limit]);
  const keys = newest.map(keyOf);
  const fresh = freshKeys(keys, seen.current);

  useEffect(() => {
    seen.current = new Set(keys);
  });

  if (!newest.length) {
    return (
      <p className="empty">
        No vault has bought anything yet. The first purchase appears here with what it paid, what it bought, and the Bitcoin
        transaction that settled it.
      </p>
    );
  }

  return (
    <ul className="feed">
      {newest.map((a, i) => {
        const key = keys[i];
        const proof = a.verify?.links?.[0];
        const what = a.kind === "rune"
          ? `${units(BigInt(a.amount || "0"), 0)} of ${a.assetRef}`
          : `inscription ${a.assetRef.slice(0, 8)}…${a.assetRef.slice(-3)}`;
        return (
          <li className={`feed__row${fresh.has(key) ? " feed__row--new" : ""}`} key={key}>
            <span className="feed__art"><AssetArt kind={a.kind} assetRef={a.assetRef} /></span>
            <span className="feed__say">
              <span className="feed__lead">
                <span className="num">{Number(a.satsPaid || 0).toLocaleString("en-US")}</span> sats bought {what}
              </span>
              <span className="feed__meta quiet">
                {a.coinSymbol ? <a href={href("coin", a.coin)}>{a.coinSymbol}</a> : "a coin"} fees
                {" · "}{ago(a.settledAt)}
                {a.delivery.state === "delivered" ? " · delivered" : a.delivery.overdue ? " · delivery overdue" : ""}
              </span>
            </span>
            <span className="feed__proof">
              {proof ? <Ext href={proof.url}>Bitcoin</Ext> : <span className="quiet">no link</span>}
            </span>
          </li>
        );
      })}
    </ul>
  );
}

// ------------------------------------------------------------------- 5. the rules

/** What the bytecode enforces, whatever anybody says. The one panel that can never be empty, which is the point. */
export function Rules() {
  const lines: [string, string, string][] = [
    ["the split", "80 / 20", "Eighty of every hundred of a coin's fees reach its vault. The number is in the bytecode, not a setting."],
    ["withdrawing", "no function", "A vault has no withdraw. Nothing in it can leave except by buying the one asset it names."],
    ["who may buy first", "the operator, for one hour", "After an hour any wallet may execute a declared purchase and claim the same reward."],
    ["the cap", "per vault, per hour", "A vault cannot spend more than its cap in an hour, and the cap expires rather than rolling over."],
    ["the challenge window", "15 minutes", "Any wallet may challenge a settlement inside it. Nothing is final until it closes."],
  ];
  return (
    <dl className="rules">
      {lines.map(([k, v, why]) => (
        <div className="rules__row" key={k}>
          <dt>{k}</dt>
          <dd className="rules__val mono">{v}</dd>
          <dd className="rules__why quiet">{why}</dd>
        </div>
      ))}
    </dl>
  );
}

/**
 * Where the catalogue's figures came from, when, and — first, because it is the thing a reader will otherwise assume —
 * what its order actually means. The sentence is the fetch's own record, not this component's belief about it, so a
 * curated list is never printed as a ranking and a file with no basis says it has none. docs/phase9.md §11.
 */
export function Provenance({ catalogue }: { catalogue: Catalogue | undefined }) {
  if (!catalogue) return null;
  return (
    <p className="quiet small">
      {rankingLine(catalogue)} Figures as of {asOf(catalogue.fetchedAt)}, fetched once and committed, never live. From{" "}
      {catalogue.attribution.map((a, i) => (
        <span key={a.name}>
          {i ? ", " : ""}
          <Ext href={a.url}>{a.name}</Ext>
        </span>
      ))}
      .
    </p>
  );
}
