ifeq ($(strip $(PVSNESLIB_HOME)),)
$(error "Set PVSNESLIB_HOME to a PVSnesLib checkout before building chipviz-snes.sfc")
endif

include $(PVSNESLIB_HOME)/devkitsnes/snes_rules

export ROMNAME := chipviz-snes

.PHONY: all clean

all: hdr.asm $(ROMNAME).sfc

hdr.asm: $(PVSNESLIB_HOME)/devkitsnes/include/hdr.asm
	cp $< $@

clean: cleanBuildRes cleanRom cleanGfx
	rm -f hdr.asm
