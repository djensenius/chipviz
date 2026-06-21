.PHONY: help check scaffold-check build-sim simulate host-sample clean

CC ?= cc
PYTHON ?= python3
CFLAGS ?= -std=c99 -Wall -Wextra -Werror -pedantic -Ishared/include

SIM_PLATFORMS := n64 gba c64
SIM_TARGETS := $(SIM_PLATFORMS:%=build/sim/%)
SHARED_SRCS := shared/src/control_frame.c shared/src/connection.c

help:
	@echo "chipviz targets:"
	@echo "  make check       - validate and build the portable scaffold"
	@echo "  make build-sim   - build native simulator binaries"
	@echo "  make simulate    - run one smoke pass per platform"
	@echo "  make host-sample - emit sample raw control frames"
	@echo "  make clean       - remove generated files"

check: scaffold-check build-sim simulate host-sample
	@echo "chipviz check OK"

scaffold-check:
	@test -f README.md
	@test -f shared/specs/control-frame-v0.md
	@test -f shared/specs/control-frame-v0.json
	@test -f shared/include/chipviz/control_frame.h
	@test -f shared/include/chipviz/connection.h
	@test -f host/bridge/chipviz_bridge.py
	@test -f cores/n64/src/main.c
	@test -f cores/gba/src/main.c
	@test -f cores/c64/src/main.c
	@echo "chipviz scaffold OK"

build-sim: $(SIM_TARGETS)

build/sim/%: cores/%/src/main.c $(SHARED_SRCS) | build/sim
	@$(CC) $(CFLAGS) -DCHIPVIZ_SIMULATOR $^ -o $@

simulate: build-sim
	@for target in $(SIM_PLATFORMS); do build/sim/$$target >/dev/null; done
	@echo "chipviz platform simulations OK"

host-sample: build/control-frame-v0.cvz
	@test -s build/control-frame-v0.cvz
	@echo "chipviz host bridge sample OK"

build/control-frame-v0.cvz: host/bridge/chipviz_bridge.py | build
	@$(PYTHON) host/bridge/chipviz_bridge.py --frames 8 --output $@

build/sim build:
	@mkdir -p $@

clean:
	@rm -rf build
