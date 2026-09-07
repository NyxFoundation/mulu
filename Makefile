# Convenience targets. Requires `lake`/`lean` (elan) and `cargo` on PATH.
# On NixOS: nix-shell -p lean4 cargo rustc --run 'make check'

LEAN_DIR := $(CURDIR)/lean
export MULU_LEAN_DIR := $(LEAN_DIR)

.PHONY: build lean rust test check fixtures ir analyze regen-fixtures semantics semantics-check semantics-diff semantics-probe clean

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

# The Limits example is meant to contain a specification violation, so analyze
# exits 1 here. Anything other than 0 or 1 is a real failure.
analyze: build
	./target/release/mulu analyze examples/limits/Limits.sol --contract Limits \
	  --spec examples/limits/Limits.spec.json --out analysis-limits; \
	  code=$$?; [ $$code -le 1 ] || exit $$code
	./target/release/mulu verify analysis-limits
	./target/release/mulu analyze examples/access/Vault.sol --contract Vault \
	  --spec examples/access/Vault.spec.json --out analysis-vault; \
	  code=$$?; [ $$code -le 1 ] || exit $$code
	./target/release/mulu verify analysis-vault
	./target/release/mulu analyze examples/typed/Meter.sol --contract Meter \
	  --spec examples/typed/Meter.spec.json --out analysis-meter; \
	  code=$$?; [ $$code -le 1 ] || exit $$code
	./target/release/mulu verify analysis-meter
	./target/release/mulu analyze examples/overload/Over.sol --contract Over \
	  --spec examples/overload/Over.spec.json --out analysis-over; \
	  code=$$?; [ $$code -le 1 ] || exit $$code
	./target/release/mulu verify analysis-over
	./target/release/mulu analyze examples/guards/Gate.sol --contract Gate \
	  --spec examples/guards/Gate.spec.json --out analysis-gate; \
	  code=$$?; [ $$code -le 1 ] || exit $$code
	./target/release/mulu verify analysis-gate

fixtures: build
	@for f in examples/models/*.json examples/limits/model.json; do \
	  out=analysis-$$(basename $$(dirname $$f))-$$(basename $$f .json); \
	  echo "== $$f"; ./target/release/mulu analyze-model $$f --out $$out; code=$$?; \
	  ./target/release/mulu verify $$out || exit 1; \
	done

# Opt in. `semantics/` depends on EvmYul, which depends on mathlib: several
# gigabytes and a network fetch. `check` must stay buildable from Lean core
# alone, because the point of the kernel re-check is that a reader can
# reproduce it without trusting a supply chain. Nothing in `analyze` or
# `verify` imports this.
semantics:
	cd semantics && lake build

# The generated modules have to be accepted by the semantics they are written
# for. Rendering that only mulu can read would prove nothing about anything.
semantics-check: semantics
	cd semantics && lake build EvmYul.Yul.YulNotation
	@set -e; for f in examples/limits/Limits examples/access/Vault examples/typed/Meter \
	                  examples/overload/Over examples/guards/Gate; do \
	  n=$$(basename $$f); \
	  ./target/release/mulu yul-lean $$f.sol --contract $$n --out /tmp/mulu-yul-lean; \
	  (cd semantics && lake env lean /tmp/mulu-yul-lean/$$n.lean); \
	  echo "$$n elaborates"; \
	done

# Run every example in the Lean semantics and on revm, and compare. Agreement
# is evidence, never a proof: the obligation stays open either way. What this
# catches is a rendering that is a different program.
# What the adopted semantics does with programs whose meaning the Yul
# specification fixes. Two of the four cases still differ from it.
semantics-probe: semantics
	cd semantics && lake build mulu-semantics-probe && ./.lake/build/bin/mulu-semantics-probe

semantics-diff: semantics
	./target/release/mulu semantics-diff examples/limits/Limits.sol --contract Limits \
	  --call 'setLimit(uint256)=50' --call 'setLimit(uint256)=101' --call 'forceSet(uint256)=1001'
	./target/release/mulu semantics-diff examples/access/Vault.sol --contract Vault \
	  --call 'setLimit(uint256)=50' --call 'setLimit(uint256)=101' --call 'forceSet(uint256)=2000'
	./target/release/mulu semantics-diff examples/typed/Meter.sol --contract Meter \
	  --call 'record(uint8)=50' --call 'record(uint8)=200' --call 'force(uint256)=5000'
	./target/release/mulu semantics-diff examples/overload/Over.sol --contract Over \
	  --call 'set(uint256)=500' --call 'set(uint8)=200' --call 'set(uint8)=50'
	./target/release/mulu semantics-diff examples/guards/Gate.sol --contract Gate \
	  --call 'setLimit(uint256)=50' --call 'setLimit(uint256)=2000' --call 'forceSet(uint256)=9999'

check: test fixtures analyze

clean:
	rm -rf target lean/.lake analysis-*
