#!/bin/sh
# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Builds the engine for the browser (docs/DESIGN-web-demo.md): WASI (wasm32-wasip1), the `web` feature, the `wasm` profile, with
# Moonglow's patched copy (scripts/wasm/patches.toml) and nwn-bake with its WASI fix (NWN_BAKE_REV, fetched into
# target/wasm-deps: Cargo takes no patch from the same git URL as the dependency, so it is patched by path). The C and C++ in Moonglow (zstd, glslang, the
# NWScript compiler) build with wasi-sdk: WASI_SDK names its folder, else it is downloaded into target/wasi-sdk.
# Output: target/wasm32-wasip1/wasm/landscaper.wasm, and its size with brotli (scripts/wasm-size.mjs, which fails over the limit).
set -eu
cd "$(dirname "$0")/.."
# plenarius/nwn-bake#20 until its release; then the tag in Cargo.toml has it, and this goes.
NWN_BAKE_REV=4eb1e000077dae4d648beb71562d052f7b653e22
bake=target/wasm-deps/nwn-bake
if [ "$(git -C "$bake" rev-parse HEAD 2> /dev/null)" != "$NWN_BAKE_REV" ]; then
    rm -rf "$bake" && mkdir -p "$bake"
    git -C "$bake" init -q
    git -C "$bake" fetch -q --depth 1 https://github.com/plenarius/nwn-bake "$NWN_BAKE_REV"
    git -C "$bake" checkout -q FETCH_HEAD
fi
version=34
sdk=${WASI_SDK:-target/wasi-sdk/wasi-sdk-$version.0-x86_64-linux}
if [ ! -x "$sdk/bin/clang" ]; then
    mkdir -p target/wasi-sdk
    curl -sSfL --retry 5 --retry-all-errors --retry-delay 5 "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-$version/wasi-sdk-$version.0-x86_64-linux.tar.gz" |
        tar -xz -C target/wasi-sdk
fi
sdk=$(cd "$sdk" && pwd)
rustup target add wasm32-wasip1 > /dev/null
export CC_wasm32_wasip1="$sdk/bin/clang" CXX_wasm32_wasip1="$sdk/bin/clang++" AR_wasm32_wasip1="$sdk/bin/llvm-ar"
export CFLAGS_wasm32_wasip1="--sysroot=$sdk/share/wasi-sysroot"
# (No C++ exceptions: WASI has none to unwind, and the NWScript compiler throws none.)
export CXXFLAGS_wasm32_wasip1="--sysroot=$sdk/share/wasi-sysroot -fno-exceptions"
export CARGO_TARGET_WASM32_WASIP1_RUSTFLAGS="-L native=$sdk/share/wasi-sysroot/lib/wasm32-wasip1/noeh -l static=c++abi"
# The patches change Cargo.lock's sources: the browser build locks into a copy, so the native lock stays as it is.
cp Cargo.lock target/Cargo.lock.native
trap 'cp target/Cargo.lock.native Cargo.lock' EXIT
set -- --config scripts/wasm/patches.toml --config "patch.\"https://github.com/plenarius/nwn-bake\".nwn-bake.path=\"$PWD/$bake/nwn-bake\""
# (The fix's nwn-bake may be a newer version than the locked one, which Cargo would keep.)
cargo update -p nwn-bake "$@"
cargo build --target wasm32-wasip1 --profile wasm --features web "$@"
node scripts/wasm-size.mjs target/wasm32-wasip1/wasm/landscaper.wasm
