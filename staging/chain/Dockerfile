# docs/phase6.md: the staging fork as ONE service — the node (127.0.0.1 only), the bootstrap, the public proxy and the operator.
# Build context: the REPOSITORY ROOT (it deploys our contracts with the same guarded wrapper as everywhere else), e.g.
#   docker build -f staging/chain/Dockerfile -t etched-staging-chain .
# NOT BUILT on the machine that wrote it (Docker was down): see docs/phase6.md, unverified.
FROM ghcr.io/foundry-rs/foundry:stable AS foundry

FROM node:22-bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends bash curl python3 ca-certificates git && rm -rf /var/lib/apt/lists/*
COPY --from=foundry /usr/local/bin/anvil /usr/local/bin/forge /usr/local/bin/cast /usr/local/bin/
WORKDIR /app
# the contracts and the wrapper, compiled at build time so a first boot deploys in seconds
COPY foundry.toml remappings.txt ./
COPY lib ./lib
COPY src ./src
COPY script ./script
RUN forge build --skip test
# the three packages the chain runs: the snapshot tool (the operator builds roots with it), the keeper (the operator), staging
COPY tools/snapshot/package.json tools/snapshot/package-lock.json ./tools/snapshot/
RUN cd tools/snapshot && npm ci
COPY keeper/package.json keeper/package-lock.json ./keeper/
RUN cd keeper && npm ci
COPY staging/package.json staging/package-lock.json ./staging/
RUN cd staging && npm ci
COPY tools/snapshot ./tools/snapshot
COPY keeper ./keeper
COPY staging ./staging
# docs/phase11.md: the committed catalogue (Phase 9) is what the inventory is listed from. Read only; the site owns the file.
COPY web/public/catalogue/catalogue.json ./web/public/catalogue/catalogue.json
RUN mkdir -p /app/deployments /data
ENV STAGING_DATA=/data STAGING_REPO=/app PORT=8080 HOME=/root
# /data is a mounted volume at run time (the node's state); no VOLUME line, which Railway refuses
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=180s CMD curl -fsS "http://127.0.0.1:${PORT}/healthz" >/dev/null || exit 1
WORKDIR /app/staging
CMD ["npx", "tsx", "chain/start.ts"]
