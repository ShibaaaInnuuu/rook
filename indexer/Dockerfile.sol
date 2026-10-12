# Rook's Solana indexer + its API as ONE process (docs/v9/indexer-solana.md). Build context: the REPOSITORY ROOT, e.g.
#   docker build -f indexer/Dockerfile.sol -t rook-indexer-sol .
# It reads Rook's DBC configs from the site's own config file (web-rook/src/config/solana.ts), copied in below, so the site and the
# indexer are built from the same addresses; ROOK_CONFIGS overrides it (devnet, tests).
FROM node:22-alpine
RUN apk add --no-cache su-exec
WORKDIR /app/indexer
COPY indexer/package.json indexer/package-lock.json ./
RUN npm ci --omit=dev
COPY indexer/tsconfig.json indexer/types.ts ./
COPY indexer/src ./src
COPY web-rook/src/config/solana.ts /app/web-rook/src/config/solana.ts
ENV NODE_ENV=production SOL_DB_PATH=/data/rook-sol.db DB_PATH=/data/rook-sol.db PORT=8788 SITE_CONFIG_PATH=/app/web-rook/src/config/solana.ts
# /data is a mounted volume at run time (the database); no VOLUME line, which Railway refuses
EXPOSE 8788
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s CMD wget -qO- "http://127.0.0.1:${PORT}/v1/health" >/dev/null || exit 1
COPY indexer/entrypoint.sh /usr/local/bin/entrypoint.sh
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["node", "--experimental-sqlite", "--no-warnings", "--import", "tsx", "src/sol/main.ts"]
