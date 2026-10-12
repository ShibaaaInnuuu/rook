# The indexer + its API as ONE process. Build context: the REPOSITORY ROOT (it needs deployments/), e.g.
#   docker build -f indexer/Dockerfile -t etch-indexer .
FROM node:22-alpine
RUN apk add --no-cache su-exec
WORKDIR /app/indexer
COPY indexer/package.json indexer/package-lock.json ./
RUN npm ci --omit=dev
COPY indexer/tsconfig.json indexer/types.ts ./
COPY indexer/src ./src
# a public chain's deployment file, once one exists; without it the service idles and says so
COPY deployments /app/deployments
ENV NODE_ENV=production DEPLOYMENTS_DIR=/app/deployments DB_PATH=/data/etch.db PORT=8787
# /data is a mounted volume at run time (the database); no VOLUME line, which Railway refuses
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s CMD wget -qO- "http://127.0.0.1:${PORT}/v1/health" >/dev/null || exit 1
COPY indexer/entrypoint.sh /usr/local/bin/entrypoint.sh
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["node", "--experimental-sqlite", "--no-warnings", "--import", "tsx", "src/main.ts"]
