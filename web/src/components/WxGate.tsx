import type { ReactNode } from "react";
import { useEnv } from "../lib/env";
import { useWrappedSource, type WSource } from "../lib/wrapped";
import { FixtureBanner } from "./wx";
import { Loading } from "./ui";

/**
 * Every wrapped-collection page goes through this: it waits for the source, says plainly when there is none (nothing of the v3/v4
 * stack is deployed), and puts the FIXTURE banner on top of anything read from the fixture set.
 */
export function WxGate({ title, children }: { title: string; children: (src: WSource) => ReactNode }) {
  const env = useEnv();
  const s = useWrappedSource();
  if (env === null || s.isLoading) return <Loading what="the collections" />;
  if (!s.data) return (
    <>
      <header className="pagehead"><h1>{title}</h1></header>
      <div className="notice" role="status">
        <h2>Nothing is deployed yet</h2>
        <p>The wrapped collections, their launch curves and their pools are written and tested, and not on Robinhood Chain. There are no items in custody to prove, no coins to trade and no tickets to track, and this page will not invent any.</p>
        <p>To see what it looks like with figures in it, <a href={`?demo=sample${location.hash || "#/reserves"}`}>open the fixture</a>: the worked examples of the design documents, run through the real reserve computation and labelled FIXTURE on every screen.</p>
      </div>
    </>
  );
  return <>{s.data.kind === "fixture" ? <FixtureBanner note={s.data.note} /> : null}{children(s.data)}</>;
}
