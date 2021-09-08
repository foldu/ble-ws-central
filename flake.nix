{
  description = "A thing.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        nativeBuildInputs = with pkgs; [
          pkg-config
          rustfmt
        ];
      in
      {
        defaultPackage = pkgs.rustPlatform.buildRustPackage
          {
            name = "ble-ws-central";
            src = self;
            cargoLock = {
              lockFile = ./Cargo.lock;
            };
            PROTOC = "${pkgs.protobuf}/bin/protoc";
            PROTOC_INCLUDE = "${pkgs.protobuf}/include";
            inherit nativeBuildInputs;
          };
        defaultApp = {
          type = "app";
          program = "${self.defaultPackage.${system}}/bin/ble-ws-central";
        };
        devShell = pkgs.mkShell {
          inherit nativeBuildInputs;
          PROTOC = "${pkgs.protobuf}/bin/protoc";
          PROTOC_INCLUDE = "${pkgs.protobuf}/include";
        };
      }
    );
}
