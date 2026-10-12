# K3: the production keeper sender (docs/v6/p1-services.md §1, docs/deploy/services-v1.md).
# Build context: the REPO ROOT, because a draw's snapshot is built by tools/snapshot's own core, and the stack file lives in
# deployments/:   docker build -f keeper/Dockerfile .      (keeper/Dockerfile.dockerignore keeps the context to what it needs)
# No secret is needed to build, and none is baked in. At run time the operator sets, in Railway's secret store:
#   KEEPER_CHAIN_ID=4663  KEEPER_STACK=/app/deployments/stack-4663.json  KEEPER_PUBLIC_URL=https://<this service's domain>
#   KEEPER_PRIVATE_KEY (secret) and KEEPER_MODE=send only after a dry run has been read; UNISAT_API_KEY (secret) for the feed.
#   KEEPER_SNAPSHOT_S3_ENDPOINT / _BUCKET / _PREFIX / _ACCESS_KEY_ID / _SECRET_ACCESS_KEY (secret) for the Cloudflare R2 bucket, or
#   KEEPER_SNAPSHOT_UPLOAD_URL (+ KEEPER_SNAPSHOT_UPLOAD_AUTH, secret) for a plain-PUT mirror, and KEEPER_SNAPSHOT_BASE_URL (the public
#   URL): where a draw's snapshot is mirrored off this volume (required before a draw is announced on 4663; docs/deploy/services-v1.md
#   §2.2); KEEPER_STATUS_TOKEN (secret) for /status/full.
# Without KEEPER_MODE=send it is a dry run: every call simulated, nothing signed. A volume at /data keeps the one transaction in
# flight, the day's gas, the coin index, the log cache and the published snapshots (also mirrored; the volume should outlive a
# redeploy). The same image runs the standalone resolver with the start command
# `drop-to-node node --import tsx service/resolver-main.ts` (drop-to-node first: a start command replaces the image's command)
# (its own RESOLVER_PRIVATE_KEY, RESOLVER_MODE=send; docs/audit/2026-09-26/fixes-W2-services.md §2).
# Audit E-16 / R2-S7: production dependencies only (tsx, which runs the TypeScript, is one), and the service runs as `node`, not root:
# the entrypoint (ops/docker/drop-to-node.sh, as the monitor's) makes the /data volume writable by `node` and drops to it before
# node starts. The key is only ever in the environment of a process that is not root.
FROM node:22-alpine
RUN apk add --no-cache su-exec
WORKDIR /app
COPY keeper/package.json keeper/package-lock.json keeper/
RUN cd keeper && npm ci --omit=dev --no-audit --no-fund
COPY tools/snapshot/package.json tools/snapshot/package-lock.json tools/snapshot/
RUN cd tools/snapshot && npm ci --omit=dev --no-audit --no-fund
COPY keeper/tsconfig.json keeper/
COPY keeper/src keeper/src
COPY keeper/service keeper/service
COPY tools/snapshot/src tools/snapshot/src
COPY tools/snapshot/exclusions.json tools/snapshot/
COPY deployments deployments
COPY --chmod=0755 ops/docker/drop-to-node.sh /usr/local/bin/drop-to-node
ENV NODE_ENV=production KEEPER_STATE_DIR=/data PORT=8080 DATA_DIRS=/data
RUN mkdir -p /data && chown node:node /data
WORKDIR /app/keeper
EXPOSE 8080
HEALTHCHECK --interval=60s --timeout=5s --start-period=120s CMD wget -qO- "http://127.0.0.1:${PORT:-8080}/healthz" >/dev/null || exit 1
ENTRYPOINT ["drop-to-node"]
CMD ["node", "--import", "tsx", "service/main.ts"]
