#!/bin/sh
# Buildroot post-build script: final touches to the target tree.
set -eu
TARGET=$1
cd "$TARGET"

mkdir -p data mnt

# NetworkManager owns DNS (rc-manager=symlink).
ln -sfn ../run/NetworkManager/resolv.conf etc/resolv.conf

# Not wanted at boot (BusyBox httpd would also take port 80).
rm -f etc/init.d/S50crond etc/init.d/S50telnet etc/init.d/S90httpd \
	etc/init.d/S41ifplugd etc/init.d/S10mdev

# Development conveniences from the gitignored secrets/ directory, if
# present: Wi-Fi networks (*.nmconnection) and SSH keys for root. Left
# out with X6100_DIST=1 (images for others). The target tree outlives a
# build, so what an earlier build installed is removed first.
rm -f etc/NetworkManager/system-connections/*.nmconnection root/.ssh/authorized_keys
SECRETS=$BR2_EXTERNAL_X6100_PATH/secrets
if [ "${X6100_DIST:-0}" != 1 ]; then
	for f in "$SECRETS"/*.nmconnection; do
		[ -f "$f" ] || continue
		install -D -m 0600 "$f" "etc/NetworkManager/system-connections/${f##*/}"
	done
	if [ -f "$SECRETS/authorized_keys" ]; then
		install -d -m 0700 root/.ssh
		install -m 0600 "$SECRETS/authorized_keys" root/.ssh/authorized_keys
	fi
fi

# Licences of everything in the image, from "make legal-info" when it has
# been run (scripts/docker-build.sh --dist always does): the manifest and
# licence texts, readable on the radio and from the web page.
rm -rf usr/share/legal-info
LEGAL=$BASE_DIR/legal-info
if [ -f "$LEGAL/manifest.csv" ]; then
	mkdir -p usr/share/legal-info
	cp "$LEGAL/manifest.csv" "$LEGAL/README" usr/share/legal-info/ 2>/dev/null || true
	cp -R "$LEGAL/licenses" usr/share/legal-info/licenses
fi

# Versions, for bug reports.
{
	v=${X6100_VERSION:-$(git -C "$BR2_EXTERNAL_X6100_PATH" describe --always --dirty 2>/dev/null || echo unknown)}
	echo "X6100_Q3=$v"
	[ "${X6100_DIST:-0}" = 1 ] && echo "DIST=1"
	echo "BUILD_DATE=$(date -u +%Y-%m-%dT%H:%MZ)"
} > etc/x6100-release
