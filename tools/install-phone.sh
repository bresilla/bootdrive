#!/usr/bin/env bash
# Install/update the BootDrive GUI Flatpak on the phone from the latest CI build.
#
#   ./tools/install-phone.sh [ssh-host]
#
# Waits for the newest `flatpak` workflow run to finish, downloads its aarch64
# bundle, copies it to the phone, and installs it as your user (no sudo). Run
# this after pushing a change to refresh the app on the device.
set -euo pipefail

HOST="${1:-${BOOTDRIVE_PHONE:-100.68.168.31}}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
ARTIFACT="bootdrive-aarch64-aarch64.flatpak"
BUNDLE="bootdrive-aarch64.flatpak"
TMP="$(mktemp -d)"
SSH_OPTS="-o StrictHostKeyChecking=accept-new"
trap 'rm -rf "$TMP"' EXIT

echo "==> latest flatpak workflow run…"
RUN_ID="$(gh run list --workflow=flatpak.yml --limit 1 --json databaseId -q '.[0].databaseId')"
STATUS="$(gh run list --workflow=flatpak.yml --limit 1 --json status -q '.[0].status')"
if [ "$STATUS" != "completed" ]; then
	echo "    run $RUN_ID is $STATUS — waiting for it to finish…"
	gh run watch "$RUN_ID" --exit-status >/dev/null || {
		echo "!! the flatpak build failed; check: gh run view $RUN_ID --log-failed" >&2
		exit 1
	}
fi

echo "==> downloading aarch64 bundle from run $RUN_ID…"
gh run download "$RUN_ID" -n "$ARTIFACT" -D "$TMP"

echo "==> copying to $HOST and installing (user, no sudo)…"
scp $SSH_OPTS "$TMP/$BUNDLE" "$HOST:~/"
ssh $SSH_OPTS "$HOST" '
	flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
	flatpak uninstall --user -y net.bresilla.BootDrive 2>/dev/null || true
	flatpak install --user --noninteractive ~/'"$BUNDLE"'
	echo "installed:"; flatpak --user list --app | grep -i bootdrive || true
'

echo "==> done. Launch BootDrive from the phosh app grid (or: flatpak run net.bresilla.BootDrive)."
