# The KRIA privilege broker

A small service that performs a **fixed, closed set** of privileged operations on
KRIA's behalf. It exists so that KRIA itself never runs as root and never invokes
`sudo`.

## Install

One command, and the only step in the whole OS-control feature that needs root:

```bash
sudo bash deploy/broker/install.sh
```

Undo it with `sudo bash deploy/broker/install.sh --uninstall`.

Without the broker, KRIA works normally — the four privileged actions below simply
report "not available" instead of silently doing nothing.

## What it can do

Five operations, and no way to add a sixth at runtime. Each is a typed message, not
a command string, so nothing KRIA sends can become a new program, a new argument,
or a shell word.

| Operation | Privilege it needs | Status |
|---|---|---|
| Change a file's owner | `CAP_CHOWN`, `CAP_FOWNER` | implemented |
| Set battery charge limits | write `/sys/class/power_supply` | implemented |
| Turn the firewall on or off | `CAP_NET_ADMIN`, `CAP_NET_RAW` | implemented |
| Configure a printer | write `/etc/cups`, talk to `cupsd` | implemented |
| Install or remove packages | effectively unconfined | implemented, **opt-in** |
| Change a privacy toggle | none | **permanently refused** — see below |

### Why a privacy toggle is refused

Camera, microphone and location permissions are **per-user** settings. Root writing
them would change *root's* settings and report success while the user's actual
permission stayed exactly as it was. A false confirmation about a privacy control
is the worst possible outcome, so the broker refuses, and KRIA changes those
settings directly as the user instead — which needs no privilege at all.

### Why package installation is opt-in

Installing a package runs that package's own maintainer scripts **as root**. Those
scripts are arbitrary code and cannot be meaningfully confined: a package that
needs to load a kernel module or add a user must be allowed to.

So the base service stays hardened and cannot install packages. If you want that,
read the warning in `10-packages.conf` and install it deliberately:

```bash
sudo mkdir -p /etc/systemd/system/kria-os-broker.service.d
sudo cp deploy/broker/10-packages.conf /etc/systemd/system/kria-os-broker.service.d/
sudo systemctl daemon-reload && sudo systemctl restart kria-os-broker
```

Most people should skip it and let their desktop's Software application install
packages. KRIA can still search, inspect and plan package changes without it.

## How a request is authorized

1. KRIA sends a typed request over a Unix socket at `/run/kria/broker.sock`.
2. The kernel tells the broker the caller's uid and pid (`SO_PEERCRED`) — the
   caller cannot forge them, so the broker never trusts what the request claims
   about who sent it.
3. The broker asks **Polkit**, which shows *your desktop's own* password dialog.
   The broker never sees your password.
4. Only then does it perform the one operation, and it reports back one of
   `Applied`, `PartiallyApplied` (naming which steps landed), or `Uncertain`.

A denied Polkit request stays denied. There is no retry with wider privileges and
no fallback path.

## How it is confined

See the comments in `kria-os-broker.service` — each relaxation is justified next to
the setting it relaxes. Two are worth knowing about:

- **`PrivateNetwork=no`.** With a private network namespace, `ufw` would configure
  the firewall of *that* namespace: the rules would protect nothing while the call
  reported success. Outbound traffic is still impossible because `AF_INET` is not
  in the allowed address families.
- **`ProtectHome=no`.** Changing a file's owner is the point of one operation, and
  those files are usually in your home directory.

## Verifying an install

```bash
systemctl status kria-os-broker      # should be active
journalctl -u kria-os-broker -n 50   # startup and per-request decisions
ls -l /run/kria/broker.sock          # the socket KRIA connects to
```
