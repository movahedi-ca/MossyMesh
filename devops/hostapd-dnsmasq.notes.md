# Optional: Wi-Fi AP + DNS hijack for captive portal

On a Pi genesis node you can advertise a local SSID and redirect all DNS to
the portal so phones open MessyMash automatically.

> These notes are operational guidance — review interface names and firewall
> policy before enabling on production hardware.

## Packages (Raspberry Pi OS)

```bash
sudo apt install -y hostapd dnsmasq iptables
sudo systemctl stop hostapd dnsmasq || true
sudo systemctl unmask hostapd
```

## Example `hostapd` (`/etc/hostapd/hostapd.conf`)

```
interface=wlan0
driver=nl80211
ssid=MessyMash-Mesh
hw_mode=g
channel=6
wmm_enabled=0
auth_algs=1
ignore_broadcast_ssid=0
wpa=2
wpa_passphrase=changeme-mesh
wpa_key_mgmt=WPA-PSK
rsn_pairwise=CCMP
```

## Example `dnsmasq` (`/etc/dnsmasq.d/messymash.conf`)

```
interface=wlan0
bind-interfaces
dhcp-range=10.42.0.10,10.42.0.200,12h
# Hijack every name to the portal node
address=/#/10.42.0.1
```

Assign static IP on `wlan0` (e.g. `10.42.0.1/24`), enable IPv4 forward only if
you intentionally bridge to another interface (usually **off** for pure offline
islands).

## Portal

```bash
export PORTAL_PORT=80
./devops/deploy-pi.sh
```

nginx redirects OS captive probes (`/generate_204`, `/hotspot-detect.html`,
etc.) to `/` — combined with DNS hijack this reliably raises the captive sheet.

## Hardening tips

- Keep WPA2+ passphrase rotated per event / island
- Do not expose port 80 on an upstream WAN interface
- Prefer `network_mode: host` only when you understand port conflicts with
  hostapd / system nginx
