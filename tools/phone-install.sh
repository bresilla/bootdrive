#!/bin/sh
# Runs ON THE PHONE as root (via sudo). Installs the BootDrive backend + CLI and
# all integration files, then enables the service. Idempotent.
#
# Arg 1: the unprivileged user to add to the `bootdrive` group (for the GUI/CLI
#        to talk to the backend without sudo).
set -eu

STAGE="$(cd "$(dirname "$0")" && pwd)"
TARGET_USER="${1:-}"

echo "== installing binaries =="
install -Dm755 "$STAGE/bootdrived" /usr/libexec/bootdrived
install -Dm755 "$STAGE/bootdrive"  /usr/bin/bootdrive

echo "== installing service + D-Bus + polkit files =="
install -Dm644 "$STAGE/bootdrived.service" \
	/usr/lib/systemd/system/bootdrived.service
install -Dm644 "$STAGE/net.bresilla.BootDrive1.service" \
	/usr/share/dbus-1/system-services/net.bresilla.BootDrive1.service
install -Dm644 "$STAGE/net.bresilla.BootDrive1.conf" \
	/usr/share/dbus-1/system.d/net.bresilla.BootDrive1.conf
install -Dm644 "$STAGE/net.bresilla.BootDrive1.policy" \
	/usr/share/polkit-1/actions/net.bresilla.BootDrive1.policy

echo "== bootdrive group =="
getent group bootdrive >/dev/null 2>&1 || addgroup -S bootdrive 2>/dev/null || true
if [ -n "$TARGET_USER" ]; then
	addgroup "$TARGET_USER" bootdrive 2>/dev/null || true
	echo "   added $TARGET_USER to 'bootdrive' (re-login for the GUI to pick it up)"
fi

echo "== reloading systemd + D-Bus, starting bootdrived =="
systemctl daemon-reload
systemctl reload dbus 2>/dev/null || true
systemctl enable bootdrived.service >/dev/null 2>&1 || true
systemctl restart bootdrived.service

sleep 1
echo
echo "== status =="
systemctl --no-pager --lines=0 status bootdrived.service || true
echo
echo "Installed. Starting the service does NOT touch USB — it only acts on expose/eject."
echo
echo "Test it now (as root, no group needed):"
echo "    sudo bootdrive status"
echo "    sudo bootdrive expose /path/to/image.iso     # or --disk for a raw image"
echo "    sudo bootdrive eject"
echo
echo "As your user (after re-login, once in the 'bootdrive' group):"
echo "    bootdrive status"
