TARGET := chipviz-gba
BUILD := build
SOURCES := source
INCLUDES := include ../../../shared/include

ifeq ($(strip $(DEVKITARM)),)
$(error DEVKITARM is not set. Install devkitPro/devkitARM and source devkit-env.sh)
endif

include $(DEVKITARM)/gba_rules

ARCH := -mthumb -mthumb-interwork
CFLAGS := -g -Wall -O2 -mcpu=arm7tdmi -mtune=arm7tdmi $(ARCH) $(addprefix -I,$(INCLUDES))
LDFLAGS := -g $(ARCH) -Wl,-Map,$(notdir $*.map)
LIBS :=

CFILES := $(foreach dir,$(SOURCES),$(notdir $(wildcard $(dir)/*.c)))
OFILES := $(CFILES:.c=.o)
VPATH := $(foreach dir,$(SOURCES),$(CURDIR)/$(dir))

.PHONY: all clean

all: $(BUILD) $(TARGET).gba

$(BUILD):
	mkdir -p $@

$(TARGET).gba: $(BUILD)/$(TARGET).elf
	$(OBJCOPY) -O binary $< $@
	gbafix $@

$(BUILD)/$(TARGET).elf: $(addprefix $(BUILD)/,$(OFILES))
	$(CC) $(LDFLAGS) $^ $(LIBS) -o $@

$(BUILD)/%.o: %.c | $(BUILD)
	$(CC) $(CFLAGS) -c $< -o $@

clean:
	rm -rf $(BUILD) $(TARGET).gba
