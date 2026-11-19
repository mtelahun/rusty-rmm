#!/usr/bin/env bash
# certbot deploy hook: copy the renewed cert and key for the Server and restart it.
# The Server reads the Operator cert only at startup; Agents reconnect with backoff.
set -euo pipefail

install -o rustyrmm -g rustyrmm -m 0600 "$RENEWED_LINEAGE/fullchain.pem" /etc/rustyrmm/operator.crt
install -o rustyrmm -g rustyrmm -m 0600 "$RENEWED_LINEAGE/privkey.pem" /etc/rustyrmm/operator.key
systemctl restart rustyrmm-server
