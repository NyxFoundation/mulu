# Convenience targets. Requires `lake`/`lean` (elan) and `cargo` on PATH.
# On NixOS: nix-shell -p lean4 cargo rustc --run 'make check'

LEAN_DIR := $(CURDIR)/lean
export MULU_LEAN_DIR := $(LEAN_DIR)

.PHONY: build lean rust test check fixtures ir regen-fixtures clean

build: lean rust

lean:
	cd lean && lake build

rust:
	cargo build --release

test: lean
	cargo test
	cd lean && lake env lean Tests/Fixtures.lean

# Regenerate the committed solc output the mulu-yul tests read.
regen-fixtures:
	./tools/regen-yul-fixtures.sh

ir: build
	./target/release/mulu ir examples/limits/Limits.sol --contract Limits --out analysis-ir-limits

fixtures: build
	@for f in examples/fixtures/*.json examples/limits/model.json; do \
	  out=analysis-$$(basename $$(dirname $$f))-$$(basename $$f .json); \
	  echo "== $$f"; ./target/release/mulu analyze-model $$f --out $$out; code=$$?; \
	  ./target/release/mulu verify $$out || exit 1; \
	done

check: test fixtures

clean:
	rm -rf target lean/.lake analysis-*
