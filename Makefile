# Shortcuts for building and running astroterm.
#
#   make build                          build the release binary
#   make build-aggressive                     build for this CPU with fat LTO, into target/aggressive
#   make build-aggressive-pgo                 additionally train/apply PGO and BOLT, into target/aggressive-pgo
#   make run -- -i Tokyo -cCu           run the last built binary with arguments
#   make build-run -- -i Tokyo -m       rebuild if needed, then run with arguments
#   make test                           run correctness tests and compile benchmark targets
#   make test-benchmarks                execute benchmark smoke checks (includes 2.5M stars)
#   make clean-stale                    delete build artefacts no build has touched for three days
#
# The `--` stops make from reading the arguments as its own options. For values with spaces or `=`, use ARGS:
#   make run ARGS='-i "Rio de Janeiro" --fov=90'

BINARY := target/release/astroterm
ARGS ?=

# Both aggressive targets use fat LTO, one codegen unit, panic = abort, and code for this CPU.
# build-aggressive-pgo additionally uses PGO and BOLT, both trained by scripts/pgo-training.sh.
# Only build-aggressive-pgo needs cargo-pgo, llvm-tools-preview and BOLT (see check-aggressive-pgo-tools).
# Run the PGO result with
#   make run BINARY=target/aggressive-pgo/astroterm -- -i Tokyo -cCu
HOST := $(shell rustc -vV | sed -n 's/^host: //p')
AGGRESSIVE_DIR := target/$(HOST)/aggressive
AGGRESSIVE_PGO_BINARY := target/aggressive-pgo/astroterm
BOLT_DIR := $(patsubst %/llvm-bolt,%,$(firstword $(wildcard /usr/lib/llvm-*/bin/llvm-bolt)))
AGGRESSIVE_ENV := RUSTFLAGS='-C target-cpu=native'
AGGRESSIVE_PGO_ENV := $(AGGRESSIVE_ENV) PATH="$(BOLT_DIR):$$PATH"

# words after `run` / `build-run` are arguments for astroterm, not make targets
ifneq ($(filter run build-run,$(firstword $(MAKECMDGOALS))),)
RUN_ARGS := $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS))
%:
	@:
endif

.PHONY: build build-aggressive build-aggressive-pgo check-aggressive-pgo-tools run build-run test test-benchmarks clean-stale check-sweep

build:
	cargo build --release

build-aggressive:
	$(AGGRESSIVE_ENV) cargo build --profile aggressive

build-aggressive-pgo: check-aggressive-pgo-tools
	@echo "== 1/3 PGO: instrumented build and training"
	$(AGGRESSIVE_PGO_ENV) cargo pgo build -- --profile aggressive
	scripts/pgo-training.sh $(AGGRESSIVE_DIR)/astroterm
	@echo "== 2/3 BOLT: PGO-optimized, BOLT-instrumented build and training"
	$(AGGRESSIVE_PGO_ENV) cargo pgo bolt build --with-pgo -- --profile aggressive
	scripts/pgo-training.sh $(AGGRESSIVE_DIR)/astroterm-bolt-instrumented
	@echo "== 3/3 final build: PGO and BOLT"
	$(AGGRESSIVE_PGO_ENV) cargo pgo bolt optimize --with-pgo -- --profile aggressive
	mkdir -p $(dir $(AGGRESSIVE_PGO_BINARY))
	cp $(AGGRESSIVE_DIR)/astroterm-bolt-optimized $(AGGRESSIVE_PGO_BINARY)
	@echo "built $(AGGRESSIVE_PGO_BINARY)"

check-aggressive-pgo-tools:
	@command -v cargo-pgo >/dev/null || { echo "cargo-pgo not found: cargo install cargo-pgo"; exit 1; }
	@test -x "$$(rustc --print sysroot)/lib/rustlib/$(HOST)/bin/llvm-profdata" || \
		{ echo "llvm-profdata not found: rustup component add llvm-tools-preview"; exit 1; }
	@test -n "$(BOLT_DIR)" || { echo "BOLT not found: sudo apt install bolt-18 (or another bolt-NN)"; exit 1; }

run:
	@test -x $(BINARY) || { echo "$(BINARY) not found, run 'make build' first"; exit 1; }
	$(BINARY) $(ARGS) $(RUN_ARGS)

build-run: build run

# libtest schedules independent tests in parallel; each test's calculations stay sequential.
# Criterion smoke checks include large workloads, so compile them here and run them separately.
test:
	cargo test --lib --bins --tests --examples
	cargo test --doc
	cargo test --benches --no-run

test-benchmarks:
	cargo test --bench frame --bench spatial

# cargo never deletes artefacts whose hash changed (feature sets, dependency updates, superseded test binaries), so
# target/ grows without bound. This removes what no build has used for three days. The compile-fail tests build a
# private copy of the crate under target/tests/trybuild (host build scripts in debug/, the crate itself under the
# host triple) that sweep does not see; `cargo clean --profile dev` removes only the `debug` directory of the
# tree it is given, nothing else, and the next `cargo test` rebuilds it.
TRYBUILD_DIR := target/tests/trybuild
clean-stale: check-sweep
	cargo sweep --time 3
	cargo clean --profile dev --target-dir $(TRYBUILD_DIR)
	cargo clean --profile dev --target-dir $(TRYBUILD_DIR)/$(HOST)

check-sweep:
	@command -v cargo-sweep >/dev/null || { echo "cargo-sweep not found: cargo install cargo-sweep"; exit 1; }
