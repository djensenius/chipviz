TARGET := gba-lines-fixture
BUILD := build

ifeq ($(strip $(DEVKITARM)),)
$(error DEVKITARM is not set. Install devkitPro/devkitARM and source devkit-env.sh)
endif

include $(DEVKITARM)/base_rules

ARCH := -mthumb -mthumb-interwork
CFLAGS := -std=c99 -Wall -Wextra -Werror -O2 -mcpu=arm7tdmi -mtune=arm7tdmi $(ARCH)
LDFLAGS := $(ARCH) -specs=gba.specs -Wl,-Map,$(BUILD)/$(TARGET).map
VPATH := . ..
OBJECTS := $(BUILD)/main.o $(BUILD)/fixture.o $(BUILD)/lines.o

.PHONY: all
all: $(BUILD)/$(TARGET).gba

$(BUILD):
	mkdir -p $@

$(BUILD)/main.o: main.c fixture.h ../lines.h | $(BUILD)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/fixture.o: fixture.c fixture.h ../lines.h | $(BUILD)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/lines.o: ../lines.c ../lines.h | $(BUILD)
	$(CC) $(CFLAGS) -c $< -o $@

$(BUILD)/$(TARGET).elf: $(OBJECTS)
	$(CC) $(LDFLAGS) $^ -o $@

$(BUILD)/$(TARGET).gba: $(BUILD)/$(TARGET).elf
	$(OBJCOPY) -O binary $< $@
	gbafix $@
