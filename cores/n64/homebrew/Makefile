all: chipviz-n64.z64
.PHONY: all clean

BUILD_DIR = build
include $(N64_INST)/include/n64.mk

OBJS = $(BUILD_DIR)/main.o

chipviz-n64.z64: N64_ROM_TITLE = "chipviz"

$(BUILD_DIR)/chipviz-n64.elf: $(OBJS)

clean:
	rm -rf $(BUILD_DIR) *.z64
.PHONY: clean

-include $(wildcard $(BUILD_DIR)/*.d)
