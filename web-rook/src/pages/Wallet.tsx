import { useAccount } from "wagmi";
import { useQuery } from "@tanstack/react-query";
import type { WalletProfile } from "../rook/apiTypes";
import { href, useEnv, useNow } from "../lib/env";
import { EXPLORER_ADDRESS, EXPLORER_TX, explorerFor } from "../lib/explorer";
import { short, when } from "../lib/format";
import { useRookSource, useBoard } from "../rook/data";
import { ageText, capText, priceText } from "../rook/boardlogic";
import { Board } from "../ui/Board";
import { CoinFace } from "../ui/Identicon";
import { Fig, FixtureBanner, Loading, NothingDeployed, ago } from "../ui/bits";

/** `#/wallet/<address>`: what an address launched, holds, traded lately and won. An address, not a person; nothing here groups wallets. */
export function WalletPage({ address }: { address?: string }) {
  const env = useEnv(), { address: mine } = useAccount();
  const src = useRookSource(), board = useBoard();
  const a = address && /^0x[0-9a-fA-F]{40}$/.test(address) ? address : mine;
  const q = useQuery({ queryKey: ["wallet", src.data?.kind, a], enabled: !!src.data && !!a, retry: 0, refetchInterval: src.data?.kind === "live" ? 15_000 : false, queryFn: () => src.data!.get<WalletProfile>(`/v1/rook/wallets/${a}`) });
  const now = useNow();
  if (env === null || src.isLoading) return <Loading what="the wallet" />;
  if (!src.data) return <NothingDeployed page={`wallet/${address ?? ""}`} />;
  if (!a) return <div className="pagehead"><h1>Your wallet</h1><p className="lede">Connect a wallet with the button above, or open any address as <span className="mono">#/wallet/0x…</span>, to see what it launched, holds, traded and won.</p></div>;
  const explorer = explorerFor(env && "chain" in env ? env.chain.id : null, env?.mode === "sample");
  const p = q.data;
  return (
    <>
      {src.data.kind === "fixture" ? <FixtureBanner note={src.data.note} /> : null}
      <header className="pagehead rise">
        <p className="eyebrow">Wallet{mine && a.toLowerCase() === mine.toLowerCase() ? " · yours" : ""}</p>
        <h1 className="mono break" style={{ fontSize: "clamp(1.1rem, 3vw, 1.8rem)", fontFamily: "var(--mono)", fontWeight: 500, letterSpacing: 0 }}>{explorer ? <a href={EXPLORER_ADDRESS(explorer, a)} target="_blank" rel="noreferrer noopener">{a}</a> : a}</h1>
        <p className="small muted">One address, one page: nothing here groups a trader's several wallets, and a custodian holding for many people is one address.</p>
      </header>
      {q.isLoading ? <Loading what="its record" /> : q.error ? <p className="small warn">This wallet is not known here: {String((q.error as Error).message).slice(0, 120)}</p> : p ? (
        <div className="stack-lg rise rise-1">
          <div className="figs">
            <Fig label="Launched" value={p.launches.length} />
            <Fig label="Holds" value={p.holdings.length} unit="coins" />
            <Fig label="Trades" value={p.trades.length >= 50 ? "50+" : p.trades.length} />
            <Fig label="Raffles won" value={p.wins.length} />
            <Fig label="Value held" value={p.holdings.every((h) => h.valueEth === null) ? "—" : capText(p.holdings.reduce((s, h) => s + (h.valueEth ?? 0), 0), board.data?.usdPerEth ?? null)} />
          </div>
          <section>
            <p className="eyebrow">Holdings</p>
            {p.holdings.length ? <div className="scroll"><table className="tape"><thead><tr><th>Coin</th><th className="r">Balance</th><th className="r">Share</th><th className="r">Value</th></tr></thead>
              <tbody>{p.holdings.map((h) => <tr key={h.coin.address}><td><a className="board__coin" href={href("coin", h.coin.address)}><CoinFace address={h.coin.address} logoUrl={h.coin.logoUrl} name={h.coin.name} size={28} /><span><strong>{h.coin.symbol}</strong><small>{h.coin.name}</small></span></a></td><td className="r num">{(Number(BigInt(h.balanceWei)) / 1e18).toLocaleString("en-US", { maximumFractionDigits: 0 })}</td><td className="r num">{h.share === null ? "—" : `${(h.share * 100).toFixed(2)}%`}</td><td className="r num">{h.valueEth === null ? "—" : capText(h.valueEth, board.data?.usdPerEth ?? null)}</td></tr>)}</tbody></table></div>
              : <p className="small muted">Holds nothing on the board.</p>}
          </section>
          <section>
            <p className="eyebrow">Recent trades</p>
            {p.trades.length ? <div className="scroll"><table className="tape"><thead><tr><th>Time</th><th>Side</th><th>Coin</th><th className="r">Price</th><th className="r">ETH</th><th>Tx</th></tr></thead>
              <tbody>{p.trades.map((t) => <tr key={`${t.coin.address}-${t.id}`}><td className="num muted" title={t.at ? when(t.at) : ""}>{ago(t.at, now)}</td><td><span className={t.side === "buy" ? "good" : "bad"}>{t.side}</span> <span className="muted small">{t.venue}</span></td><td><a href={href("coin", t.coin.address)}>{t.coin.symbol}</a></td><td className="r num">{priceText(t.price)}</td><td className="r num">{(Number(BigInt(t.quoteWei)) / 1e18).toFixed(4)}</td><td className="num">{explorer ? <a href={EXPLORER_TX(explorer, t.tx)} target="_blank" rel="noreferrer noopener">{short(t.tx)}</a> : <span className="muted">{short(t.tx)}</span>}</td></tr>)}</tbody></table></div>
              : <p className="small muted">No trade on the board's coins.</p>}
          </section>
          <section>
            <p className="eyebrow">Raffle wins</p>
            {p.wins.length ? <ul className="activity">{p.wins.map((w) => <li key={w.tx}><time>{ago(w.at, now)}</time><span><span className="kind kind--raffle-won">raffle won</span> · {w.nest}: {w.text}</span></li>)}</ul> : <p className="small muted">No raffle won yet.</p>}
          </section>
          <section>
            <p className="eyebrow">Launches</p>
            {p.launches.length ? <Board rows={p.launches} usdPerEth={board.data?.usdPerEth ?? null} initialTab="all" compact /> : <p className="small muted">Launched nothing{p.launches.length === 0 && p.at ? ` as of ${ageText(p.at, now)} ago` : ""}.</p>}
          </section>
        </div>) : null}
    </>
  );
}
