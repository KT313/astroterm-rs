# Shortcuts for building and running astroterm.
#
#   make build                          build the release binary
#   make run -- -i Tokyo -cCu           run the last built binary with arguments
#   make build-run -- -i Tokyo -m       rebuild if needed, then run with arguments
#   make test                           run all tests (unit, integration and doc tests)
#
# The `--` stops make from reading the arguments as its own options. For values with spaces or `=`, use ARGS:
#   make run ARGS='-i "Rio de Janeiro" --fov=90'

BINARY := target/release/astroterm
ARGS ?=

# words after `run` / `build-run` are arguments for astroterm, not make targets
ifneq ($(filter run build-run,$(firstword $(MAKECMDGOALS))),)
RUN_ARGS := $(wordlist 2,$(words $(MAKECMDGOALS)),$(MAKECMDGOALS))
%:
	@:
endif

.PHONY: build run build-run test

build:
	cargo build --release

run:
	@test -x $(BINARY) || { echo "$(BINARY) not found, run 'make build' first"; exit 1; }
	$(BINARY) $(ARGS) $(RUN_ARGS)

build-run: build run

test:
	cargo test --all-targets
	cargo test --doc
