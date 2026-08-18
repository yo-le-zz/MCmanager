{
  description = "MCManager - gestionnaire de serveurs Minecraft (web + Rust)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "mcmanager";
          version = "1.0.0";
          src = ../..;

          cargoLock.lockFile = ../../Cargo.lock;

          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.openssl ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.darwin.apple_sdk.frameworks.Security
          ];

          # Ship the static web UI alongside the binary so mcmanager can find
          # it at runtime (see resolve_web_dir() in src/main.rs).
          postInstall = ''
            mkdir -p $out/share/mcmanager
            cp -r ${../../web} $out/share/mcmanager/web
            mkdir -p $out/bin/web
            cp -r ${../../web}/* $out/bin/web/ 2>/dev/null || true
          '';

          meta = with pkgs.lib; {
            description = "Gestionnaire de serveurs Minecraft avec interface web, marketplace Modrinth et integration playit.gg";
            homepage = "https://github.com/yolezz/mcmanager";
            license = licenses.mit;
            maintainers = [ "yolezz" ];
            mainProgram = "mcmanager";
          };
        };

        apps.default = flake-utils.lib.mkApp {
          drv = self.packages.${system}.default;
          name = "mcmanager";
        };

        devShells.default = pkgs.mkShell {
          buildInputs = [ pkgs.cargo pkgs.rustc pkgs.pkg-config pkgs.openssl ];
        };
      }) // {
        # NixOS module: allows `services.mcmanager.enable = true;` on GLF OS / NixOS.
        nixosModules.default = { config, lib, pkgs, ... }:
          with lib;
          let cfg = config.services.mcmanager;
          in {
            options.services.mcmanager = {
              enable = mkEnableOption "MCManager Minecraft server manager";
              port = mkOption { type = types.port; default = 7777; };
              host = mkOption { type = types.str; default = "127.0.0.1"; };
              package = mkOption { type = types.package; default = self.packages.${pkgs.system}.default; };
            };
            config = mkIf cfg.enable {
              systemd.services.mcmanager = {
                description = "MCManager";
                wantedBy = [ "multi-user.target" ];
                after = [ "network.target" ];
                environment = {
                  MCMANAGER_HOST = cfg.host;
                  MCMANAGER_PORT = toString cfg.port;
                };
                serviceConfig = {
                  ExecStart = "${cfg.package}/bin/mcmanager";
                  Restart = "on-failure";
                  DynamicUser = true;
                  StateDirectory = "mcmanager";
                  Environment = "MCMANAGER_DATA_DIR=/var/lib/mcmanager";
                };
              };
            };
          };
      };
}
