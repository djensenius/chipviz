TARGET := chipviz-c64
BUILD := build
SRC := src/main.c src/colour_field.c
MAP := $(BUILD)/$(TARGET).map
# The colour-field screens live at $3000-$3FFF; code, data and BSS must end below.
FIELD_SCREENS := 0x3000
# Flash/gate is compiled out of release builds until it is measured (#96).
# UNMEASURED_FLASH=1 builds a local measurement ROM that can arm it.
UNMEASURED_FLASH ?= 0
FLASH_DEFINE := $(if $(filter 1,$(UNMEASURED_FLASH)),-DCF_UNMEASURED_FLASH)
# Rebuild when the flag changes: checked while parsing, because a same-second
# rebuild cannot be detected from timestamps.
ifneq ($(FLASH_DEFINE),$(shell cat $(BUILD)/flags 2>/dev/null))
$(shell mkdir -p $(BUILD) && echo '$(FLASH_DEFINE)' > $(BUILD)/flags && rm -f $(TARGET).prg)
endif

.PHONY: all clean

all: $(TARGET).prg

$(BUILD):
	mkdir -p $@

$(BUILD)/flags: | $(BUILD)
	echo '$(FLASH_DEFINE)' > $@

$(TARGET).prg: $(SRC) src/colour_field.h $(BUILD)/flags | $(BUILD)
	cl65 -t c64 -Oirs --static-locals $(FLASH_DEFINE) -m $(MAP) -o $@ $(SRC)
	@run=$$(awk '{ for (i = 1; i < NF; i++) if ($$i == "__BSS_RUN__") print $$(i + 1) }' $(MAP) | head -n 1); \
	  size=$$(awk '{ for (i = 1; i < NF; i++) if ($$i == "__BSS_SIZE__") print $$(i + 1) }' $(MAP) | head -n 1); \
	  end=$$((0x$$run + 0x$$size)); \
	  if [ "$$end" -le 2049 ] || [ "$$end" -gt $$(($(FIELD_SCREENS))) ]; then \
	    echo "program end $$end overlaps colour-field screens at $(FIELD_SCREENS)" >&2; rm -f $@; exit 1; fi

clean:
	rm -rf $(BUILD) $(TARGET).prg
