ifeq ($(strip $(PVSNESLIB_HOME)),)
$(error "Set PVSNESLIB_HOME to a PVSnesLib checkout before building chipviz-snes.sfc")
endif

include $(PVSNESLIB_HOME)/devkitsnes/snes_rules

export ROMNAME := chipviz-snes

.PHONY: all clean

all: $(ROMNAME).sfc

clean: cleanBuildRes cleanRom cleanGfx
