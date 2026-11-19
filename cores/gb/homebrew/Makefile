BUILD_DIR = build
# INPUT_ROLE=operator builds the operator role (colour-field-v0 input roles).
# FIXTURE=1 builds the emulator test ROM, which steps from a WRAM mailbox.
INPUT_ROLE ?= data
FIXTURE ?= 0

ifeq ($(FIXTURE),1)
ROM = chipviz-gb-fixture.gb
OBJ_DIR = $(BUILD_DIR)/fixture
ASFLAGS = -D CHIPVIZ_FIXTURE
else ifeq ($(INPUT_ROLE),operator)
ROM = chipviz-gb-operator.gb
OBJ_DIR = $(BUILD_DIR)/operator
ASFLAGS = -D CHIPVIZ_INPUT_ROLE_OPERATOR
else ifeq ($(INPUT_ROLE),data)
ROM = chipviz-gb.gb
OBJ_DIR = $(BUILD_DIR)/data
ASFLAGS =
else
$(error INPUT_ROLE must be data or operator)
endif

all: $(ROM)
.PHONY: all clean

$(OBJ_DIR)/main.o: main.asm colour_field.asm
	@mkdir -p $(OBJ_DIR)
	rgbasm $(ASFLAGS) -o $@ main.asm

$(ROM): $(OBJ_DIR)/main.o
	rgblink -n $(OBJ_DIR)/chipviz-gb.sym -o $@ $<
	rgbfix -v -p 0 -t CHIPVIZGB $@

clean:
	rm -rf $(BUILD_DIR) chipviz-gb.gb chipviz-gb-operator.gb chipviz-gb-fixture.gb
