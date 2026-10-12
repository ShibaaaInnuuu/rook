# K6: watcher v2 and the custody canary (docs/v6/k6-watcher-canary.md §7).
# Build context: the REPO ROOT, because watcher v2 imports watcher v1's pure functions from scripts/timelock-watch.mjs:
#   docker build -f monitor/Dockerfile .
# Monitor/Dockerfile.dockerignore keeps the context to the paths it needs.
# No secret is needed to build. At run time: ALERT_TELEGRAM_BOT_TOKEN and ALERT_TELEGRAM_CHAT_ID (both required: it refuses to
# start without them), MONITOR_CONFIG_JSON (or a config file via MONITOR_CONFIG), and MONITOR_GUARDIAN_KEY only if the config
# names a guardian; a volume at /data keeps the state (alert keys sent, txids judged, the 4663 cursor, draws verified) and the
# holder indexes (holders-<provider>.json, one per draws.providers entry: fixes-R3-services.md §1.1; a few MB, bounded).
# Audit E-16: the monitor runs as `node`, not root. Railway mounts volumes root-owned, so the entrypoint (ops/docker/
# drop-to-node.sh) makes /data writable by `node` and drops to it before node starts.
FROM node:22-alpine
RUN apk add --no-cache su-exec
WORKDIR /app
COPY monitor/package.json monitor/package-lock.json monitor/
RUN cd monitor && npm ci --omit=dev --no-audit --no-fund
COPY scripts/timelock-watch.mjs scripts/
COPY monitor/src monitor/src
COPY monitor/fixtures monitor/fixtures
# R2-L2: the draw exclusions are the published list itself (src/config.mjs reads it; its sha256 is printed and in /status)
COPY tools/snapshot/exclusions.json tools/snapshot/exclusions.json
COPY --chmod=0755 ops/docker/drop-to-node.sh /usr/local/bin/drop-to-node
ENV NODE_ENV=production DATA_DIRS=/data
RUN mkdir -p /data && chown node:node /data
EXPOSE 8080
HEALTHCHECK --interval=60s --timeout=5s CMD wget -qO- http://127.0.0.1:${PORT:-8080}/healthz || exit 1
ENTRYPOINT ["drop-to-node"]
CMD ["node", "monitor/src/main.mjs"]
