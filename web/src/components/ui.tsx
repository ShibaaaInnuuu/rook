import { useEffect, useState, type ReactNode } from "react";
import { ArrowUpRight, Check, Copy, Moon, Sun } from "@phosphor-icons/react";
import { href, useEnv } from "../lib/env";
import { short } from "../lib/format";

export function Figure({ label, value, unit, note, tone }: { label: string; value: ReactNode; unit?: string; note?: ReactNode; tone?: "warn" | "good" }) {
  return (
    <div className={`figure${tone ? ` figure--${tone}` : ""}`}>
      <dt>{label}</dt>
      <dd><span className="num">{value}</span>{unit ? <span className="unit"> {unit}</span> : null}</dd>
      {note ? <p className="figure__note">{note}</p> : null}
    </div>
  );
}

export const Section = ({ title, aside, children }: { title: string; aside?: ReactNode; children: ReactNode }) => (
  <section className="section"><header className="cut"><h2>{title}</h2>{aside ? <div className="cut__aside">{aside}</div> : null}</header>{children}</section>
);

export const Addr = ({ a }: { a: string }) => <span className="mono" title={a}>{short(a)}</span>;

/** An address that copies itself when pressed, and says so quietly. Not for use inside a link. */
export function CopyAddr({ a }: { a: string }) {
  const [said, setSaid] = useState(false);
  useEffect(() => { if (!said) return; const t = setTimeout(() => setSaid(false), 1400); return () => clearTimeout(t); }, [said]);
  return (
    <button type="button" className="copy" title={`Copy ${a}`} onClick={() => { void navigator.clipboard?.writeText(a).then(() => setSaid(true), () => undefined); }}>
      <span className="mono">{short(a)}</span>
      {said ? <><Check size={14} weight="bold" aria-hidden /><span className="copy__said" role="status">copied</span></> : <Copy size={14} aria-hidden />}
    </button>
  );
}

/** Light or dark, remembered on this device only. With no choice made, the system's preference decides. */
export function ThemeToggle() {
  const read = (): "light" | "dark" => (document.documentElement.dataset.theme as "light" | "dark" | undefined) ?? (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
  const [theme, setTheme] = useState<"light" | "dark">(read);
  const flip = () => {
    const next = theme === "dark" ? "light" : "dark";
    document.documentElement.dataset.theme = next;
    try { localStorage.setItem("etch-theme", next); } catch { /* a private window: the choice lasts for this visit */ }
    setTheme(next);
  };
  return <button type="button" className="iconbtn" onClick={flip} aria-label={theme === "dark" ? "Switch to the light theme" : "Switch to the dark theme"}>{theme === "dark" ? <Sun aria-hidden /> : <Moon aria-hidden />}</button>;
}
export const Ext = ({ href, children }: { href: string; children: ReactNode }) => <a href={href} target="_blank" rel="noreferrer noopener">{children}<ArrowUpRight className="ext" size="0.8em" aria-hidden /><span className="vh"> (opens in a new tab)</span></a>;

/** What every page shows instead of numbers when there is nothing deployed to read. */
export function NoDeployment() {
  return (
    <div className="notice" role="status">
      <h2>Nothing is deployed yet</h2>
      <p>The Rook contracts are not on Robinhood Chain. There are no vaults, no coins and no fees, so this page has no figures to show, and it will not invent any.</p>
      <p>The contracts and their tests are written and unaudited. What they enforce, and what is left to somebody's word, is on the <a href={href("trust")}>Trust page</a>, which does not depend on a deployment.</p>
      <p>To see what this page looks like with figures in it, <a href="?demo=sample#/">open a recorded demo</a>: the indexer's answers from a run on a local fork, where every Bitcoin value is a fixture. Every screen says so while you do. Developers can also run the keeper's end-to-end script against a fork on their own machine and open the site with <code>?demo</code>.</p>
    </div>
  );
}
/** A placeholder in the shape of what is coming: a figure, then two lines. */
export const Loading = ({ what }: { what: string }) => <div className="skel" role="status" aria-label={`Reading ${what}`}><i /><i /><i /></div>;
export const Failed = ({ what, error }: { what: string; error: unknown }) => <p className="error" role="alert">Could not read {what}: {String((error as Error)?.message ?? error).slice(0, 160)}</p>;

