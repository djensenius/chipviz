TARGET := chipviz-c64
BUILD := build
SRC := src/main.c

.PHONY: all clean

all: $(TARGET).prg

$(BUILD):
	mkdir -p $@

$(TARGET).prg: $(SRC) | $(BUILD)
	cl65 -t c64 -Oirs --static-locals -o $@ $<

clean:
	rm -rf $(BUILD) $(TARGET).prg
