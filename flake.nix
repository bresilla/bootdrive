{
  description = "BootDrive development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs?rev=4c1018dae018162ec878d42fec712642d214fdfa";
    flake-utils.url = "github:numtide/flake-utils";
    nixgl.url = "github:nix-community/nixGL";
  };

  outputs =
    { nixpkgs, flake-utils, nixgl, ... }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          config = {
            allowUnfree = true;
          };
        };

        # GTK4 / libadwaita GUI runtime + build libraries.
        guiLibs = with pkgs; [
          gtk4
          libadwaita
          glib
          gsettings-desktop-schemas
          graphene
          cairo
          pango
          gdk-pixbuf
          harfbuzz
          librsvg
          dbus
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            # Rust toolchain.
            rustc
            cargo
            rustfmt
            clippy
            rust-analyzer
            cargo-edit
            cargo-watch
            cargo-nextest

            # Build glue.
            pkg-config
            clang
            mold

            # System integration.
            dbus
            polkit

            # Packaging / validation.
            flatpak
            flatpak-builder
            # Python with the modules the Flatpak Cargo generator needs.
            (python3.withPackages (ps: [ ps.aiohttp ps.tomlkit ]))
            appstream
            appstream-glib
            desktop-file-utils
            gettext
            just
            git-cliff

            # GPU launcher wrapper for running the GUI on Arch (nixGL).
            nixgl.packages.${system}.nixGLIntel
          ] ++ guiLibs;

          shellHook = ''
            export XDG_DATA_DIRS="${pkgs.gsettings-desktop-schemas}/share:${pkgs.gtk4}/share:${pkgs.libadwaita}/share''${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"
            export RUST_LOG="bootdrive_gui=debug,bootdrived=debug"
          '';
        };
      }
    );
}
