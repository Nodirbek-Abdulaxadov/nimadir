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

# Build the componentizer once (native binary).
cargo build --release -p componentize

APPS=(counter hello)
for app in "${APPS[@]}"; do
  echo "==> building mini-app: $app"
  cargo build --release --target wasm32-unknown-unknown \
    --manifest-path "mini-apps/$app/Cargo.toml"
  core="mini-apps/$app/target/wasm32-unknown-unknown/release/$app.wasm"
  out="mini-apps/$app/$app.component.wasm"
  ./target/release/componentize "$core" "$out"
done

echo
echo "components ready:"
ls -la mini-apps/*/*.component.wasm
