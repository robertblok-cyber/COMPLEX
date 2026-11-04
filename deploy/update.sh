#!/bin/sh
# RobertBlok.com — Fullstack development
# Redeploy after a change: sudo sh deploy/update.sh
set -e
/srv/complex/complex check site            # refuse to deploy a site that does not compile
cp complex /srv/complex/complex.new && mv /srv/complex/complex.new /srv/complex/complex
rm -rf /srv/complex/site && cp -r site /srv/complex/site
chown -R complex:complex /srv/complex
systemctl restart complex
echo "deployed"
