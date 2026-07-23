#!/usr/bin/env bash
# Build (if needed) and install the patched usb-signaller on the phone.
#
#   ./tools/deploy-usb-signaller.sh [ssh-host]
#
# Cross-compiles the static aarch64 usb-signaller (with mass_storage_mode),
# copies it to the phone, backs up the stock binary, installs the patched one,
# and restarts the service. Replacing a system binary needs sudo — the script
# uses an SSH TTY so the password prompt works. Nothing here touches USB; the
# service just gains a new supported mode.
set -euo pipefail

HOST="${1:-${BOOTDRIVE_PHONE:-100.68.168.31}}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SIGNALLER_DIR="${SIGNALLER_DIR:-/home/bresilla/data/code/github/usb-signaller}"
BIN="$SIGNALLER_DIR/target/aarch64-unknown-linux-musl/release/usb-signaller"
SSH_OPTS="-o StrictHostKeyChecking=accept-new"

if [ "${SKIP_BUILD:-0}" != "1" ] || [ ! -f "$BIN" ]; then
	echo "==> cross-compiling patched usb-signaller…"
	SIGNALLER_DIR="$SIGNALLER_DIR" "$ROOT/tools/build-usb-signaller-aarch64.sh" >/dev/null
fi
[ -f "$BIN" ] || { echo "!! build produced no binary" >&2; exit 1; }

echo "==> copying to $HOST…"
scp $SSH_OPTS "$BIN" "$HOST:~/usb-signaller.patched"
scp $SSH_OPTS "$ROOT/data/zz-bootdrive-usb-moded.conf" "$HOST:~/zz-bootdrive-usb-moded.conf"

echo "==> installing (enter your sudo password when prompted)…"
# STAGED expands in the login shell to the real path; each sudo then uses the
# absolute path (under sudo, ~ would resolve to /root, not your home).
ssh $SSH_OPTS -t "$HOST" '
set -e
STAGED="$HOME/usb-signaller.patched"
CONF="$HOME/zz-bootdrive-usb-moded.conf"
# Write the usb-signaller config as the user (with $HOME expanded), then let
# sudo install it. storage_path points at BootDrive'"'"'s current-image symlink;
# BootDrive re-points that symlink at whatever image you expose.
cat > "$HOME/usb-signaller.toml" <<EOF
[main]
default_mode = "developer_mode"

[mass_storage]
storage_path = "$HOME/.var/app/net.bresilla.BootDrive/data/bootdrive/current.img"
EOF
sudo sh -c "
  [ -e /usr/bin/usb-signaller.orig ] || cp -a /usr/bin/usb-signaller /usr/bin/usb-signaller.orig
  install -m755 \"$STAGED\" /usr/bin/usb-signaller
  install -m644 \"$CONF\" /usr/share/dbus-1/system.d/zz-bootdrive-usb-moded.conf
  install -Dm644 \"$HOME/usb-signaller.toml\" /etc/usb-signaller/usb-signaller.toml
  systemctl restart usb-signaller
  systemctl reload dbus || systemctl restart dbus || true
"
sleep 1
echo "--- current mode (mass_storage_mode/cdrom_mode are unadvertised but work): ---"
busctl call com.meego.usb_moded /com/meego/usb_moded com.meego.usb_moded mode_request
'
echo "==> done. (stock binary backed up at /usr/bin/usb-signaller.orig on the phone)"
