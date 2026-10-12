import type { ReactNode } from "react";
import { useEnv } from "../lib/env";
import { useWrappedSource, type WSource } from "../lib/wrapped";
import { FixtureBanner, Loading, NothingDeployed } from "./bits";

/** The gate every wrapped-collection page stands behind: live when the indexer has collections, the fixture under ?demo=sample, else "nothing is deployed". */
export function Gate({ page, children }: { page: string; children: (src: WSource) => ReactNode }) {
  const env = useEnv(), s = useWrappedSource();
  if (env === null || s.isLoading) return <Loading what="where to read from" />;
  if (!s.data) return <NothingDeployed page={page} />;
  return <>{s.data.kind === "fixture" ? <FixtureBanner note={s.data.note} /> : null}{children(s.data)}</>;
}
