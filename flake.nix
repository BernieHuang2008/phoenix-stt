{
  description = "Phoenix STT — a small voice input method for NixOS + niri";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f (import nixpkgs { inherit system; }));
    in
    {
      packages = forAllSystems (pkgs:
        let
          runtimeDeps = with pkgs; [
            whisper-cpp
            wl-clipboard
            wtype
            pipewire
            curl
          ];

          phoenix-stt = pkgs.rustPlatform.buildRustPackage ({
            pname = "phoenix-stt";
            version = "0.1.0";
            src = pkgs.lib.cleanSourceWith {
              src = ./.;
              filter = path: _type:
                let base = baseNameOf path;
                in base != "target" && base != "result" && base != ".git";
            };

            nativeBuildInputs = with pkgs; [
              pkg-config
              wrapGAppsHook4
            ];

            buildInputs = with pkgs; [
              gtk4
              gtk4-layer-shell
            ];

            preFixup = ''
              gappsWrapperArgs+=(
                --prefix PATH : ${pkgs.lib.makeBinPath runtimeDeps}
                --set GDK_BACKEND wayland
              )
            '';

            meta = with pkgs.lib; {
              description = "Phoenix voice input method (whisper + local LLM)";
              license = licenses.mit;
              mainProgram = "phoenix-stt";
              platforms = platforms.linux;
            };
          } // (
            if builtins.pathExists ./vendor
            then { cargoVendorDir = "vendor"; }
            else { cargoLock.lockFile = ./Cargo.lock; }
          ));
        in
        {
          default = phoenix-stt;
          phoenix-stt = phoenix-stt;
        });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            pkg-config
            gobject-introspection
            gtk4
            gtk4-layer-shell
            whisper-cpp
            wl-clipboard
            wtype
            pipewire
            curl
          ];

          shellHook = ''
            export GDK_BACKEND=wayland
            echo "phoenix-stt dev shell"
            echo "  cargo check / cargo build"
            echo "  model: set PHOENIX_STT_WHISPER_MODEL to ggml-small-q5_1.bin"
          '';
        };
      });

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/phoenix-stt";
          meta = {
            description = "Phoenix voice input method";
          };
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt);
    };
}
