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

# Select the flatpak run for the CURRENT commit (not just the newest run —
# right after a push the new run may not be registered yet, which used to grab
# a stale build).
SHA="$(git rev-parse HEAD)"
echo "==> flatpak run for commit ${SHA:0:7}…"
RUN_ID=""
for _ in $(seq 1 30); do
	RUN_ID="$(gh run list --workflow=flatpak.yml --limit 20 \
		--json databaseId,headSha \
		-q "[.[] | select(.headSha==\"$SHA\")][0].databaseId" 2>/dev/null || true)"
	[ -n "$RUN_ID" ] && break
	echo "    run not registered yet, waiting…"; sleep 5
done
[ -n "$RUN_ID" ] || { echo "!! no flatpak run found for $SHA — did you push?" >&2; exit 1; }

STATUS="$(gh run view "$RUN_ID" --json status -q '.status')"
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
