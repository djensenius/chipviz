TARGET := chipviz-c64
BUILD := build
SRC := src/main.c src/colour_field.c
HEADERS := src/colour_field.h
CFLAGS := -t c64 -Oirs --static-locals
# The colour-field screens live at $3000-$3FFF; code, data and BSS must end below.
FIELD_SCREENS := 0x3000

# Flash/gate is compiled out of the release ROM until it is measured (#96).
# UNMEASURED_FLASH=1 builds a separate local measurement ROM that can arm it,
# with its own name and object directory, so it can never replace the
# release artefact $(TARGET).prg. Do not ship it.
FLASH_TARGET := $(TARGET)-unmeasured-flash
FLASH_BUILD := $(BUILD)/unmeasured-flash
UNMEASURED_FLASH ?= 0

OBJS := $(SRC:src/%.c=$(BUILD)/%.o)
FLASH_OBJS := $(SRC:src/%.c=$(FLASH_BUILD)/%.o)

.PHONY: all clean

ifeq ($(UNMEASURED_FLASH),1)
all: $(FLASH_TARGET).prg
else
all: $(TARGET).prg
endif

$(BUILD) $(FLASH_BUILD):
	mkdir -p $@

$(OBJS): $(BUILD)/%.o: src/%.c $(HEADERS) | $(BUILD)
	cl65 $(CFLAGS) -c -o $@ $<

$(FLASH_OBJS): $(FLASH_BUILD)/%.o: src/%.c $(HEADERS) | $(FLASH_BUILD)
	cl65 $(CFLAGS) -DCF_UNMEASURED_FLASH -c -o $@ $<

# Link $(2) into $@ with map $(1), then check the program ends below the screens.
define link
	cl65 -t c64 -m $(1) -o $@ $(2)
	@run=$$(awk '{ for (i = 1; i < NF; i++) if ($$i == "__BSS_RUN__") print $$(i + 1) }' $(1) | head -n 1); \
	  size=$$(awk '{ for (i = 1; i < NF; i++) if ($$i == "__BSS_SIZE__") print $$(i + 1) }' $(1) | head -n 1); \
	  end=$$((0x$$run + 0x$$size)); \
	  if [ "$$end" -le 2049 ] || [ "$$end" -gt $$(($(FIELD_SCREENS))) ]; then \
	    echo "program end $$end overlaps colour-field screens at $(FIELD_SCREENS)" >&2; rm -f $@; exit 1; fi
endef

$(TARGET).prg: $(OBJS)
	$(call link,$(BUILD)/$(TARGET).map,$(OBJS))

$(FLASH_TARGET).prg: $(FLASH_OBJS)
	$(call link,$(FLASH_BUILD)/$(FLASH_TARGET).map,$(FLASH_OBJS))

clean:
	rm -rf $(BUILD) $(TARGET).prg $(FLASH_TARGET).prg
