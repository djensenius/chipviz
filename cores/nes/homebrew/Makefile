BUILD_DIR = build
ROM = chipviz-nes.nes

all: $(ROM)
.PHONY: all clean

$(BUILD_DIR)/main.o: src/main.s
	@mkdir -p $(BUILD_DIR)
	ca65 $< -o $@

$(ROM): $(BUILD_DIR)/main.o nes.cfg
	ld65 -C nes.cfg -o $@ $<

clean:
	rm -rf $(BUILD_DIR) $(ROM)
