import { useEffect, useMemo, useState } from "react";
import { Marked, type Tokens } from "marked";
import { href } from "../lib/env";
import { HOW_IT_WORKS } from "../rook/routes";
import { HowFlow } from "../ui/HowFlow";

/**
 * The public docs: short user documentation written for the site, one Markdown file per page under src/content/docs/
 * (`NN-<slug>.md`, NN is the order). Each file opens with a small front matter block — title, summary, group — and the body
 * uses plain Markdown plus two callouts: a blockquote whose first line is `[!NOTE]` or `[!RISK]`.
 * Links between pages are written `#/docs/<slug>` or `#/docs/<slug>/<heading-id>`.
 */
const FILES = import.meta.glob("../content/docs/*.md", { query: "?raw", import: "default", eager: true }) as Record<string, string>;

/** The left nav's groups, in order. A page names its group in its front matter. */
export const GROUPS = ["Start here", "Using Rook", "Trust"] as const;

type Page = { slug: string; title: string; summary: string; group: string; body: string };

export function parsePage(file: string, text: string): Page {
  const slug = file.replace(/^.*\//, "").replace(/\.md$/, "").replace(/^\d+-/, "");
  const m = /^---\n([\s\S]*?)\n---\n/.exec(text);
  const meta: Record<string, string> = {};
  for (const line of (m?.[1] ?? "").split("\n")) {
    const i = line.indexOf(":");
    if (i > 0) meta[line.slice(0, i).trim()] = line.slice(i + 1).trim();
  }
  return { slug, title: meta.title ?? slug, summary: meta.summary ?? "", group: meta.group ?? GROUPS[0], body: m ? text.slice(m[0].length) : text };
}

const byFile = Object.keys(FILES).sort().map((f) => parsePage(f, FILES[f]));
/** Nav order: by group, then by file number. Previous / next follow it. */
const PAGES: Page[] = GROUPS.flatMap((g) => byFile.filter((p) => p.group === g));

/** Links that pointed at the old design documents land on the page that now covers the subject. */
const MOVED: Record<string, string> = {
  "site-design": "how-it-works", "floor-module": "how-it-works", "floor-feed": "how-it-works", "ux-and-reserves": "how-it-works", "offer-handins": "how-it-works",
  "collections-catalog": "backing", "policies-and-fees": "fees",
  "launch-gates": "security", "custody-hardening": "security", "custody-threat-model": "security", "trust-model": "security",
  // the earlier model's deposit page went with the Solana programs it paid for (canister-only Rook deploys none)
  "deploy-fund": "fees",
};

const slugify = (html: string) => html.replace(/<[^>]+>/g, "").replace(/&[a-z#0-9]+;/gi, "").toLowerCase().trim().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
/** Heading HTML as plain text for the "On this page" list (React escapes it again). */
const plain = (html: string) => html.replace(/<[^>]+>/g, "").replace(/&#39;/g, "'").replace(/&quot;/g, '"').replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");
const CALLOUT: Record<string, string> = { NOTE: "Note", RISK: "Risk" };

function render(page: Page): { html: string; toc: { id: string; text: string }[] } {
  const toc: { id: string; text: string }[] = [];
  const md = new Marked({ gfm: true });
  md.use({
    renderer: {
      heading(this: { parser: { parseInline: (t: Tokens.Heading["tokens"]) => string } }, { tokens, depth }: Tokens.Heading) {
        const text = this.parser.parseInline(tokens);
        const id = slugify(text);
        if (depth === 2) toc.push({ id, text: plain(text) });
        return `<h${depth} id="${id}" class="docs__h"><a class="docs__anchor" href="${href("docs", page.slug, id)}" aria-label="Link to this section">#</a>${text}</h${depth}>\n`;
      },
      blockquote(this: { parser: { parse: (t: Tokens.Blockquote["tokens"]) => string } }, { tokens }: Tokens.Blockquote) {
        const body = this.parser.parse(tokens);
        const m = /^<p>\[!(NOTE|RISK)\]\s*/.exec(body);
        if (!m) return `<blockquote>${body}</blockquote>\n`;
        const kind = m[1].toLowerCase();
        const rest = body.slice(m[0].length).replace(/^<\/p>\s*/, "");
        const content = rest.startsWith("<") ? rest : `<p>${rest}`;
        return `<aside class="docs__callout docs__callout--${kind}" role="note"><p class="docs__callout-label">${CALLOUT[m[1]]}</p>${content}</aside>\n`;
      },
    },
  });
  let html = md.parse(page.body, { async: false }) as string;
  html = html
    .replace(/<table>/g, '<div class="docs__table"><table>').replace(/<\/table>/g, "</table></div>")
    // in-site links keep the query string (?net=…) the visitor came with
    .replace(/href="#\//g, `href="${location.search}#/`)
    .replace(/<a href="https?:\/\/(?!userook\.fun)/g, (s) => s.replace("<a ", '<a target="_blank" rel="noopener noreferrer" '));
  return { html, toc };
}

/** The heading a link points at: #/docs/<slug>/<id>. */
const anchorOf = () => location.hash.replace(/^#\/?/, "").split("/")[2] ?? "";

export function Docs({ name }: { name?: string }) {
  const slug = name && MOVED[name] ? MOVED[name] : name;
  const i = Math.max(0, PAGES.findIndex((p) => p.slug === slug));
  const page = PAGES[i];
  const { html, toc } = useMemo(() => (page ? render(page) : { html: "", toc: [] }), [page]);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    setOpen(false);
    const go = () => {
      const id = anchorOf();
      // after the shell's own scroll-to-top on a hash change
      requestAnimationFrame(() => { if (id) document.getElementById(id)?.scrollIntoView({ block: "start" }); else window.scrollTo(0, 0); });
    };
    go();
    addEventListener("hashchange", go);
    return () => removeEventListener("hashchange", go);
  }, [page?.slug]);

  if (!page) return <p className="muted">No documentation in this build.</p>;
  const prev = PAGES[i - 1];
  const next = PAGES[i + 1];
  return (
    <div className="docs">
      <nav className="docs__nav" aria-label="Documentation">
        <button type="button" className="docs__navtoggle" aria-expanded={open} aria-controls="docs-nav-list" onClick={() => setOpen((o) => !o)}>
          <span className="eyebrow">Docs</span><span className="docs__navcurrent">{page.title}</span><span aria-hidden="true" className="docs__chevron">▾</span>
        </button>
        <div id="docs-nav-list" className={`docs__navlist${open ? " docs__navlist--open" : ""}`}>
          {GROUPS.map((g) => (
            <div key={g} className="docs__group">
              <p className="docs__grouplabel">{g}</p>
              <ul>{PAGES.filter((p) => p.group === g).map((p) => <li key={p.slug}><a href={href("docs", p.slug)} aria-current={p.slug === page.slug ? "page" : undefined}>{p.title}</a></li>)}</ul>
            </div>
          ))}
        </div>
      </nav>
      <article className="docs__page">
        <header className="docs__head">
          <p className="eyebrow">{page.group}</p>
          <h1>{page.title}</h1>
          {page.summary ? <p className="docs__summary">{page.summary}</p> : null}
          {toc.length >= 3 ? (
            <nav className="docs__toc" aria-label="On this page">
              <p className="docs__grouplabel">On this page</p>
              <ul>{toc.map((t) => <li key={t.id}><a href={href("docs", page.slug, t.id)}>{t.text}</a></li>)}</ul>
            </nav>
          ) : null}
        </header>
        {page.slug === HOW_IT_WORKS ? <HowFlow /> : null}
        <div className="prose docs__body" dangerouslySetInnerHTML={{ __html: html }} />
        <nav className="docs__pager" aria-label="Previous and next">
          {prev ? <a className="docs__pagerlink" href={href("docs", prev.slug)}><span className="docs__grouplabel">Previous</span><span>{prev.title}</span></a> : <span />}
          {next ? <a className="docs__pagerlink docs__pagerlink--next" href={href("docs", next.slug)}><span className="docs__grouplabel">Next</span><span>{next.title}</span></a> : <span />}
        </nav>
      </article>
    </div>
  );
}
