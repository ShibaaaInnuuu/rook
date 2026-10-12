import { useState } from "react";
import { useAccount } from "wagmi";
import { href, useEnv } from "../lib/env";
import { STAGING_RPC } from "../lib/chain";
import { STAGING_ID, STAGING_NAME, STAGING_CURRENCY } from "../lib/chains";
import { Section } from "../components/ui";

/** docs/phase6.md: the staging fork's own page. What it is, how to put it in a wallet, and the faucet. */
export function Staging() {
  const env = useEnv();
  const { address } = useAccount();
  const [to, setTo] = useState("");
  const [said, setSaid] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  if (!STAGING_RPC) return <><header className="pagehead"><h1>Staging</h1></header><div className="notice"><h2>This build does not know where the staging chain is</h2><p>It is switched on when the site is built with the staging chain's address. Nothing on this page works until then.</p></div></>;
  const target = (to || address || "").trim();
  const add = async () => {
    const eth = (window as unknown as { ethereum?: { request: (a: { method: string; params?: unknown[] }) => Promise<unknown> } }).ethereum;
    if (!eth) return setSaid({ ok: false, text: "No wallet was found in this browser." });
    try {
      await eth.request({ method: "wallet_addEthereumChain", params: [{ chainId: `0x${STAGING_ID.toString(16)}`, chainName: STAGING_NAME, rpcUrls: [STAGING_RPC], nativeCurrency: STAGING_CURRENCY }] });
      setSaid({ ok: true, text: `Added. Your wallet now knows chain ${STAGING_ID}, and calls its money sETH.` });
    } catch (e) { setSaid({ ok: false, text: String((e as Error).message ?? e).slice(0, 160) }); }
  };
  const drip = async () => {
    if (!/^0x[0-9a-fA-F]{40}$/.test(target)) return setSaid({ ok: false, text: "That is not an address." });
    setBusy(true);
    try {
      const r = await fetch(`${STAGING_RPC}/faucet`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ address: target }) });
      const j = (await r.json()) as { error?: string; addedWei?: string };
      setSaid(r.ok ? { ok: true, text: `Sent ${Number(BigInt(j.addedWei ?? "0") / 10n ** 16n) / 100} staging ETH. It is worth nothing anywhere.` } : { ok: false, text: j.error ?? `The faucet answered ${r.status}.` });
    } catch { setSaid({ ok: false, text: "The staging chain did not answer." }); }
    setBusy(false);
  };
  return (
    <>
      <header className="pagehead">
        <h1>Staging</h1>
        <p>A copy of Robinhood Chain at one block, run by us, with its own chain id ({STAGING_ID}) so that nothing you sign here can be replayed on the real chain. Pons's real contracts came with the copy; ours are deployed on top. Use it to try everything with money that is worth nothing.</p>
      </header>
      {env?.net !== "staging" ? <p className="notice notice--plain">You are not reading staging right now. <a href={`?net=staging${location.hash || "#/staging"}`}>Switch this site to staging</a>.</p> : null}
      <Section title="What is real here, and what is made up">
        <dl className="kv kv--wide">
          <dt>Real</dt><dd>Pons's factory, curve and fees. Every Rook contract. The 80 / 20 split, the cap, the bond, the queue. The holder snapshot and the tool that rebuilds it. The drand round behind a draw, and its check on chain. Your wallet's signature.</dd>
          <dt>Made up</dt><dd>Prices, the ETH to BTC quote, every Bitcoin receipt and every delivery. They are marked <span className="mono">staging-fixture</span> wherever they are recorded. No Bitcoin asset is bought or sent. <strong>A coin's market cap is a dollar figure built out of those invented prices</strong>: the supply and the dollars-per-ETH rate behind it are genuine chain reads — the fork carries Robinhood Chain's real state, so the rate is the real one — but the price it multiplies is not, so the dollars are not either.</dd>
          <dt>Cut short</dt><dd>The contracts make the public wait an hour before a purchase, fifteen minutes before a draw, and an hour before a delivery. On staging an operator cuts each wait short once it has been public for a couple of minutes, so a countdown here ends early. On the real chain nobody can do that.</dd>
          <dt>Played for you</dt><dd>The keeper, the purchaser and the attestor are one program beside the node. If a coin's creator does not choose what happens to an acquisition within a few minutes, it chooses a draw for them, which only a staging node can do.</dd>
        </dl>
      </Section>
      <Section title="1. Put the chain in your wallet">
        <button className="btn" onClick={() => void add()}>Add staging to my wallet</button>
        <p className="quiet">Or by hand: chain id <span className="mono">{STAGING_ID}</span>, RPC <span className="mono">{STAGING_RPC}</span>, currency sETH.</p>
      </Section>
      <Section title="2. Get staging ETH">
        <label className="field"><span>Address</span><input id="faucet-address" className="mono" value={to} placeholder={address ?? "0x…"} onChange={(e) => setTo(e.target.value)} spellCheck={false} autoComplete="off" /></label>
        <button className="btn" disabled={busy} onClick={() => void drip()}>{busy ? "Asking…" : "Send me staging ETH"}</button>
        <p className="quiet">Once a day for an address. It launches a coin and trades it many times over.</p>
        {said ? <p className={said.ok ? "good" : "error"} role="status">{said.text}</p> : null}
      </Section>
      <Section title="3. Try it">
        <ol className="steps">
          <li><a href={href("launch")}>Launch a coin</a> and pair it with a listing.</li>
          <li>On the coin's page, buy and sell on the curve from a second wallet or two, then press Harvest. Watch 80% arrive in the vault.</li>
          <li>When the vault can afford one item, a purchase is declared. Watch it on the <a href={href("listing", 0)}>listing's page</a>.</li>
          <li>On <a href={href("draws")}>Draws</a>, as the creator, choose how the acquisition is distributed; check the snapshot yourself; request the draw; claim; post a Bitcoin address.</li>
        </ol>
      </Section>
    </>
  );
}
