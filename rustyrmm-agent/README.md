# RustyRMM Server Daemon
[![License](https://img.shields.io/badge/License-BSD_2--Clause-orange.svg)](https://opensource.org/licenses/BSD-2-Clause)

RustyRMM - the Open Source Remote Management and Monitoring Agent

## Usage
```
Usage: rustyrmm-agent [OPTIONS]

OPTIONS:
-s, --server                 The IP address of the RustyRMM server daemon
-p, --port                   The TCP port the RustyRMM server is listening on [default: 50052]
```

## Update Installs (Linux)

On request the Agent installs the pending Updates (all, or security only) with apt-get or dnf.
It runs the package manager directly in its own transient systemd unit,
`systemd-run --unit=rustyrmm-update-<id> --collect`, so a restarting Agent never stops it and
the Agent never kills it. The output goes to `<state dir>/update-<id>.log`.

`<state dir>/update-install.json` holds the install's id, unit name and pending packages.
After a restart the Agent reattaches to the unit, and resends the outcome until the Server
acknowledges it, then deletes both files. A unit that ended while the Agent was down, with no
outcome recorded, is reported Interrupted.

`rustyrmm-agent install-service` (or `pair --install-service`) also writes `/etc/needrestart/conf.d/rustyrmm.conf` so
needrestart does not restart the Agent after an install; `uninstall-service` removes it.
