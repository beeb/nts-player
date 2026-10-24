# Rust toolchain fork with Xtensa support
{
  stdenv,
  fetchurl,
  autoPatchelfHook,
  zlib,
  libxml2_13,
}:
let
  version = "1.97.0.0";
  clangVersion = "esp-20.1.1_20250829";
  rustBuild = "https://github.com/esp-rs/rust-build/releases/download/v${version}";
in
stdenv.mkDerivation {
  pname = "esp-rust";
  inherit version;

  srcs = [
    (fetchurl {
      url = "${rustBuild}/rust-${version}-x86_64-unknown-linux-gnu.tar.xz";
      hash = "sha256-qZv+5pIh6f9thjiPaBHuaIzUBeagQAo80XhOjUY+nZk=";
    })
    (fetchurl {
      url = "${rustBuild}/rust-src-${version}.tar.xz";
      hash = "sha256-Vo1oi5+PMy7E0EZXVE+tI+mc4R6ebPWDWXnmiijGi3M=";
    })
    (fetchurl {
      url = "https://github.com/espressif/llvm-project/releases/download/${clangVersion}/libs-clang-${clangVersion}-x86_64-linux-gnu.tar.xz";
      hash = "sha256-ASjiqGsmLj7fzgJgNO3gfi9FcysQxsuKxSLI9Mcr79g=";
    })
  ];
  sourceRoot = ".";

  nativeBuildInputs = [ autoPatchelfHook ];
  buildInputs = [
    zlib
    libxml2_13
    stdenv.cc.cc.lib
  ];

  installPhase = ''
    runHook preInstall
    bash rust-nightly-x86_64-unknown-linux-gnu/install.sh --prefix=$out --disable-ldconfig \
      --without=rust-docs,rust-docs-json-preview
    bash rust-src-nightly/install.sh --prefix=$out --disable-ldconfig
    cp -r esp-clang $out/esp-clang
    runHook postInstall
  '';
}
