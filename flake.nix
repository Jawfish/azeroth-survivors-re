{
  description = "Azeroth Survivors";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        rustPlatform = pkgs.makeRustPlatform {
          cargo = toolchain;
          rustc = toolchain;
        };

        gameToml = builtins.fromTOML (builtins.readFile ./crates/warcraft-survivors/Cargo.toml);
        workspaceToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);

        # Bevy dlopens these at run time.
        runtimeLibs = with pkgs; [ alsa-lib udev vulkan-loader wayland libxkbcommon libx11 libxcursor libxi libxrandr ];
      in
      {
        # The native game. It still needs a 1.12.1 client at run time: WOW_DATA=/path/to/Data.
        packages.default = rustPlatform.buildRustPackage {
          pname = gameToml.package.name;
          inherit (workspaceToml.workspace.package) version;
          cargoBuildFlags = [ "-p" gameToml.package.name ];
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          # The workspace tests need a WoW install; the dev shell runs them.
          doCheck = false;

          # Bevy links against these system libraries.
          nativeBuildInputs = [ pkgs.pkg-config pkgs.makeWrapper ];
          buildInputs = with pkgs; [ alsa-lib udev wayland libxkbcommon ];

          postFixup = ''
            wrapProgram $out/bin/${gameToml.package.name} \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath runtimeLibs}
          '';
        };

        checks.default = self.packages.${system}.default;
      });
}
