import { EnvProvider, href, useEnv, useRoute } from "./lib/env";
import { Suspense, lazy, useState, useEffect } from "react";
import { Loading, ThemeToggle } from "./components/ui";
import { Home } from "./pages/Home";
import { Lost } from "./pages/Lost";
import { X_HANDLE } from "./config";
import { STAGING_ID } from "./lib/chains";
import { CANONICAL, KNOWN, LEGAL, NAV, sectionOf } from "./lib/routes";

const AppPages = lazy(() => import("./AppPages"));
const WalletIsland = lazy(() => import("./WalletIsland"));
const SearchBox = lazy(() => import("./components/Search").then((m) => ({ default: m.Search })));
// the legal pages read no chain and need no wallet code: their own small chunk
const Legal = lazy(() => import("./pages/Legal").then((m) => ({ default: ({ which }: { which: string }) => which === "terms" ? <m.Terms /> : which === "risk" ? <m.Risk /> : <m.Privacy /> })));
/** Holds the wallet button's place while its chunk loads, so nothing in the masthead moves. */
const WalletPlace = () => <button className="btn btn--quiet" disabled>Connect wallet</button>;
/** On the landing page the wallet code (wagmi and viem, the heaviest thing on the site) is not fetched until somebody reaches for
 *  the button; the button that is shown until then is the same size, so nothing moves. Every other page needs it at once. */
function WalletSlot({ eager }: { eager: boolean }) {
  const [armed, setArmed] = useState<false | "hover" | "press">(false);
  if (!eager && !armed) return <button className="btn btn--quiet" onPointerEnter={() => setArmed("hover")} onFocus={() => setArmed("hover")} onClick={() => setArmed("press")}>Connect wallet</button>;
  return <Suspense fallback={<WalletPlace />}><WalletIsland connectNow={armed === "press"} /></Suspense>;
}

const EXPLORER = "https://robinhoodchain.blockscout.com/address";

function Shell() {
  const env = useEnv();
  const route = useRoute();
  const at = route[0] ?? "";
  const section = sectionOf(at);

  // An old name still answers, and then quietly becomes the current one, so a link in the deployed site or the docs never
  // 404s and nobody ends up bookmarking two URLs for one page.
  useEffect(() => {
    const to = CANONICAL[at];
    if (to) location.replace(`${location.pathname}${location.search}#/${[to, ...route.slice(1)].join("/")}`);
  }, [at, route]);

  const d = env && env.mode !== "none" && "deployment" in env && env.deployment.chainId ? env.deployment : null;
  const cut = (a: string) => `${a.slice(0, 6)}…${a.slice(-4)}`;
  return (
    <>
      <a className="skip" href="#main">Skip to content</a>
      {env?.net === "staging" && env.mode !== "none" ? <p className="banner banner--staging" role="status"><strong>STAGING.</strong> A copy of Robinhood Chain that we run (chain {STAGING_ID}). Its sETH is worthless; prices, Bitcoin receipts and deliveries are made up; waits are cut short. <a href={href("staging")}>Wallet, faucet and what is real</a> · <a href="?#/">Leave</a></p> : null}
      {env?.net === "test" && env.mode !== "none" ? <p className="banner banner--test" role="status"><strong>TEST DEPLOYMENT.</strong> Real contracts on Robinhood Chain, tiny limits, unaudited, holding small amounts of real money. Not the launch. <a href="?#/">Leave</a></p> : null}
      {env?.mode === "none" && env.net ? <p className="banner banner--demo" role="status"><strong>{env.net === "staging" ? "Staging" : "Test deployment"} is not available.</strong> {env.why} <a href="?#/">Back to the site</a></p> : null}
      {env?.mode === "demo" ? <p className="banner banner--demo" role="status"><strong>Demo.</strong> Reading a fork on this machine (chain 31337). The receipts, prices and deliveries in it are fixtures. Nothing here is real or worth anything.</p> : null}
      {env?.mode === "sample" ? <p className="banner banner--demo" role="status"><strong>{env.source.label === "synthetic" ? "Demo, invented events." : "Demo, recorded."}</strong> {env.source.note} <a href="?#/">Leave the demo</a></p> : null}
      <header className="masthead">
        <div className="wrap-page masthead__in">
          <a className="wordmark" href={href()} aria-label="Rook, home">Rook<span aria-hidden="true" className="wordmark__cut" /></a>
          <nav aria-label="Main"><ul>{NAV.map(([to, label]) => <li key={to}><a href={href(to)} aria-current={section === to ? "page" : undefined}>{label}</a></li>)}</ul></nav>
          <div className="masthead__tools">
            <Suspense fallback={null}><SearchBox /></Suspense>
            <ThemeToggle />
            <a className="btn btn--small" href={href("launch")}>Launch</a>
            <WalletSlot eager={at !== "" && KNOWN.has(at) && !LEGAL.has(at)} />
          </div>
        </div>
      </header>
      <main id="main" className="wrap-page">
        {!KNOWN.has(at) ? <Lost /> : at === "" ? <Home /> : LEGAL.has(at) ? <Suspense fallback={<Loading what="the page" />}><Legal which={at} /></Suspense> : <Suspense fallback={<Loading what="the page" />}><AppPages route={route} /></Suspense>}
      </main>
      <footer className="foot">
        <div className="wrap-page foot__in">
          <p className="foot__mark" aria-hidden="true">Rook</p>
          <div className="foot__say">
            <p><strong>Unaudited.</strong> Not affiliated with Robinhood Markets, Pons, Nosh, UniSat or Magic Eden.</p>
            <p>Nothing here is financial advice. A coin launched here can go to zero, and most do. This site sets no cookies, loads nothing from third parties, and records nothing about you.</p>
            {X_HANDLE ? <p><a href={`https://x.com/${X_HANDLE}`} rel="noreferrer">@{X_HANDLE} on X</a></p> : null}
          </div>
          <nav className="foot__cols" aria-label="Footer">
            <div>
              <h2>What you can do</h2>
              <ul>
                <li><a href={href("explore")}>Explore coins</a></li>
                <li><a href={href("vaults")}>Vaults</a></li>
                <li><a href={href("receipts")}>Receipts</a></li>
                <li><a href={href("analytics")}>Analytics</a></li>
                <li><a href={href("launch")}>Launch a coin</a></li>
              </ul>
            </div>
            <div>
              <h2>The protocol</h2>
              {d ? (
                <ul className="foot__addrs">
                  <li><span>Registry</span><a className="mono" href={`${EXPLORER}/${d.registry}`} rel="noreferrer">{cut(d.registry)}</a></li>
                  <li><span>Launcher</span><a className="mono" href={`${EXPLORER}/${d.launcher}`} rel="noreferrer">{cut(d.launcher)}</a></li>
                  <li><span>Distributor</span><a className="mono" href={`${EXPLORER}/${d.distributor}`} rel="noreferrer">{cut(d.distributor)}</a></li>
                  <li><span>Pons factory</span><a className="mono" href={`${EXPLORER}/${d.ponsFactory}`} rel="noreferrer">{cut(d.ponsFactory)}</a></li>
                </ul>
              ) : <p className="quiet">Nothing is deployed, so there are no addresses to publish. They appear here the day there are.</p>}
            </div>
            <div>
              <h2>Read</h2>
              <ul>
                <li><a href={href("docs")}>How it works</a></li>
                <li><a href={href("trust")}>What is trusted</a></li>
                <li><a href={href("terms")}>Terms of use</a></li>
                <li><a href={href("risk")}>Risk disclosure</a></li>
                <li><a href={href("privacy")}>Privacy</a></li>
                {env?.mode === "none" ? <><li><a href="?demo=sample#/">See a recorded demo</a></li><li><a href="?demo=states#/draws">Every state a page can show</a></li></> : null}
              </ul>
            </div>
          </nav>
        </div>
      </footer>
    </>
  );
}
export const App = () => <EnvProvider><Shell /></EnvProvider>;
