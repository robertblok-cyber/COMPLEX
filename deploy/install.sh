#!/bin/sh
# RobertBlok.com — Fullstack development
# One-time install on the VPS. Run from the unpacked folder: sudo sh deploy/install.sh
set -e
id -u complex >/dev/null 2>&1 || useradd --system --home /srv/complex --shell /usr/sbin/nologin complex
mkdir -p /srv/complex
cp -r complex site /srv/complex/
chown -R complex:complex /srv/complex
chmod 755 /srv/complex/complex
cp deploy/complex.service /etc/systemd/system/complex.service
systemctl daemon-reload
systemctl enable --now complex
systemctl --no-pager status complex | head -5

if ! command -v caddy >/dev/null; then
  apt-get install -y debian-keyring debian-archive-keyring apt-transport-https curl
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | tee /etc/apt/sources.list.d/caddy-stable.list
  apt-get update && apt-get install -y caddy
fi
cp deploy/Caddyfile /etc/caddy/Caddyfile
systemctl reload caddy || systemctl restart caddy
echo "done: https://complex.robertblok.com"
