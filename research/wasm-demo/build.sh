#!/bin/sh
# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Spike (wasm-demo): builds the engine for WASI (wasm32-wasip1). Moonglow and nwn-bake are cloned beside this repository at the
# versions Cargo.toml pins and patched (moonglow-wasi.patch, nwn-bake-wasi.patch); the C and C++ (zstd, glslang, the NWScript
# compiler) build with wasi-sdk, fetched into ../tools. Output: target/wasm32-wasip1/release/landscaper.wasm.
set -e
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
parent=$(dirname "$repo")
cd "$parent"
[ -d moonglow ] || { git clone -q https://github.com/jadzziaa/moonglow-toolset.git moonglow && git -C moonglow checkout -q ac17ade3292c9e9a81ac72efbd75411899cf4583 && git -C moonglow apply "$here/moonglow-wasi.patch"; }
[ -d nwn-bake ] || { git clone -q --branch v0.7.0 https://github.com/plenarius/nwn-bake.git nwn-bake && git -C nwn-bake apply "$here/nwn-bake-wasi.patch"; }
sdk=wasi-sdk-34.0-x86_64-linux
[ -d "tools/$sdk" ] || { mkdir -p tools && curl -sSL "https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/$sdk.tar.gz" | tar xz -C tools; }
W="$parent/tools/$sdk"
rustup target add wasm32-wasip1
export CC_wasm32_wasip1="$W/bin/clang" CXX_wasm32_wasip1="$W/bin/clang++" AR_wasm32_wasip1="$W/bin/llvm-ar"
export CFLAGS_wasm32_wasip1="--sysroot=$W/share/wasi-sysroot" CXXFLAGS_wasm32_wasip1="--sysroot=$W/share/wasi-sysroot -fno-exceptions"
export CARGO_TARGET_WASM32_WASIP1_RUSTFLAGS="-L native=$W/share/wasi-sysroot/lib/wasm32-wasip1/noeh -l static=c++abi"
cd "$repo"
cargo build --release --target wasm32-wasip1
ls -la target/wasm32-wasip1/release/landscaper.wasm
