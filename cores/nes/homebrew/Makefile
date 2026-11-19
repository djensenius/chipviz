BUILD_DIR = build
# INPUT_ROLE=operator builds the operator role (colour-field-v0 input roles);
# INPUT_ROLE=test builds a data-role ROM for the emulator tests.
INPUT_ROLE ?= data

ifeq ($(INPUT_ROLE),operator)
ROM = chipviz-nes-operator.nes
OBJ_DIR = $(BUILD_DIR)/operator
ASFLAGS = -g -D CHIPVIZ_INPUT_ROLE_OPERATOR
else ifeq ($(INPUT_ROLE),data)
ROM = chipviz-nes.nes
OBJ_DIR = $(BUILD_DIR)/data
ASFLAGS = -g
else ifeq ($(INPUT_ROLE),test)
# Data role plus a test-only input: Select zeroes the pad energy (silence).
ROM = chipviz-nes-test.nes
OBJ_DIR = $(BUILD_DIR)/test
ASFLAGS = -g -D CHIPVIZ_TEST_SILENCE
else
$(error INPUT_ROLE must be data, operator or test)
endif

all: $(ROM)
.PHONY: all clean

$(OBJ_DIR)/main.o: src/main.s
	@mkdir -p $(OBJ_DIR)
	ca65 $(ASFLAGS) $< -o $@

# The label file lets emulator tests read the colour-field trace from RAM.
$(ROM): $(OBJ_DIR)/main.o nes.cfg
	ld65 -C nes.cfg -Ln $(OBJ_DIR)/labels.txt -o $@ $<

clean:
	rm -rf $(BUILD_DIR) chipviz-nes.nes chipviz-nes-operator.nes chipviz-nes-test.nes
