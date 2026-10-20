#!/usr/bin/env bash
# Build every mini-app into a WASM *component*, in two steps per app:
#
#   1. cargo build --target wasm32-unknown-unknown   -> a core module
#      (wit-bindgen embeds the world's type info as custom sections)
#   2. componentize <core> <component>               -> a Component-Model component
#      (our tiny tool wraps wit-component; no external CLI needed)
#
# The host then loads the resulting  mini-apps/<app>/<app>.component.wasm .
set -euo pipefail
cd "$(dirname "$0")"

# One-time: the WASM target used to build mini-apps.
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true

# Build the componentizer once (native binary), and the packer that gives an app
# its manifest (ADR-1).
cargo build --release -p componentize -p nimadir-cli

APPS=(counter hello gallery rates store runaway future fetcher fuzz)
for app in "${APPS[@]}"; do
  echo "==> building mini-app: $app"
  cargo build --release --target wasm32-unknown-unknown \
    --manifest-path "mini-apps/$app/Cargo.toml"
  core="mini-apps/$app/target/wasm32-unknown-unknown/release/$app.wasm"
  out="mini-apps/$app/$app.component.wasm"
  ./target/release/componentize "$core" "$out"
  # An app with a manifest.json carries it inside — and is signed with it when
  # NIMADIR_SIGNING_KEY names a key (`nimadir keygen`), as the registry
  # workflow can from a secret.
  if [[ -f "mini-apps/$app/manifest.json" ]]; then
    sign=()
    [[ -n "${NIMADIR_SIGNING_KEY:-}" ]] && sign=(--key "$NIMADIR_SIGNING_KEY")
    ./target/release/nimadir pack "$out" "mini-apps/$app/manifest.json" "$out" "${sign[@]}" >/dev/null
  fi
done

# The C# mini-app is built by the .NET SDK (componentize-dotnet), not cargo. It is
# optional: skipped cleanly when `dotnet` isn't on PATH, so the Rust apps still
# build everywhere. Needs the .NET 10 SDK; its toolchain (NativeAOT-LLVM + the
# WASI SDK) is downloaded and cached on the first build (a few minutes once).
#
# The SDK version is checked, not just `dotnet`: CI images and many machines
# carry an older SDK, which would fail the build outright rather than skip it.
if command -v dotnet >/dev/null 2>&1 && dotnet --list-sdks 2>/dev/null | grep -q '^10\.'; then
  echo "==> building mini-app: counter-cs (C#, componentize-dotnet)"
  dotnet build -c Release mini-apps/counter-cs/counter-cs.csproj
  cp mini-apps/counter-cs/bin/Release/net10.0/wasi-wasm/publish/counter_cs.wasm \
     mini-apps/counter-cs/counter-cs.component.wasm
else
  echo "==> skipping counter-cs (C#): no .NET 10 SDK found on PATH"
fi

# The Python mini-app is built by componentize-py (`pip install componentize-py`),
# which bundles a CPython interpreter into the component. Optional in the same
# way the C# one is.
#
# `pip` puts the launcher in the interpreter's scripts directory, which on
# Windows is routinely not on PATH — so ask Python where it put it rather than
# reporting "not installed" for a tool that is installed.
cpy=$(command -v componentize-py 2>/dev/null || true)
if [[ -z "$cpy" ]]; then
  cpy=$(python -c "import os,sysconfig
d = sysconfig.get_path('scripts')
for name in ('componentize-py', 'componentize-py.exe'):
    p = os.path.join(d, name)
    if os.path.exists(p):
        print(p); break" 2>/dev/null || true)
fi

if [[ -n "$cpy" ]]; then
  echo "==> building mini-app: counter-py (Python, componentize-py)"
  "$cpy" -d wit -w mini-app componentize app \
    -p mini-apps/counter-py \
    -o mini-apps/counter-py/counter-py.component.wasm
else
  echo "==> skipping counter-py (Python): componentize-py not installed"
fi

echo
echo "components ready:"
ls -la mini-apps/*/*.component.wasm
