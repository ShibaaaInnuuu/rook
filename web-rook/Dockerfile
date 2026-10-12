# The site (Solana since 2026-10-10; it was the Phase-1 Robinhood site, docs/v6/phase1-scope.md §8.3). The build context is THIS directory alone (web-rook/, not web/):
#   railway up ./web-rook --path-as-root --service <the site's service> --ci        (or: docker build web-rook)
# In that context scripts/sync-content.mjs finds no ../docs or ../deployments and copies nothing, so the build ships what is
# COMMITTED: run `npm run sync` in the repository and commit src/content/ and src/deployments/{4663,stack-4663}.json first
# (test/copies.test.ts fails while src/content differs from docs/; a Phase-1 build without the stack file refuses every send).
# Two stages: build the static site, then serve it with a dependency-free Node server. No secret is needed to build or to run.
FROM node:22-alpine AS build
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY . .
# Vite reads these when it builds, so they must be declared here to be visible inside the build. Neither is a secret:
# they are the public addresses of the staging chain and its API. Unset, the site simply has no staging view.
ARG VITE_ETCH_API=
ARG VITE_ETCH_STAGING_RPC=
ARG VITE_ETCH_STAGING_API=
# docs/v6/phase1-scope.md §8.3: "1" builds the Phase-1 site (no Bots, Rent, YOYO, desk or Pons path; sends only with the bundled
# src/deployments/stack-4663.json; the Trust page opens with the Phase-1 risk statement). Any other non-empty value fails the build.
ARG VITE_ROOK_PHASE=
# docs/v7/dormant-collections.md §4.2 (independent review DI-L-1): the runner's GET /collections (absolute, or a same-origin path), whose
# lives the Launch page shows ("warming up (~N min)", "queued (#2)") and whose GET /capacity sits beside it. Not a secret. Unset, the page
# still fails closed on the module's on-chain freshness and offers the wake from the chain alone.
ARG VITE_COLLECTIONS_URL=
# The Solana site (2026-10-10): the RPC endpoint the browser reads Solana through, in place of src/config/solana.ts's rpcUrl (the public
# mainnet-beta endpoint, rate-limited). Production: a Helius key RESTRICTED TO THE SITE'S DOMAINS in Helius's dashboard (userook.fun,
# www.userook.fun). It ends up in the public JavaScript, so it is not a secret: the domain restriction is what protects it.
ARG VITE_SOLANA_RPC=
# Rook's Solana indexer (docs/v9/indexer-solana.md): its public base URL, e.g. https://rook-indexer-sol.up.railway.app. Not a secret.
# Unset, the front page and the board read the curves straight from the RPC, and the chart, trades and holders say they need it.
ARG VITE_ROOK_INDEXER=
ENV VITE_ETCH_API=$VITE_ETCH_API VITE_ETCH_STAGING_RPC=$VITE_ETCH_STAGING_RPC VITE_ETCH_STAGING_API=$VITE_ETCH_STAGING_API VITE_ROOK_PHASE=$VITE_ROOK_PHASE \
    VITE_COLLECTIONS_URL=$VITE_COLLECTIONS_URL VITE_SOLANA_RPC=$VITE_SOLANA_RPC VITE_ROOK_INDEXER=$VITE_ROOK_INDEXER
RUN npm run build

FROM node:22-alpine
WORKDIR /app
ENV NODE_ENV=production
COPY --from=build /app/dist ./dist
COPY server.mjs ./
# Runtime only (set on the Railway service, never at build): CLOUDINARY_CLOUD_NAME, CLOUDINARY_API_KEY, CLOUDINARY_API_SECRET enable
# POST /api/upload (the Launch page's image + metadata hosting). Unset, the route answers 503 and the page says upload isn't set up.
USER node
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s CMD wget -qO- http://127.0.0.1:${PORT:-8080}/healthz || exit 1
CMD ["node", "server.mjs"]
