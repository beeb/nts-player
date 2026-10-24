{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { nixpkgs, ... }:
    let
      pkgs = nixpkgs.legacyPackages.x86_64-linux;
      esp-rust = pkgs.callPackage ./nix/esp-rust.nix { };
    in
    {
      devShells.x86_64-linux.default = pkgs.mkShell {
        packages = with pkgs; [
          esp-rust
          rust-analyzer-unwrapped
          ldproxy
          espflash

          # required for esp-idf build
          (python313.withPackages (ps: [
            ps.pip
            ps.virtualenv
          ]))
          git
          cmake
          ninja
          ccache
        ];

        LIBCLANG_PATH = "${esp-rust}/esp-clang/lib";
        # make esp-idf-sys stop looking for clang at ~/.espup/esp-clang
        ESP_IDF_ESPUP_CLANG_SYMLINK = "ignore";
      };
    };
}
