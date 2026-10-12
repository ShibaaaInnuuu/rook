import { useState } from "react";
import { SITE_URL } from "../config";
// SITE_URL is site.config.ts's: https://userook.fun, the canonical domain (www.userook.fun serves the same site).

/** Copies the coin's userook.fun link (the OG meta on index.html describes the site; the hash names the coin). Says what it did, and when it could not. */
export function Share({ address, symbol }: { address: string; symbol: string }) {
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");
  const url = `${SITE_URL}/#/coin/${address}`;
  const copy = async () => {
    try { await navigator.clipboard.writeText(url); setState("copied"); } catch { setState("failed"); }
    setTimeout(() => setState("idle"), 2_000);
  };
  return <button type="button" className="btn btn--quiet btn--small" onClick={() => void copy()} title={url} aria-label={`Copy the link to ${symbol}`}>{state === "copied" ? "Link copied" : state === "failed" ? "Could not copy: " + url : "Share"}</button>;
}
