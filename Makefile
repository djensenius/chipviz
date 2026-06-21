.PHONY: help check scaffold-check build build-sim simulate host-sample modern-check modern-pi5 modern-m1 clean

JUST ?= mise exec -- just

help:
	@$(JUST) --list

check:
	@$(JUST) check

scaffold-check:
	@$(JUST) scaffold-check

build: build-sim

build-sim:
	@$(JUST) build-sim

simulate:
	@$(JUST) simulate

host-sample:
	@$(JUST) host-sample

modern-check:
	@$(JUST) modern-check

modern-pi5:
	@$(JUST) modern-pi5

modern-m1:
	@$(JUST) modern-m1

clean:
	@$(JUST) clean
