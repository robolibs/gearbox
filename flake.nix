{
  description = "gearbox Rust vehicle/robot simulator development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    # nixGL wraps a command with the host's GPU drivers so OpenGL / Vulkan
    # apps (e.g. Bevy via wgpu) work inside a nix devShell on non-NixOS hosts.
    nixgl.url = "github:nix-community/nixGL";
  };

  outputs =
    { self, nixpkgs, rust-overlay, flake-utils, nixgl, ... }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];

        pkgs = import nixpkgs {
          inherit system overlays;
          config = {
            allowUnfree = true;
            nvidia.acceptLicense = true;
          };
        };

        # .envrc snapshots /proc/driver/nvidia/version into this path before
        # direnv loads the flake. nixGL's auto-detect derivation can't read
        # /proc because the nix build sandbox doesn't mount it; reading a
        # regular file side-steps that entirely. The flake only evaluates
        # with --impure, wired into .envrc.
        nvidiaVersion = let
          firstLine = builtins.head (
            builtins.filter builtins.isString
              (builtins.split "\n" (builtins.readFile ./.direnv/nvidia-version))
          );
          # First line looks like:
          #   NVRM version: NVIDIA UNIX ... Module for x86_64  580.126.09  Release Build ...
          # Version sits between double spaces right before "Release" in the
          # NVRM line, for both classic and "UNIX Open Kernel" driver strings.
          # (nixGL's own "Module  X  " regex misses the newer format.)
          m = builtins.match ".*  ([0-9.]+)  Release.*" firstLine;
        in if m != null then builtins.head m
           else throw "gearbox: couldn't parse .direnv/nvidia-version — is direnv loaded?";

        # Build nixGL pinned to the detected version. `nvidiaHash = null`
        # makes it fetch the matching .run impurely (--impure is wired into
        # .envrc), so this stays automatic as the host driver changes.
        # Note: nixGL still refs xorg.libX11/libxcb/libxshmfence internally,
        # which prints deprecation warnings during eval — upstream bug.
        nixglPkgs = import "${nixgl}/default.nix" {
          inherit pkgs nvidiaVersion;
          nvidiaHash = null;
        };

        # Stable, unversioned `nixGL` / `nixVulkan` aliases — the underlying
        # binaries have the detected driver version baked into their names.
        nixGLAlias = pkgs.runCommand "nixGL" { } ''
          mkdir -p $out/bin
          ln -s ${nixglPkgs.nixGLNvidia}/bin/nixGLNvidia-${nvidiaVersion} $out/bin/nixGL
        '';
        nixVulkanAlias = pkgs.runCommand "nixVulkan" { } ''
          mkdir -p $out/bin
          ln -s ${nixglPkgs.nixVulkanNvidia}/bin/nixVulkanNvidia-${nvidiaVersion} $out/bin/nixVulkan
        '';

        # Runtime libs Bevy needs on Linux (audio, input, windowing, GPU).
        bevyLibs = with pkgs; [
          alsa-lib
          udev
          vulkan-loader
          libxkbcommon
          wayland
          libx11
          libxcursor
          libxi
          libxrandr
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            (pkgs.rust-bin.stable.latest.default.override {
              extensions = [ "rust-src" "rustfmt" "clippy" ];
            })
            pkgs.clang
            pkgs.mold
            pkgs.pkg-config

            # GPU wrappers.
            nixGLAlias
            nixVulkanAlias
            nixglPkgs.nixGLNvidia
            nixglPkgs.nixVulkanNvidia
            nixglPkgs.nixGLIntel      # Mesa fallback (AMD / Intel iGPU)
            nixglPkgs.nixVulkanIntel
          ] ++ bevyLibs;

          RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath bevyLibs;
          # Silence wgpu's validation-layer spam. wgpu's generated SPIR-V
          # uses relaxed atomic ordering that Vulkan 1.3 validation
          # rejects — naga/wgpu upstream bug, harmless at runtime.
          WGPU_VALIDATION = "0";
          WGPU_DEBUG = "0";
        };
      }
    );
}
