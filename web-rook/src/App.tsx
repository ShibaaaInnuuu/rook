import { Suspense, lazy, useEffect } from "react";
import { href, useRoute } from "./lib/env";
import { GITHUB_URL, SITE_URL, X_HANDLE } from "./config";
import { BOTTOM, KNOWN, NAV, ROBINHOOD_ONLY, SOLANA, STATIC, sectionOf } from "./rook/routes";
import { SolWalletButton, SolWalletProvider } from "./rook/sol/wallet";
import { DEMO, DEMO_BANNER } from "./rook/sol/demo";
import { SOL } from "./rook/sol/siteConfig";
import { Mark } from "./ui/Mark";
import { Loading, ThemeToggle } from "./ui/bits";
import { NotOnSolana } from "./ui/sol";
import { SolHome } from "./pages/SolHome";
import { Lost } from "./pages/Lost";

// The Solana pages (and the tracked-collection list) are one lazy chunk; the static pages another. The Robinhood design's wagmi
// pages (src/AppPages.tsx) are no longer routed: #/tracked used to land there and, with no indexer in the build, said "Nothing is
// deployed yet" instead of listing anything.
const SolPages = lazy(() => import("./SolPages"));
const StaticPages = lazy(() => import("./StaticPages"));

function Shell() {
  const route = useRoute();
  const at = route[0] ?? "";
  const section = sectionOf(at);
  useEffect(() => { document.title = at ? `${at[0].toUpperCase()}${at.slice(1).replace(/-/g, " ")} — Rook` : "Rook — memecoins that buy the floor, on Solana"; }, [at]);
  return (
    <>
      <a className="skip" href="#main">Skip to content</a>
      {DEMO ? <p className="banner banner--slim banner--fixture banner--demo" role="status"><strong>Demo</strong>{DEMO_BANNER}: coins and trades are invented; nothing can be traded. <a href={`${location.pathname}${location.hash}`}>Leave the demo</a></p>
        : !SOL.ready ? <p className="banner banner--slim" role="status"><strong>Before launch</strong>Launching and trading open once the custodian has opened the first collections on Solana. <a href={href("docs", "what-is-rook")}>What is Rook</a></p> : null}
      <header className="masthead">
        <div className="page masthead__in">
          <a className="wordmark rise" href={href()} aria-label="Rook, home"><Mark />Rook</a>
          <nav className="masthead__nav" aria-label="Main"><ul>{NAV.map(([to, label]) => <li key={to}><a href={href(to)} aria-current={section === to ? "page" : undefined}>{label}</a></li>)}</ul></nav>
          <div className="masthead__tools">
            <ThemeToggle />
            <a className="btn btn--small btn--primary masthead__launch" href={href("launch")}>Launch a coin</a>
            <SolWalletButton />
          </div>
        </div>
      </header>
      <nav className="subnav" aria-label="Main, small screens">{NAV.map(([to, label]) => <a key={to} href={href(to)} aria-current={section === to ? "page" : undefined}>{label}</a>)}</nav>
      <main id="main" className="page">
        {!KNOWN.has(at) ? <Lost /> : ROBINHOOD_ONLY.has(at) ? <NotOnSolana at={at} /> : at === "" ? <SolHome /> : STATIC.has(at) ? <Suspense fallback={<Loading what="the page" />}><StaticPages route={route} /></Suspense>
          : SOLANA.has(at) ? <Suspense fallback={<Loading what="the page" />}><SolPages route={route} /></Suspense> : <Lost />}
      </main>
      <nav className="bottomnav" aria-label="Main, phone">{BOTTOM.map(([to, label]) => <a key={to} href={href(to)} aria-current={(to === "" ? at === "" : section === to) ? "page" : undefined}>{label}</a>)}</nav>
      <footer className="foot">
        <div className="page foot__in">
          <div>
            <p className="foot__mark" aria-hidden="true">Rook</p>
            <p><strong>On Solana. Rook deploys no programs of its own;</strong> coins launch and trade on Meteora's Dynamic Bonding Curve, a program Meteora operates. Not affiliated with Meteora, Jupiter, Phantom, Solflare, Backpack or the collections' creators.</p>
            <p>Nothing here is financial advice. A coin launched here can go to zero, and most do. This site sets no cookies and records nothing about you. It reads Solana through an RPC endpoint; it serves collection logos itself and loads coin images from where they are hosted, sending no referrer. The only app is <a href={SITE_URL}>{SITE_URL.replace(/^https?:\/\//, "")}</a>; check the address before you sign anything.</p>
            <p className="foot__social">{X_HANDLE ? <><a href={`https://x.com/${X_HANDLE}`} rel="noreferrer">@{X_HANDLE} on X</a> · </> : null}<a href={GITHUB_URL} rel="noreferrer" target="_blank">GitHub ↗</a></p>
          </div>
          <div>
            <h2>Trade</h2>
            <ul><li><a href={href("coins")}>Coins</a></li><li><a href={href("launch")}>Launch a coin</a></li><li><a href={href("collections")}>Collections</a></li><li><a href={href("tracked")}>Tracked collections</a></li><li><a href={href("reserves")}>Reserves</a></li></ul>
          </div>
          <div>
            <h2>Read</h2>
            <ul><li><a href={href("how-it-works")}>How it works</a></li><li><a href={href("docs")}>Docs</a></li><li><a href={href("trust")}>What is trusted</a></li><li><a href={href("terms")}>Terms</a></li><li><a href={href("risk")}>Risk</a></li><li><a href={href("privacy")}>Privacy</a></li></ul>
          </div>
        </div>
      </footer>
    </>
  );
}

export function App() { return <SolWalletProvider><Shell /></SolWalletProvider>; }
