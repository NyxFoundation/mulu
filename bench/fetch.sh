#!/usr/bin/env bash
# Fetch the corpora. Not vendored: solc is GPL-3.0 and mulu is MIT, so the
# tests are read as data at measurement time rather than copied in.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$here/corpus"

if [ ! -d "$here/corpus/solidity" ]; then
  git clone --depth 1 --filter=blob:none --sparse \
    https://github.com/argotorg/solidity.git "$here/corpus/solidity"
  git -C "$here/corpus/solidity" sparse-checkout set test/libsolidity/semanticTests
fi
echo "semanticTests: $(find "$here/corpus/solidity/test/libsolidity/semanticTests" -name '*.sol' | wc -l) case(s)"

if [ ! -d "$here/corpus/contracts-verification-benchmark" ]; then
  git clone --depth 1 https://github.com/fsainas/contracts-verification-benchmark.git \
    "$here/corpus/contracts-verification-benchmark"
fi
echo "verification benchmark: $(find "$here/corpus/contracts-verification-benchmark" -name '*.sol' | wc -l) contract(s)"
