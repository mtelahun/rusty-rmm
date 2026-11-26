# Deploying the Server

Run the Server on one public VM (Debian/Ubuntu, systemd) with a DNS name. Postgres 17 runs on the same VM. The Operator port listens on 443 with a Let's Encrypt certificate. The Agent port stays at its default, 7443. All commands run as root. Config files live in [`deploy/`](../deploy).

Set the name once:

```sh
export RMM_HOST=rmm.example.com   # DNS A/AAAA record must already point at the VM
```

## 1. Firewall

Allow inbound TCP 80 (certbot), 443 (Operator port) and 7443 (Agent port).

```sh
ufw allow 80/tcp && ufw allow 443/tcp && ufw allow 7443/tcp
```

Postgres listens on localhost only; do not open 5432.

## 2. Install the Server

Create the `rustyrmm` user, then install the binary from the release installer into `/opt/rustyrmm/bin` (the installer puts binaries in `$CARGO_HOME/bin`).

```sh
useradd --system --no-create-home --shell /usr/sbin/nologin rustyrmm
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/mtelahun/rusty-rmm/releases/download/v0.1.0/rustyrmm-server-installer.sh \
  | CARGO_HOME=/opt/rustyrmm INSTALLER_NO_MODIFY_PATH=1 sh
/opt/rustyrmm/bin/rustyrmm-server --version
```

## 3. Postgres 17

Install Postgres 17 from your distro or [apt.postgresql.org](https://wiki.postgresql.org/wiki/Apt), then create the role and database. The Server applies its own migrations on startup.

```sh
apt install -y postgresql-17
DB_PASSWORD=$(openssl rand -hex 24); echo "$DB_PASSWORD"
sudo -u postgres psql -c "CREATE ROLE rustyrmm LOGIN PASSWORD '$DB_PASSWORD'"
sudo -u postgres createdb -O rustyrmm rustyrmm
```

## 4. Environment file

```sh
install -d -o rustyrmm -g rustyrmm -m 0750 /etc/rustyrmm
install -o rustyrmm -g rustyrmm -m 0600 deploy/server.env.example /etc/rustyrmm/server.env
$EDITOR /etc/rustyrmm/server.env   # set DATABASE_URL (password from step 3) and RUSTYRMM_HOSTNAME
```

`RUSTYRMM_HOSTNAME` becomes `--agent-name`: the name Agents dial. Pairing one-liners carry `RUSTYRMM_HOSTNAME:7443`.

### Email Notifications (optional)

Alert emails are off unless the Server knows an SMTP server. Set these in `/etc/rustyrmm/server.env`; the unit's `ExecStart` needs no change, because `serve` reads each flag from its environment variable.

| Flag | Environment variable | Meaning |
| --- | --- | --- |
| `--smtp-host` | `RUSTYRMM_SMTP_HOST` | SMTP server (email is on when this and `--smtp-from` are set) |
| `--smtp-port` | `RUSTYRMM_SMTP_PORT` | Default 25 for `none`, 587 for `starttls`, 465 for `tls` |
| `--smtp-tls` | `RUSTYRMM_SMTP_TLS` | `none`, `starttls` (default) or `tls` |
| `--smtp-username` | `RUSTYRMM_SMTP_USERNAME` | Optional |
| (no flag) | `RUSTYRMM_SMTP_PASSWORD` | Optional; only the environment, so it stays out of `ps` |
| `--smtp-from` | `RUSTYRMM_SMTP_FROM` | Sender, e.g. `RustyRMM <rmm@example.com>` |
| `--public-url` | `RUSTYRMM_PUBLIC_URL` | Web UI base URL for links in emails, e.g. `https://rmm.example.com`; required when email is on |

The Server refuses to start if email is on and `--public-url` is missing or `--smtp-from` is not a valid mailbox. An Operator needs an email address to receive email. A failed send is retried after 1, 5 and 25 minutes, then dropped with a line in the journal.

## 5. Certificate (certbot)

certbot runs standalone on port 80. The deploy hook copies the certificate and key to `/etc/rustyrmm/` (owner `rustyrmm`, 0600) and restarts the Server. It runs on issue and on every renewal. The Server reads the certificate only at startup, so the restart is required; Agents reconnect with backoff.

```sh
apt install -y certbot
install -D -m 0755 deploy/certbot-deploy-hook.sh /etc/letsencrypt/renewal-hooks/deploy/rustyrmm.sh
certbot certonly --standalone -d "$RMM_HOST" --deploy-hook /etc/letsencrypt/renewal-hooks/deploy/rustyrmm.sh
```

The Server does not exist as a unit yet, so the first hook run fails at `systemctl restart`. The files are already copied; continue. Renewal uses the packaged `certbot.timer` and needs port 80 free.

## 6. Run the Server

```sh
install -m 0644 deploy/rustyrmm-server.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now rustyrmm-server
journalctl -u rustyrmm-server -n 20
```

The log shows the Agent CA fingerprint and both listeners. On first start the Server creates the CA in `/var/lib/rustyrmm`. If the unit fails, check that `/etc/rustyrmm/operator.crt` and `operator.key` exist.

## 7. First Operator

```sh
sudo -u rustyrmm sh -c 'set -a; . /etc/rustyrmm/server.env; exec /opt/rustyrmm/bin/rustyrmm-server operator add admin admin@example.com'
```

Open `https://$RMM_HOST/` and log in.

## 8. Backups

Nightly: `pg_dump` plus `/var/lib/rustyrmm`, sent by restic to an S3-compatible bucket, keeping 14 daily snapshots. The state directory holds the CA key. Losing it means re-pairing every Endpoint.

```sh
apt install -y restic
install -m 0600 deploy/backup.env.example /etc/rustyrmm/backup.env
$EDITOR /etc/rustyrmm/backup.env   # bucket, restic password, S3 keys
set -a; . /etc/rustyrmm/backup.env; set +a
restic init
install -m 0755 deploy/rustyrmm-backup /usr/local/sbin/rustyrmm-backup
install -m 0644 deploy/rustyrmm-backup.service deploy/rustyrmm-backup.timer /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now rustyrmm-backup.timer
systemctl start rustyrmm-backup.service && restic snapshots
```

Store the restic password somewhere off the VM; without it the backups cannot be read.

## 9. Restore

On a fresh VM, repeat steps 1-5 (installer, Postgres, firewall, certbot; skip `createdb`), keep the DNS name the same, and do not start the Server. Then:

```sh
set -a; . /etc/rustyrmm/backup.env; set +a   # recreate backup.env first
restic restore latest --host rustyrmm --target /tmp/restore
systemctl stop rustyrmm-server 2>/dev/null || true
rm -rf /var/lib/rustyrmm && cp -a /tmp/restore/var/lib/rustyrmm /var/lib/rustyrmm
chown -R rustyrmm:rustyrmm /var/lib/rustyrmm
sudo -u postgres createdb -O rustyrmm rustyrmm       # skip if it exists
sudo -u postgres pg_restore --clean --if-exists --no-owner --role=rustyrmm \
  -d rustyrmm /tmp/restore/var/backups/rustyrmm/rustyrmm.dump
systemctl start rustyrmm-server
rm -rf /tmp/restore
```

Agents trust the same CA key and reconnect on their own. If the hostname changed, they cannot find the Server and must be re-paired.

## 10. First Endpoint

On the Server host (or anywhere with the `rustyrmm` CLI), create an Environment and a Pairing Token:

```sh
rustyrmm login https://$RMM_HOST
rustyrmm env create "Home"   # optionally: --owner "<customer or site>"
rustyrmm pairing-token create Home
```

The command prints one-liners for Linux/macOS and Windows. Run the right one on the Endpoint; it installs the Agent, pairs it and installs its service. The web UI shows the Endpoint in its Environment within a minute. The token is shown only once.
