import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import { SECURITY_CONTACT, SITE_NAME, SITE_URL, securityTxt } from "./site.config";
import { ogPng } from "./scripts/og-image.mjs";
import { phaseFrom } from "./src/rook/phase";

// docs/v6/phase1-scope.md §8.3: VITE_ROOK_PHASE is "1" or unset; anything else stops the build here, not in a visitor's browser
phaseFrom(process.env.VITE_ROOK_PHASE);

/** The site's address into index.html (%SITE_URL%), and robots.txt + sitemap.xml written from the same constant. A hash-routed
 *  static site has one crawlable URL; the sitemap says so rather than listing fragments. The social card (og.png, 1200 × 630:
 *  the mark and the wordmark on the dark ground, scripts/og-image.mjs) is drawn at build so a shared link has a preview, and
 *  served at /og.png in dev. */
const site = (): Plugin => ({
  name: "rook-site",
  transformIndexHtml: { order: "pre", handler: (html) => html.replaceAll("%SITE_URL%", SITE_URL).replaceAll("%SITE_NAME%", SITE_NAME) },
  configureServer(server) {
    server.middlewares.use((req, res, next) => {
      if (req.url?.split("?")[0] !== "/og.png") return next();
      res.setHeader("content-type", "image/png"); res.end(ogPng());
    });
  },
  generateBundle() {
    this.emitFile({ type: "asset", fileName: "og.png", source: ogPng() });
    this.emitFile({ type: "asset", fileName: "robots.txt", source: `User-agent: *\nAllow: /\nSitemap: ${SITE_URL}/sitemap.xml\n` });
    // a security contact once one is published (docs/v6/phase1-scope.md K5, O9); none yet means no file, not a placeholder
    const sec = securityTxt(SECURITY_CONTACT, new Date());
    if (sec) this.emitFile({ type: "asset", fileName: ".well-known/security.txt", source: sec });
    else if (process.env.VITE_ROOK_PHASE === "1") this.warn("Phase-1 build with no SECURITY_CONTACT (site.config.ts): no /.well-known/security.txt is published (O9)");
    this.emitFile({ type: "asset", fileName: "sitemap.xml", source: `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n  <url><loc>${SITE_URL}/</loc></url>\n</urlset>\n` });
  },
});

// No analytics, no third-party scripts: everything the page loads is built from this directory. Nothing is inlined as a data: URL
// either: the server's CSP (font-src 'self') would refuse a font that Vite folded into the stylesheet.
export default defineConfig({ plugins: [react(), site()], build: { target: "es2022", sourcemap: false, chunkSizeWarningLimit: 900, assetsInlineLimit: 0 } });
