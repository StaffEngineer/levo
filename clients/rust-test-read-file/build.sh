#!/bin/bash

set -e

cargo build --target wasm32-wasi --release
wasm-tools component new ../../target/wasm32-wasi/release/test_rust_read_file.wasm \
  -o my-component.wasm --adapt ../wasi_snapshot_preview1.reactor.wasm
wasm-tools component wit my-component.wasm
cargo run --package brotli-encoder --release -- my-component.wasm "../../levo-server/public/read-file.wasm"

