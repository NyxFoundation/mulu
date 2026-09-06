#!/usr/bin/env bash
# Regenerate the committed solc output used by mulu-yul's tests.
# Run after changing any example contract; the tests fail if it drifts.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=crates/mulu-yul/tests/fixtures
mkdir -p "$OUT"
cargo run --quiet -p mulu-cli -- ir examples/limits/Limits.sol --contract Limits --out "$OUT/.build" >/dev/null
cp examples/limits/Limits.sol "$OUT/Limits.sol"
cp "$OUT/.build/build/Limits.yul" "$OUT/Limits.yul"
cp "$OUT/.build/build/abi.json" "$OUT/Limits.abi.json"
cp "$OUT/.build/build/storage-layout.json" "$OUT/Limits.storage.json"
rm -rf "$OUT/.build"

# the multi-file, modifier example
cargo run --quiet -p mulu-cli -- ir examples/access/Vault.sol --contract Vault --out "$OUT/.build" >/dev/null
cp examples/access/Base.sol "$OUT/access-Base.sol"
cp examples/access/Vault.sol "$OUT/access-Vault.sol"
cp "$OUT/.build/build/Vault.yul" "$OUT/Vault.yul"
cp "$OUT/.build/build/abi.json" "$OUT/Vault.abi.json"
cp "$OUT/.build/build/storage-layout.json" "$OUT/Vault.storage.json"
rm -rf "$OUT/.build"

echo "regenerated $OUT from examples/ using $(solc --version | tail -1)"
