.PHONY: help check lint test scaffold-check c-lint c-test python-lint python-test docs-lint rust-lint rust-test build build-sim simulate host-sample host-fixtures modern-check modern-pi5 modern-m1 clean

JUST ?= mise exec -- just

help:
	@$(JUST) --list

check:
	@$(JUST) check

lint:
	@$(JUST) lint

test:
	@$(JUST) test

scaffold-check:
	@$(JUST) scaffold-check

c-lint:
	@$(JUST) c-lint

c-test:
	@$(JUST) c-test

python-lint:
	@$(JUST) python-lint

python-test:
	@$(JUST) python-test

docs-lint:
	@$(JUST) docs-lint

rust-lint:
	@$(JUST) rust-lint

rust-test:
	@$(JUST) rust-test

build: build-sim

build-sim:
	@$(JUST) build-sim

simulate:
	@$(JUST) simulate

host-sample:
	@$(JUST) host-sample

host-fixtures:
	@$(JUST) host-fixtures

modern-check:
	@$(JUST) modern-check

modern-pi5:
	@$(JUST) modern-pi5

modern-m1:
	@$(JUST) modern-m1

clean:
	@$(JUST) clean
