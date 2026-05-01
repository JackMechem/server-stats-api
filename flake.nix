{
    description = "server-dash-api - system stats & command execution REST API in Rust";
    inputs = {
        nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
        rust-overlay = {
            url = "github:oxalica/rust-overlay";
            inputs.nixpkgs.follows = "nixpkgs";
        };
        flake-utils.url = "github:numtide/flake-utils";
    };
    outputs =
        {
            self,
            nixpkgs,
            rust-overlay,
            flake-utils,
            ...
        }:
        flake-utils.lib.eachDefaultSystem (
            system:
            let
                overlays = [ (import rust-overlay) ];
                pkgs = import nixpkgs { inherit system overlays; };
                rustToolchain = pkgs.rust-bin.stable.latest.default.override {
                    extensions = [
                        "rust-src"
                        "rust-analyzer"
                        "clippy"
                        "rustfmt"
                    ];
                };
                nativeBuildInputs = with pkgs; [
                    rustToolchain
                    pkg-config
                ];
                buildInputs = with pkgs; [
                    openssl
                    linux-pam
                    libclang
                    glibc.dev
                    gnumake
                ];
                package = pkgs.rustPlatform.buildRustPackage {
                    pname = "server-dash-api";
                    version = "0.1.0";
                    src = ./.;
                    cargoLock.lockFile = ./Cargo.lock;
                    inherit nativeBuildInputs buildInputs;
                    OPENSSL_NO_VENDOR = 1;
                    PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
                    LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
                    BINDGEN_EXTRA_CLANG_ARGS = "-I${pkgs.linux-pam}/include -I${pkgs.glibc.dev}/include";
                };
            in
            {
                packages.default = package;
                devShells.default = pkgs.mkShell {
                    inherit nativeBuildInputs buildInputs;
                    PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
                    OPENSSL_DIR = "${pkgs.openssl.dev}";
                    OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
                    OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include";
                    LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
                    BINDGEN_EXTRA_CLANG_ARGS = "-I${pkgs.linux-pam}/include -I${pkgs.glibc.dev}/include";
                    RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
                    shellHook = ''
                        echo "🦀 server-dash-api dev shell ready"
                        echo "   rustc  $(rustc --version)"
                        echo "   cargo  $(cargo --version)"
                    '';
                };
            }
        )
        // {
            nixosModules.default =
                {
                    config,
                    pkgs,
                    lib,
                    ...
                }:
                {
                    options.services.server-dash-api = {
                        enable = lib.mkEnableOption "server-dash-api system stats API";
                        useNixBuild = lib.mkOption {
                            type = lib.types.bool;
                            default = false;
                            description = "Build the binary via Nix instead of using a manually deployed binary";
                        };
                    };

                    config = lib.mkIf config.services.server-dash-api.enable {
                        users.users.server-dash-api = {
                            isSystemUser = true;
                            group = "server-dash-api";
                            extraGroups = [ "shadow" ];
                            home = "/var/lib/server-dash-api";
                            createHome = true;
                        };
                        users.groups.server-dash-api = { };

                        systemd.tmpfiles.rules = [
                            "d /var/lib/server-dash-api 0750 server-dash-api server-dash-api -"
                            "d /var/lib/server-dash-api/webauthn-credentials 0750 server-dash-api server-dash-api -"
                        ];

                        security.polkit.extraConfig = ''
                            polkit.addRule(function(action, subject) {
                                if ((action.id == "org.freedesktop.systemd1.manage-units" ||
                                     action.id == "org.freedesktop.login1.reboot" ||
                                     action.id == "org.freedesktop.login1.reboot-multiple-sessions" ||
                                     action.id == "org.freedesktop.login1.reboot-ignore-inhibit" ||
                                     action.id == "org.freedesktop.login1.power-off" ||
                                     action.id == "org.freedesktop.login1.power-off-multiple-sessions" ||
                                     action.id == "org.freedesktop.login1.power-off-ignore-inhibit" ||
                                     action.id == "org.freedesktop.login1.halt" ||
                                     action.id == "org.freedesktop.login1.halt-multiple-sessions" ||
                                     action.id == "org.freedesktop.login1.halt-ignore-inhibit") &&
                                    subject.user == "server-dash-api") {
                                    return polkit.Result.YES;
                                }
                            });
                        '';

                        systemd.services.server-dash-api = {
                            description = "server-dash-api - Rust System Stats API";
                            after = [ "network.target" ];
                            wantedBy = [ "multi-user.target" ];
                            serviceConfig = {
                                Type = "simple";
                                User = "server-dash-api";
                                Group = "server-dash-api";
                                SupplementaryGroups = [ "shadow" ];
                                ExecStart =
                                    if config.services.server-dash-api.useNixBuild then
                                        "${self.packages.${pkgs.system}.default}/bin/server-dash-api"
                                    else
                                        "/var/lib/server-dash-api/server-dash-api";
                                Restart = "on-failure";
                                RestartSec = "10s";
                                StateDirectory = "server-dash-api";
                                Environment = [
                                    "RUST_LOG=info"
                                    "SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"
                                ];
                                AmbientCapabilities = [ "CAP_DAC_READ_SEARCH" ];
                                CapabilityBoundingSet = [ "CAP_DAC_READ_SEARCH" ];
                            };
                        };
                    };
                };
        };
}
