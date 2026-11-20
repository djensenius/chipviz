ROM = chipviz-sms.sms
WLA_Z80 ?= wla-z80
WLALINK ?= wlalink

all: $(ROM)
.PHONY: all candidate clean

$(ROM): ../../../shared/tools/build_homebrew.py
	python3 ../../../shared/tools/build_homebrew.py --target sms --output .
	rm -f homebrew-status.json

candidate:
	python3 build_candidate.py --wla "$(WLA_Z80)" --linker "$(WLALINK)" --output chipviz-sms-candidate.sms

clean:
	rm -f $(ROM) chipviz-sms-candidate.sms homebrew-status.json
