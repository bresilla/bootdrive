#!/usr/bin/env bash
# Deploy the BootDrive backend + CLI to the phone in one shot.
#
#   ./tools/deploy-phone.sh [ssh-host]
#
# Default host is 100.68.168.31; override with the arg or $BOOTDRIVE_PHONE.
# Set SKIP_BUILD=1 to reuse existing aarch64 binaries.
#
# It cross-compiles static aarch64 binaries, copies them plus the service/D-Bus/
# polkit files to the phone, and runs the installer under sudo over an SSH TTY
# (so the sudo password prompt works). Nothing here touches USB — the backend
# only acts when you run `bootdrive expose`/`eject`.
set -euo pipefail

HOST="${1:-${BOOTDRIVE_PHONE:-100.68.168.31}}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
BIN=target/aarch64-unknown-linux-musl/release
STAGE_REMOTE='~/bootdrive-deploy'
SSH_OPTS="-o StrictHostKeyChecking=accept-new"

echo "==> target phone: $HOST"

if [ "${SKIP_BUILD:-0}" != "1" ]; then
	echo "==> cross-compiling backend + CLI for aarch64…"
	./tools/build-helper-aarch64.sh >/dev/null
fi

for b in bootdrived bootdrive probe; do
	if [ ! -f "$BIN/$b" ]; then
		echo "!! missing $BIN/$b — build failed or SKIP_BUILD set without a prior build" >&2
		exit 1
	fi
done

echo "==> discovering remote user…"
DEPLOY_USER="$(ssh $SSH_OPTS "$HOST" 'whoami' | tr -d '\r')"
echo "    remote user: $DEPLOY_USER"

echo "==> copying files to $HOST:$STAGE_REMOTE …"
# shellcheck disable=SC2029
ssh $SSH_OPTS "$HOST" "mkdir -p $STAGE_REMOTE"
scp $SSH_OPTS \
	"$BIN/bootdrived" "$BIN/bootdrive" "$BIN/probe" \
	data/bootdrived.service \
	data/net.bresilla.BootDrive1.service \
	data/net.bresilla.BootDrive1.conf \
	data/net.bresilla.BootDrive1.policy \
	tools/phone-install.sh \
	"$HOST:$STAGE_REMOTE/"

echo "==> installing on the phone (enter your sudo password when prompted)…"
# -t allocates a TTY so sudo can prompt for the password.
ssh $SSH_OPTS -t "$HOST" "chmod +x $STAGE_REMOTE/phone-install.sh && sudo sh $STAGE_REMOTE/phone-install.sh '$DEPLOY_USER'"

echo
echo "==> done. The 'probe' binary is also at $STAGE_REMOTE/probe for the low-level boot test:"
echo "    ssh $HOST 'sudo $STAGE_REMOTE/probe /path/to/image.iso'   # Ctrl-C to eject"
