# docs/phase2.md §4. The build context is this directory alone: `railway up ./web --path-as-root`.
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
ENV VITE_ETCH_API=$VITE_ETCH_API VITE_ETCH_STAGING_RPC=$VITE_ETCH_STAGING_RPC VITE_ETCH_STAGING_API=$VITE_ETCH_STAGING_API
RUN npm run build

FROM node:22-alpine
WORKDIR /app
ENV NODE_ENV=production
COPY --from=build /app/dist ./dist
COPY server.mjs ./
USER node
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s CMD wget -qO- http://127.0.0.1:${PORT:-8080}/healthz || exit 1
CMD ["node", "server.mjs"]
