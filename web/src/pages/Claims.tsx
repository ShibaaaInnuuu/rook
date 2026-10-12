import { useQueries } from "@tanstack/react-query";
import { useAccount } from "wagmi";
import { href } from "../lib/env";
import { countdown, when } from "../lib/format";
import { useNow } from "../lib/env";
import { Failed, Loading } from "../components/ui";
import { NeedsIndexer, PageHead } from "../components/catalog";
import { useDrawRows, useNeedsIndexer, useSource } from "../lib/catalog";
import type * as Api from "../api/types";

/** The states in which a draw can be waiting on a person. Anything else needs no address from anybody. */
const CANDIDATE = new Set<Api.DrawSummary["state"]>(["draw-resolved", "won", "delivered", "distribution-open", "distribution-rolled"]);

/** docs/phase7.md: there is no per-address endpoint, so the detail of every candidate draw is fetched and matched here. */
function useMine(address?: `0x${string}`) {
  const src = useSource();
  const draws = useDrawRows();
  const candidates = (draws.data?.rows ?? []).filter((d) => CANDIDATE.has(d.state));
  const details = useQueries({
    queries: candidates.map((d) => ({ queryKey: ["draw", src?.label, d.id], enabled: !!src && !!address, staleTime: 30_000,
      queryFn: () => src!.get<Api.DrawDetail>(`/v1/draws/${d.id}`) })),
  });
  const loading = draws.isLoading || details.some((q) => q.isLoading);
  const won = details.map((q) => q.data).filter((d): d is Api.DrawDetail =>
    !!d && !!address && d.draw?.winner?.toLowerCase() === address.toLowerCase());
  const distributions = details.map((q) => q.data).filter((d): d is Api.DrawDetail => !!d && d.type === "distribution" && (d.state === "distribution-open" || d.state === "distribution-rolled"));
  return { loading, error: draws.error, won, distributions, candidates: candidates.length, total: draws.data?.total ?? 0 };
}

export function Claims() {
  const { address } = useAccount();
  const now = useNow();
  const needsIndexer = useNeedsIndexer();
  const mine = useMine(address);

  return (
    <>
      <PageHead title="Claims">
        <p>An inscription or a rune bought here lives on Bitcoin, and no contract on this chain can send it there. So when you win one, it waits until you say where it should go. This page is what is waiting on you.</p>
      </PageHead>

      {needsIndexer ? <NeedsIndexer what="the list of draws" /> : !address ? (
        <div className="notice" role="status">
          <h2>Connect the wallet you hold coins with</h2>
          <p>Nothing is sent to us when you do: the page reads the draws that have already resolved and matches the winner's address against yours, in your browser.</p>
          <p className="quiet">Anything you have won that still needs an address from you shows up here. So does any rune distribution you can claim from.</p>
        </div>
      ) : mine.loading ? <Loading what="your claims" /> : mine.error ? <Failed what="draws" error={mine.error} /> : (
        <>
          {!mine.won.length && !mine.distributions.length ? (
            <div className="notice" role="status">
              <h2>Nothing is waiting on you</h2>
              <p>None of the {mine.candidates} resolved draw{mine.candidates === 1 ? "" : "s"} named <span className="mono">{address.slice(0, 6)}…{address.slice(-4)}</span> as its winner, and no open distribution needs an address from you.</p>
              <p className="quiet">Draws run on the coins you hold, not on anything you enter. <a href={href("draws")}>Open draws</a> shows what is running now.</p>
            </div>
          ) : null}

          {mine.won.map((d) => (
            <section className="lot" key={d.id}>
              <header>
                <h3>You won lot {d.listingId}-{d.acquisitionId}</h3>
                <p className="mono wrap quiet">{d.assetRef}</p>
              </header>
              {d.draw?.deliveredTxid ? (
                <p className="good">Delivered {when(d.draw.deliveredAt ?? 0)}. The Bitcoin transaction is <a className="mono" href={`https://mempool.space/tx/${d.draw.deliveredTxid}`} target="_blank" rel="noreferrer noopener">{d.draw.deliveredTxid.slice(0, 16)}…</a></p>
              ) : d.draw?.destination ? (
                <>
                  <p>Your address is recorded: <span className="mono wrap">{d.draw.destination}</span>. The keeper sends the asset and records the transaction against this lot.</p>
                  {d.draw.overdue ? <p className="error">This delivery is past its deadline. It is flagged on chain and stays flagged until it is sent.</p> : null}
                </>
              ) : (
                <>
                  <p><strong>This needs a Bitcoin address from you.</strong> Post it on the draw's own page. It locks an hour after it is set, so check it twice: an ordinary Bitcoin wallet can spend an inscription as if it were plain bitcoin.</p>
                  <p><a className="btn" href={href("draws")}>Name your address</a></p>
                </>
              )}
            </section>
          ))}

          {mine.distributions.map((d) => (
            <section className="lot" key={d.id}>
              <header>
                <h3>Rune distribution, lot {d.listingId}-{d.acquisitionId}</h3>
                <p className="quiet">{d.state === "distribution-rolled" ? "Rolled into the next distribution." : <>Open for claims{d.challengeEnds && d.challengeEnds > now ? <>, after the challenge window closes in {countdown(d.challengeEnds, now)}</> : null}.</>}</p>
              </header>
              <p>Every holder claims their own share with a proof from the published list. Amounts under the posted minimum wait and roll into the next one rather than paying more in Bitcoin fees than they are worth.</p>
              <p><a className="btn btn--quiet" href={href("draws")}>Claim on the draws page</a></p>
            </section>
          ))}

          {mine.total > mine.candidates ? <p className="quiet">Checked the {mine.candidates} draw{mine.candidates === 1 ? "" : "s"} that can involve a winner, of {mine.total} in total. The rest are open, voided or cancelled and need nothing from anyone.</p> : null}
        </>
      )}
    </>
  );
}
