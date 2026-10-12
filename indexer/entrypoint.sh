#!/bin/sh
# A mounted volume arrives owned by root, so the data directory is handed to the unprivileged user
# here — the only thing this script does as root — and the service is then run as that user.
set -e
DIR=$(dirname "${DB_PATH:-/data/etch.db}")
mkdir -p "$DIR"
chown -R node:node "$DIR"
exec su-exec node "$@"
