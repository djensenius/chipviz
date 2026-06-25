BUILD_DIR = build
ROM = chipviz-gb.gb

all: $(ROM)
.PHONY: all clean

$(BUILD_DIR)/main.o: main.asm
	@mkdir -p $(BUILD_DIR)
	rgbasm -o $@ $<

$(ROM): $(BUILD_DIR)/main.o
	rgblink -o $@ $<
	rgbfix -v -p 0 -t CHIPVIZGB $@

clean:
	rm -rf $(BUILD_DIR) $(ROM)
