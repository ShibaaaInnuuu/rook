import { useMemo } from "react";
import { marked } from "marked";
import trustModel from "../content/trust-model.md?raw";

/** docs/trust-model.md, as it is in the repository, rendered and not rewritten. The text is ours and built into the page. */
export function Trust() {
  const html = useMemo(() => marked.parse(trustModel, { async: false, gfm: true }) as string, []);
  return (
    <>
      <header className="pagehead"><h1>What is trusted</h1><p>The repository's own trust model, word for word. If a sentence here is more comfortable than the contracts justify, that is a bug: say so.</p></header>
      <article className="prose scroll-tables" dangerouslySetInnerHTML={{ __html: html }} />
    </>
  );
}
