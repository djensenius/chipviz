TARGET := chipviz-c64
BUILD := build
SRC := src/main.c src/colour_field.c
MAP := $(BUILD)/$(TARGET).map
# The colour-field screens live at $3000-$3FFF; code, data and BSS must end below.
FIELD_SCREENS := 0x3000

.PHONY: all clean

all: $(TARGET).prg

$(BUILD):
	mkdir -p $@

$(TARGET).prg: $(SRC) src/colour_field.h | $(BUILD)
	cl65 -t c64 -Oirs --static-locals -m $(MAP) -o $@ $(SRC)
	@run=$$(awk '{ for (i = 1; i < NF; i++) if ($$i == "__BSS_RUN__") print $$(i + 1) }' $(MAP) | head -n 1); \
	  size=$$(awk '{ for (i = 1; i < NF; i++) if ($$i == "__BSS_SIZE__") print $$(i + 1) }' $(MAP) | head -n 1); \
	  end=$$((0x$$run + 0x$$size)); \
	  if [ "$$end" -le 2049 ] || [ "$$end" -gt $$(($(FIELD_SCREENS))) ]; then \
	    echo "program end $$end overlaps colour-field screens at $(FIELD_SCREENS)" >&2; rm -f $@; exit 1; fi

clean:
	rm -rf $(BUILD) $(TARGET).prg
