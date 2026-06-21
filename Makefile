.PHONY: help check

help:
	@echo "chipviz targets:"
	@echo "  make check  - validate scaffold files"

check:
	@test -f README.md
	@test -f shared/specs/control-frame-v0.md
	@test -f shared/specs/control-frame-v0.json
	@test -d cores/n64
	@test -d cores/gba
	@test -d cores/c64
	@echo "chipviz scaffold OK"

