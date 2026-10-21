{ pkgs, lib, ... }:
let
  # Bevy links against these at build time and dlopens them at run time.
  bevyLibs = with pkgs; [
    alsa-lib
    udev
    vulkan-loader
    wayland
    libxkbcommon
    libx11
    libxcursor
    libxi
    libxrandr
  ];
in
{
  languages.rust = {
    enable = true;
    # Pins the channel and the wasm32-unknown-unknown target.
    toolchainFile = ./rust-toolchain.toml;

    # Keep rust-analyzer as a development-only package.
    lsp.package = pkgs.rust-analyzer;
  };

  packages =
    with pkgs;
    [
      bacon
      cargo-nextest
      just
      pkg-config
      rumdl

      # scripts/fetch-data.sh
      curl
      unzip

      # scripts/web-build.sh. wasm-bindgen-cli must match the `wasm-bindgen` version in
      # Cargo.lock exactly; bump this attribute with it. wasi-sdk still comes from
      # scripts/web-setup.sh (prebuilt; runs here through nix-ld).
      wasm-bindgen-cli_0_2_122
      binaryen
      brotli
      gzip
    ]
    ++ bevyLibs;

  env.LD_LIBRARY_PATH = lib.makeLibraryPath bevyLibs;

  # Fast incremental links for Bevy, on the host target only. devenv's `mold.enable` sets a global
  # RUSTFLAGS, which also reaches wasm32-unknown-unknown and breaks the browser build's link.
  # The rust module already sets this target's linker to clang.
  env.CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS = "-C link-arg=--ld-path=${pkgs.mold}/bin/mold";
}
