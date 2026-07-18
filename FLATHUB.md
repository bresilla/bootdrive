# Building & shipping the Flatpak (no phone required)

You never build the GUI on the phone. GitHub and Flathub build every
architecture on their own infrastructure — x86_64 **and aarch64** — so you only
need a browser.

## 1. CI builds both arches on every push

`.github/workflows/flatpak.yml` builds the Flatpak for `x86_64` and `aarch64`
using GitHub's native runners (`ubuntu-24.04` and `ubuntu-24.04-arm`) inside the
official `ghcr.io/flathub-infra/flatpak-github-actions:gnome-48` image.

Each run uploads an installable single-file bundle as an artifact:

- `bootdrive-x86_64.flatpak`
- `bootdrive-aarch64.flatpak`

Download the aarch64 one from the workflow run and, on the phone:

```sh
flatpak install --user ./bootdrive-aarch64.flatpak
flatpak run net.bresilla.BootDrive
```

That's the whole "get the GUI on the phone" story — no `flatpak-builder`, no
GTK dev packages, no cross-compiling.

## 2. Publishing on Flathub

Flathub's buildbot rebuilds the app for all arches when you submit it, so
end users just `flatpak install flathub net.bresilla.BootDrive`.

Steps ([docs.flathub.org](https://docs.flathub.org/docs/for-app-authors/submission)):

1. Fork <https://github.com/flathub/flathub> (do **not** copy only master).
2. Branch from `new-pr`:
   ```sh
   git clone --branch=new-pr git@github.com:<you>/flathub.git
   cd flathub && git checkout -b add-bootdrive new-pr
   ```
3. Add the manifest as `net.bresilla.BootDrive.yaml` (plus the committed
   `cargo-sources.json`). For Flathub, change the app source from the local
   `type: dir` to a tagged release, e.g.:
   ```yaml
   sources:
     - type: git
       url: https://github.com/bresilla/bootdrive.git
       tag: v0.1.0
     - cargo-sources.json
   ```
4. Open a PR **against the `new-pr` branch** titled `Add net.bresilla.BootDrive`.
5. Comment `bot, build` — Flathub builds x86_64 + aarch64 on its infra and gives
   you a test bundle. Iterate until green.
6. A reviewer merges it into `flathub/net.bresilla.BootDrive`; it's published
   within ~1–2 hours.

### App-ID / domain requirement (read this)

Flathub requires the reverse-DNS app-id to map to something you control:

- **`net.bresilla.BootDrive`** requires proving you own **bresilla.net** (a
  `.well-known/org.flathub.VerifiedApps.txt` file or DNS TXT record).
- If you don't own that domain, rename the app to a GitHub-based id
  **`io.github.bresilla.BootDrive`** (verified by owning the public
  `github.com/bresilla/bootdrive` repo). This means renaming the id across
  `data/*.desktop|.metainfo.xml|.svg|.yml`, the D-Bus name stays
  `net.bresilla.BootDrive1` (that's the backend, unaffected).

### Note for reviewers / users

The Flatpak is only the GUI. It talks to the native `bootdrived` backend over
system D-Bus (`--system-talk-name=net.bresilla.BootDrive1`); when the backend
isn't installed the app shows a setup page. The backend ships separately as the
postmarketOS `bootdrived` package (see `packaging/alpine/`), because a Flatpak
sandbox cannot perform the privileged USB-gadget work.

## 3. Local one-off build (optional)

If you *do* want to build locally (x86_64 workstation), inside `nix develop`:

```sh
just flatpak-sources     # refresh data/cargo-sources.json from Cargo.lock
just flatpak-build       # flatpak-builder --user --install
just flatpak-run
```
