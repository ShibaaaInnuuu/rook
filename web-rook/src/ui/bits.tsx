import { useEffect, useState, type ReactNode } from "react";
import { href } from "../lib/env";
import type { ReserveStatus } from "../api/types";
import { GLOSSARY, wrappedLine } from "../rook/words";

/** The small, shared pieces. Everything here is presentation; the data comes in as props. */

export const Loading = ({ what }: { what: string }) => <p className="muted small" role="status">Reading {what}…</p>;
export const Failed = ({ what, error }: { what: string; error: unknown }) => (
  <p className="error small" role="alert">Could not read {what}: {String((error as Error)?.message ?? error).slice(0, 160)}</p>
);

export function Fig({ label, value, unit, big, className }: { label: string; value: ReactNode; unit?: string; big?: boolean; className?: string }) {
  return (
    <div className={`fig${big ? " fig--big" : ""}${className ? ` ${className}` : ""}`}>
      <span className="fig__label">{label}</span>
      <span className="fig__value">{value}{unit ? <small>{unit}</small> : null}</span>
    </div>
  );
}

/** What a stat with no data says, instead of a bare "—" (design review 2026-09-27). */
export const NotYet = () => <span className="na">not available yet</span>;

/** A wrapped token's name, explained where it stands: the title says what one whole token is. */
export const Wrapped = ({ symbol, bitcoin }: { symbol: string; bitcoin: boolean }) => <abbr className="wrapped" title={wrappedLine(symbol, bitcoin)}>{symbol}</abbr>;

/** Provenance a person can read first, and the developer's record (sources, refs, ids) behind an info disclosure. Nothing is dropped. */
export function Provenance({ line, children, className }: { line: ReactNode; children: ReactNode; className?: string }) {
  return (
    <div className={`prov small muted${className ? ` ${className}` : ""}`}>
      <span>{line}</span>
      <details className="info"><summary aria-label="Where this comes from" title="Where this comes from"><span aria-hidden="true">i</span></summary><div className="info__body">{children}</div></details>
    </div>
  );
}

export const Pill = ({ tone, children }: { tone?: "live" | "good" | "warn" | "bad"; children: ReactNode }) => <span className={`pill${tone ? ` pill--${tone}` : ""}`}>{children}</span>;

const STATUS_WORD: Record<ReserveStatus, string> = { green: "Verified", red: "Mismatch", unverified: "Unverified" };
export const Status = ({ status }: { status: ReserveStatus }) => <span className={`status status--${status}`}>{STATUS_WORD[status]}</span>;
export const StatusPill = ({ status }: { status: ReserveStatus }) => <Pill tone={status === "green" ? "good" : status === "red" ? "bad" : "warn"}>{STATUS_WORD[status]}</Pill>;

/** The fixture label, on every page that shows one. On a phone the note past its first sentence waits behind "What is invented", so the
 *  page's own first screen is not the label; on a wider screen all of it shows. Nothing of the note is dropped. */
export function FixtureBanner({ note }: { note: string }) {
  const [open, setOpen] = useState(false);
  const body = note.replace(/^FIXTURE\.\s*/, "");
  const cut = body.search(/\.\s/);
  const lead = cut > 0 ? body.slice(0, cut + 1) : body, rest = cut > 0 ? body.slice(cut + 2) : "";
  return (
    <p className={`banner banner--fixture${open ? " banner--open" : ""}`} role="status"><strong>Fixture — not deployed</strong>{lead}{rest ? <> <span className="banner__rest">{rest}</span> <button type="button" className="banner__more" aria-expanded={open} onClick={() => setOpen(!open)}>{open ? "Less" : "What is invented"}</button></> : null} <a href="?#/">Leave</a></p>
  );
}

/** Nothing configured and no fixture asked for: the page says so, and offers the fixture. */
export function NothingDeployed({ page }: { page: string }) {
  return (
    <div className="notice notice--warn">
      <h2 style={{ fontSize: "var(--t1)" }}>Nothing is deployed yet</h2>
      <p>The contracts behind this page have no public deployment, and this build was given no indexer to read. Nothing here invents a number.</p>
      <p style={{ margin: 0 }}><a className="btn btn--small" href={`?demo=sample#/${page}`}>See the fixture</a> <span className="muted small">built from the design documents' worked examples, and labelled as such on every page</span></p>
    </div>
  );
}

/** A "?"-safe link: in a fixture every id is invented, so the link is printed as text and leads nowhere. */
export const Proof = ({ href: to, fixture, children }: { href: string | null; fixture: boolean; children: ReactNode }) =>
  fixture || !to ? <span className="mono break">{children}</span> : <a className="mono break" href={to} target="_blank" rel="noreferrer noopener">{children}</a>;

export function ThemeToggle() {
  const [theme, setTheme] = useState<"dark" | "light" | null>(() => (typeof document === "undefined" ? null : (document.documentElement.dataset.theme as "dark" | "light" | undefined) ?? null));
  useEffect(() => {
    if (theme) document.documentElement.dataset.theme = theme; else delete document.documentElement.dataset.theme;
    try { if (theme) localStorage.setItem("rook-theme", theme); else localStorage.removeItem("rook-theme"); } catch { /* no storage */ }
  }, [theme]);
  const dark = theme ? theme === "dark" : (typeof matchMedia !== "undefined" && matchMedia("(prefers-color-scheme: dark)").matches);
  return (
    <button className="btn btn--quiet theme" type="button" aria-label={dark ? "Switch to light" : "Switch to dark"} title={dark ? "Light" : "Dark"} onClick={() => setTheme(dark ? "light" : "dark")}>
      <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.6">
        {dark ? <path d="M8 2.5a5.5 5.5 0 1 0 5.5 5.5A4.5 4.5 0 0 1 8 2.5z" /> : <><circle cx="8" cy="8" r="3" /><path d="M8 1v2M8 13v2M1 8h2M13 8h2M3 3l1.4 1.4M11.6 11.6 13 13M3 13l1.4-1.4M11.6 4.4 13 3" /></>}
      </svg>
    </button>
  );
}

export const Section = ({ title, eyebrow, children, aside }: { title: string; eyebrow?: string; children: ReactNode; aside?: ReactNode }) => (
  <section className="block">
    <div className="row" style={{ marginBottom: "1rem" }}>
      <div>{eyebrow ? <p className="eyebrow">{eyebrow}</p> : null}<h2 style={{ margin: 0 }}>{title}</h2></div>
      {aside ? <div className="right">{aside}</div> : null}
    </div>
    {children}
  </section>
);

export const ago = (s: number | null, now: number): string => {
  if (!s) return "—";
  const d = Math.max(0, now - s);
  return d < 60 ? `${d}s ago` : d < 3_600 ? `${Math.floor(d / 60)} min ago` : d < 86_400 ? `${Math.floor(d / 3_600)} h ago` : `${Math.floor(d / 86_400)} d ago`;
};

export const linkTo = href;

/** The site's words against the design documents' (src/rook/words.ts): the documents are rendered as they were written, so this says once what their older words mean. */
export function Glossary() {
  return (
    <details className="glossary small">
      <summary>The documents' words, in this site's</summary>
      <p className="muted" style={{ margin: ".5rem 0" }}>The documents are shown as written. Where they use an older word, this site says:</p>
      <dl className="kv kv--left">{GLOSSARY.map(([site, doc]) => <div key={site} style={{ display: "contents" }}><dt>{doc}</dt><dd>{site}</dd></div>)}</dl>
    </details>
  );
}
