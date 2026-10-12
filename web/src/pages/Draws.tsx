import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useAccount } from "wagmi";
import { href, useEnv, useNow } from "../lib/env";
import { useAcquisitions, useCoin, useListings, useLots, useVault, type Acquisition, type Distribution, type Listing, type Lot } from "../lib/data";
import { DISTRIBUTOR } from "../lib/abis";
import { BITCOIN_TX, DRAND, INSCRIPTION } from "../lib/chain";
import {Addr, Ext, Failed, Loading, NoDeployment, Section } from "../components/ui";
import { TxButton } from "../components/wallet";
import { LOT_MODE, LOT_STAGE, countdown, units, when } from "../lib/format";
import { checkTaproot } from "../lib/bech32m";
import { destinationProblem } from "../lib/destination";

/** The file tools/snapshot writes. Only the fields this page reads. */
type Snapshot = { inputs: { coin: string; block: string; kind: string; total?: string }; root: string; totalTickets?: string;
  entries: { holder: string; first?: string; end?: string; units?: string; proof: `0x${string}`[] }[] };

function useSnapshot(uri: string, demo: boolean) {
  // https anywhere; plain http only for a staging chain run on this machine
  const url = uri.startsWith("local:") ? (demo ? `/local/${uri.slice(6)}` : null) : /^https:\/\//.test(uri) || /^http:\/\/(127\.0\.0\.1|localhost)(:\d+)?\//.test(uri) ? uri : null;
  return useQuery({ queryKey: ["snapshot", url ?? uri], enabled: !!url, staleTime: Infinity, refetchInterval: false,
    queryFn: async () => (await (await fetch(url!)).json()) as Snapshot });
}

function Destination({ label, request }: { label: string; request: (address: string) => Parameters<typeof TxButton>[0]["request"] }) {
  const [value, setValue] = useState("");
  const check = value ? checkTaproot(value) : null;
  return (
    <div className="form form--inline">
      <label htmlFor={`dest-${label}`}>Your Bitcoin taproot address (bc1p…)
        <input id={`dest-${label}`} className="mono" value={value} onChange={(e) => setValue(e.target.value.trim())} spellCheck={false} autoComplete="off" aria-invalid={check ? !check.ok : undefined} />
        {check && !check.ok ? <small className="error">{check.why}</small> : <small>Checked here before your wallet is asked for anything, and again by the contract. Use a wallet that understands inscriptions and runes: an ordinary Bitcoin wallet can spend them as if they were plain bitcoin.</small>}
      </label>
      <TxButton label={label} disabledWhy={destinationProblem(value)} request={request(value)} />
    </div>
  );
}

function Verify({ coin, lot, snap, kind, command }: { coin: string; lot: { root: string; uri: string }; snap?: Snapshot; kind: string; command?: string }) {
  const cmd = command ? `cd tools/snapshot && npm ci && ${command}` : `cd tools/snapshot && npm ci && npm run snapshot -- --coin ${coin} --block ${snap?.inputs.block ?? "<the block in the published file>"} --kind ${kind}${kind === "pro-rata" ? ` --total ${snap?.inputs.total ?? "<total>"}` : ""} --exclusions exclusions.json`;
  return (
    <details className="verify" open>
      <summary>Verify this snapshot yourself</summary>
      <p>The holder list is the keeper's word. No contract checks it. Rebuild it from public Transfer logs and compare the root; the tool is deterministic, so the same inputs give the same bytes.</p>
      <pre className="mono"><code>{cmd}</code></pre>
      <p>It must print <span className="mono wrap">root {lot.root}</span>. If it prints anything else, the published list is wrong: say so publicly before the window closes, because the owner can cancel it only until then.</p>
      <p className="quiet">Published list: <span className="mono wrap">{lot.uri}</span>. What the tool cannot prove: that the keeper chose the block it promised.</p>
    </details>
  );
}

function LotCard({ listing, a, lot, dist }: { listing: Listing; a: Acquisition; lot: Lot; dist: Distribution }) {
  const env = useEnv()!;
  const now = useNow();
  const { address } = useAccount();
  const demo = env.mode === "demo";
  const d = env.mode !== "none" ? env.deployment : null;
  const snap = useSnapshot(dist.open ? dist.uri : lot.uri, demo);
  const args = [BigInt(listing.id), BigInt(a.id)] as const;
  if (!d) return null;
  const me = address?.toLowerCase();
  const paying = useCoin(lot.mode === 0 && !lot.ended ? a.coin : undefined);
  const iAmCreator = !!me && paying.data?.creator.toLowerCase() === me;
  const [mode, setMode] = useState(listing.kind === "rune" ? 6 : 3);
  const [named, setNamed] = useState("");
  const ticketEntry = snap.data && lot.stage === 3 ? snap.data.entries.find((e) => e.first !== undefined && BigInt(e.first) <= lot.winningTicket && lot.winningTicket < BigInt(e.end!)) : undefined;
  const shareEntry = snap.data && dist.open && me ? snap.data.entries.find((e) => e.holder.toLowerCase() === me && e.units !== undefined) : undefined;
  return (
    <article className="lot">
      <header>
        <h3>{listing.kind === "rune" ? <><span className="num">{units(a.amount, listing.divisibility)}</span> {listing.name}</> : <Ext href={INSCRIPTION(a.assetRef)}><span className="mono">{a.assetRef.slice(0, 12)}…</span></Ext>}</h3>
        <p className="quiet">Acquisition {a.id} of <a href={href("listing", listing.id)}>{listing.name}</a>, paid for by <a href={href("coin", a.coin)}><Addr a={a.coin} /></a></p>
        <p><span className="tag">{LOT_MODE[lot.mode]}</span> {lot.stage ? <span className="tag tag--good">{LOT_STAGE[lot.stage]}</span> : null}{lot.overdue ? <span className="tag tag--warn">delivery overdue</span> : null}{lot.ended ? <span className="tag tag--warn">{lot.ended}</span> : null}</p>
      </header>

      {lot.ended === "voided" ? <p className="quiet">This draw resolved, nobody could prove the winning ticket for seven days, and the owner voided it. The asset stays in the vault until the coin's creator chooses again.</p> : null}
      {lot.ended === "cancelled" ? <p className="quiet">The snapshot behind this was cancelled inside its challenge window. Nothing was drawn or paid from it.</p> : null}
      {lot.mode === 0 && !lot.ended ? (iAmCreator ? (
        <div className="choose">
          <p>You created the coin that paid for this, so you choose what happens to it. Every choice but <em>hold</em> is final.</p>
          <label className="field"><span>What happens</span>
            <select id={`mode-${listing.id}-${a.id}`} value={mode} onChange={(e) => setMode(Number(e.target.value))}>
              {listing.kind === "rune" ? <option value={6}>Share it among holders, in proportion to their balances</option> : null}
              <option value={3}>A draw, with tickets by balance</option>
              <option value={4}>A draw, one ticket a wallet</option>
              <option value={5}>Give it to one address I name</option>
              <option value={1}>Hold it in the vault for now</option>
              <option value={2}>Never deliver it: it stays in the vault for good</option>
            </select></label>
          {mode === 5 ? <label className="field"><span>The address</span><input id={`named-${listing.id}-${a.id}`} className="mono" value={named} onChange={(e) => setNamed(e.target.value)} placeholder="0x…" spellCheck={false} /></label> : null}
          <TxButton label="Choose" disabledWhy={mode === 5 && !/^0x[0-9a-fA-F]{40}$/.test(named) ? "Name an address." : null}
            request={() => ({ address: d.distributor, abi: DISTRIBUTOR, functionName: "choose", args: [...args, mode, mode === 5 ? (named as `0x${string}`) : "0x0000000000000000000000000000000000000000"] })} />
        </div>) : <p className="quiet">The coin's creator has not chosen what happens to this. Only they can.{env.net === "staging" ? " On staging, a draw is chosen for them after a few minutes." : ""}</p>) : null}

      {lot.stage === 1 ? (<>
        <p>{lot.challengeEnds > now ? <>The holder snapshot is open to challenge for<strong className="countdown num" role="timer">{countdown(lot.challengeEnds, now)}</strong>No draw can be requested until then.</> : "The challenge window has closed. Anyone may now pin the draw to a drand round that does not exist yet."} <span className="quiet"><span className="num">{lot.totalTickets.toLocaleString("en-US")}</span> tickets.</span></p>
        <Verify coin={a.coin} lot={lot} snap={snap.data} kind={lot.mode === 4 ? "per-wallet" : "weighted"} command={lot.command} />
        {lot.challengeEnds <= now ? <TxButton label="Request the draw" request={() => ({ address: d.distributor, abi: DISTRIBUTOR, functionName: "requestDraw", args })} /> : null}
      </>) : null}

      {lot.stage === 2 ? (<>
        <p>Pinned to drand round <span className="num">{String(lot.round)}</span>, which {when(lot.pinnedAt)} had not been produced. Once pinned it cannot be changed or cancelled. Anyone may deliver its signature; the result is the same whoever does.</p>
        <TxButton label="Fetch the round from drand and resolve" doing="Resolving…" request={async () => {
          const r = await fetch(`${DRAND}/public/${lot.round}`);
          if (!r.ok) throw new Error("drand has not published that round yet. Try again in a few seconds.");
          const j = (await r.json()) as { signature: string };
          return { address: d.distributor, abi: DISTRIBUTOR, functionName: "resolve", args: [...args, `0x${j.signature}`] };
        }} />
        <p className="quiet">The signature is verified on chain against drand's public key. A wrong one reverts.</p>
      </>) : null}

      {lot.stage === 3 ? (<>
        <p>Winning ticket <span className="num">{lot.winningTicket.toLocaleString("en-US")}</span> of <span className="num">{lot.totalTickets.toLocaleString("en-US")}</span>.</p>
        {ticketEntry ? <><p>It falls in <Addr a={ticketEntry.holder} />'s range (<span className="num">{ticketEntry.first} to {ticketEntry.end}</span>). Anyone may submit the proof.</p>
          <TxButton label="Prove the winner" request={() => ({ address: d.distributor, abi: DISTRIBUTOR, functionName: "claimWin", args: [...args, ticketEntry.holder as `0x${string}`, BigInt(ticketEntry.first!), BigInt(ticketEntry.end!), ticketEntry.proof] })} /></>
          : <p className="quiet">{snap.isLoading ? "Reading the published ticket list…" : <>The published list (<span className="mono wrap">{lot.uri}</span>) could not be read from here. Find the range holding the winning ticket in it and call <span className="mono">claimWin</span> with its proof.</>}</p>}
      </>) : null}

      {lot.stage >= 4 ? (<>
        <dl className="kv"><dt>Winner</dt><dd><Addr a={lot.winner} /></dd>
          <dt>Destination</dt><dd>{lot.destination ? <span className="mono wrap">{lot.destination}</span> : <span className="quiet">not posted yet</span>}</dd>
          <dt>Delivery</dt><dd>{lot.stage === 5 ? <Ext href={BITCOIN_TX(lot.deliveryTxid)}><span className="mono">{lot.deliveryTxid.slice(2, 10)}…{lot.deliveryTxid.slice(-6)}</span></Ext> : lot.destinationAt ? <>due by {when(lot.destinationAt + 7 * 86400)}; after that anyone may flag it overdue</> : "waits for the winner's address"}</dd></dl>
        {lot.stage === 4 && me === lot.winner.toLowerCase() ? <Destination label={lot.destination ? "Correct my address" : "Post my address"} request={(addr) => () => ({ address: d.distributor, abi: DISTRIBUTOR, functionName: "setDestination", args: [...args, addr] })} /> : null}
        {lot.stage === 4 && me !== lot.winner.toLowerCase() ? <p className="quiet">Only the winner can name where it goes. The asset is on Bitcoin, so it cannot be sent to a Robinhood Chain address.</p> : null}
        {lot.stage === 5 ? <p className="quiet">The delivery transaction is the keeper's word. Check it on Bitcoin{demo || env.mode === "sample" ? " (in this demo it is made up and will not be found)" : ""}.</p> : null}
      </>) : null}

      {dist.open ? (<>
        <p>Pro rata: <span className="num">{units(dist.total, listing.divisibility)}</span> units, <span className="num">{units(dist.claimed, listing.divisibility)}</span> claimed. {dist.rolled ? "Closed; the unclaimed part waits for the coin's next distribution." : dist.challengeEnds > now ? <>Open to challenge for<strong className="countdown num" role="timer">{countdown(dist.challengeEnds, now)}</strong></> : <>Claims close {when(dist.claimsEnd)}.</>}</p>
        {dist.rolled ? null : <Verify coin={a.coin} lot={dist} snap={snap.data} kind="pro-rata" command={lot.command} />}
        {shareEntry && dist.challengeEnds <= now && !dist.rolled ? <TxButton label={`Claim my ${units(BigInt(shareEntry.units!), listing.divisibility)} units`} request={() => ({ address: d.distributor, abi: DISTRIBUTOR, functionName: "claimShare", args: [...args, shareEntry.holder as `0x${string}`, BigInt(shareEntry.units!), shareEntry.proof] })} /> : null}
        {me && snap.data && !shareEntry ? <p className="quiet">Your connected address is not in this list.</p> : null}
      </>) : null}
    </article>
  );
}

function ListingLots({ listing }: { listing: Listing }) {
  const vault = useVault(listing.vault, listing.id);
  const acquisitions = useAcquisitions(listing.vault, vault.data?.acquisitions ?? 0, listing.id);
  const lots = useLots(listing, acquisitions.data ?? []);
  if (vault.isLoading || acquisitions.isLoading || lots.isLoading) return <Loading what={listing.name} />;
  if (lots.error) return <Failed what={listing.name} error={lots.error} />;
  if (!lots.data?.length) return null;
  return <>{lots.data.map((x) => <LotCard key={`${listing.id}-${x.acquisition.id}`} listing={listing} a={x.acquisition} lot={x.lot} dist={x.distribution} />)}</>;
}

export function Draws() {
  const env = useEnv();
  const listings = useListings();
  return (
    <>
      <header className="pagehead"><h1>Draws and distributions</h1><p>Everything a vault has bought, and who it goes to. A draw's winner comes from a drand round pinned before it existed; nobody picks it and nobody can re-roll it. The holder list behind it is another matter: that is the keeper's word, published for fifteen minutes before it can be used.</p></header>
      {env === null ? <Loading what="the deployment" /> : env.mode === "none" ? <NoDeployment /> : listings.isLoading ? <Loading what="listings" /> : listings.error ? <Failed what="listings" error={listings.error} /> : (
        <Section title="Acquisitions">
          {listings.data?.length ? <div className="lots">{listings.data.map((l) => <ListingLots key={l.id} listing={l} />)}</div> : null}
          <p className="quiet">If nothing is listed above, no vault has a receipted purchase yet. The first one appears here with its snapshot, its fifteen public minutes, and its draw.</p>
        </Section>
      )}
    </>
  );
}
