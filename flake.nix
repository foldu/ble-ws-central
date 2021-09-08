{
  description = "A thing.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }: {
    overlay = final: prev: {
      ble-ws-central =
        let
          pkgs = nixpkgs.legacyPackages.${prev.system};
          cargoToml = builtins.fromTOML (builtins.readFile "${self}/Cargo.toml");
          version = cargoToml.package.version;
          pname = cargoToml.package.name;
        in
        pkgs.rustPlatform.buildRustPackage {
          inherit version pname;
          src = self;
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "ble-ws-api-0.1.0" = "sha256-5/xB2c3nYWB3Jm7NWKsE5gy3MGdceT/mu5Ssh90oTbw=";
              "zbus-2.0.0-beta.6" = "sha256-5RcD6SDp0oHmpXYdosdATKrg67Nki5HPgZD4l/24JBo=";
            };
          };
          PROTOC = "${pkgs.protobuf}/bin/protoc";
          PROTOC_INCLUDE = "${pkgs.protobuf}/include";
          nativeBuildInputs = with pkgs; [
            pkg-config
            rustfmt
          ];
        };
    };
  } // flake-utils.lib.eachDefaultSystem (
    system:
    let
      pkgs = import nixpkgs { inherit system; overlays = [ self.overlay ]; };
    in
    {
      defaultPackage = pkgs.ble-ws-central;
      nixosModule = import ./nix/module.nix;
      defaultApp = {
        type = "app";
        program = "${self.defaultPackage.${system}}/bin/ble-ws-central";
      };
      devShell = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [
          pkg-config
        ];
        PROTOC = "${pkgs.protobuf}/bin/protoc";
        PROTOC_INCLUDE = "${pkgs.protobuf}/include";
      };
    }
  );
}
