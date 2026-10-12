#!/bin/sh
# Audit E-16: the service runs as the image's `node` user (uid 1000), never as root. Railway mounts a service's volume owned by
# root, so this entrypoint alone runs as root, and only to make the volume writable by `node`; then it drops to `node` with
# su-exec (exec: the service replaces this shell, and no root process remains). Without a volume, or when already started as
# `node`, it only execs the command.
set -eu
if [ "$(id -u)" = "0" ]; then
  for d in ${DATA_DIRS:-/data}; do mkdir -p "$d" && chown -R node:node "$d"; done
  exec su-exec node "$@"
fi
exec "$@"
