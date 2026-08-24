{ pkgs ? import <nixpkgs> {} }:

pkgs.rustPlatform.buildRustPackage {
  pname = "mcmanager";
  version = "1.0.5";
  src = ../..;

  cargoLock.lockFile = ../../Cargo.lock;

  nativeBuildInputs = [ pkgs.pkg-config ];
  buildInputs = [ pkgs.openssl ];

  postInstall = ''
    mkdir -p $out/bin/web
    cp -r ${../../web}/* $out/bin/web/
  '';

  meta = with pkgs.lib; {
    description = "Gestionnaire de serveurs Minecraft (web UI, marketplace Modrinth, playit.gg)";
    homepage = "https://github.com/yo-le-zz/MCmanager";
    license = licenses.mit;
    mainProgram = "mcmanager";
  };
}
