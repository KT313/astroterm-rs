# Shortcuts for building and running astroterm.
#
#   make build                          build the release binary
#   make build-aggressive               build a faster binary for this machine (see below), into target/aggressive-pgo
#   make run -- -i Tokyo -cCu           run the last built binary with arguments
#   make build-run -- -i Tokyo -m       rebuild if needed, then run with arguments
#   make test                           run all tests (unit, integration and doc tests)
#
# The `--` stops make from reading the arguments as its own options. For values with spaces or `=`, use ARGS:
#   make run ARGS='-i "Rio de Janeiro" --fov=90'

BINARY := target/release/astroterm
ARGS ?=

# build-aggressive: the `aggressive` Cargo profile (fat LTO, one codegen unit, panic = abort), code for this CPU,
# then profile-guided optimization (PGO) and BOLT, both trained by scripts/pgo-training.sh. Behavior is unchanged.
# Needs cargo-pgo, llvm-tools-preview and BOLT (see check-aggressive-tools). Run the result with
#   make run BINARY=target/aggressive-pgo/astroterm -- -i Tokyo -cCu
HOST := $(shell rustc -vV | sed -n 's/^host: //p')
AGGRESSIVE_DIR := target/$(HOST)/aggressive
AGGRESSIVE_BINARY := target/aggressive-pgo/astroterm
BOLT_DIR := $(patsubst %/llvm-bolt,%,$(firstword $(wildcard /usr/lib/llvm-*/bin/llvm-bolt)))
AGGRESSIVE_ENV := RUSTFLAGS='-C target-cpu=native' PATH="$(BOLT_DIR):$$PATH"

# words after `run` / `build-run` are arguments for astroterm, not make targets
ifneq ($(filter run build-run,$(firstword $(MAKECMDGOALS))),)
RUN_ARGS := $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS))
%:
	@:
endif

.PHONY: build build-aggressive check-aggressive-tools run build-run test

build:
	cargo build --release

build-aggressive: check-aggressive-tools
	@echo "== 1/3 PGO: instrumented build and training"
	$(AGGRESSIVE_ENV) cargo pgo build -- --profile aggressive
	scripts/pgo-training.sh $(AGGRESSIVE_DIR)/astroterm
	@echo "== 2/3 BOLT: PGO-optimized, BOLT-instrumented build and training"
	$(AGGRESSIVE_ENV) cargo pgo bolt build --with-pgo -- --profile aggressive
	scripts/pgo-training.sh $(AGGRESSIVE_DIR)/astroterm-bolt-instrumented
	@echo "== 3/3 final build: PGO and BOLT"
	$(AGGRESSIVE_ENV) cargo pgo bolt optimize --with-pgo -- --profile aggressive
	mkdir -p $(dir $(AGGRESSIVE_BINARY))
	cp $(AGGRESSIVE_DIR)/astroterm-bolt-optimized $(AGGRESSIVE_BINARY)
	@echo "built $(AGGRESSIVE_BINARY)"

check-aggressive-tools:
	@command -v cargo-pgo >/dev/null || { echo "cargo-pgo not found: cargo install cargo-pgo"; exit 1; }
	@test -x "$$(rustc --print sysroot)/lib/rustlib/$(HOST)/bin/llvm-profdata" || \
		{ echo "llvm-profdata not found: rustup component add llvm-tools-preview"; exit 1; }
	@test -n "$(BOLT_DIR)" || { echo "BOLT not found: sudo apt install bolt-18 (or another bolt-NN)"; exit 1; }

run:
	@test -x $(BINARY) || { echo "$(BINARY) not found, run 'make build' first"; exit 1; }
	$(BINARY) $(ARGS) $(RUN_ARGS)

build-run: build run

test:
	cargo test --all-targets
	cargo test --doc
