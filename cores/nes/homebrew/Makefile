BUILD_DIR = build
# INPUT_ROLE=operator builds the operator role (colour-field-v0 input roles).
INPUT_ROLE ?= data

ifeq ($(INPUT_ROLE),operator)
ROM = chipviz-nes-operator.nes
OBJ_DIR = $(BUILD_DIR)/operator
ASFLAGS = -g -D CHIPVIZ_INPUT_ROLE_OPERATOR
else ifeq ($(INPUT_ROLE),data)
ROM = chipviz-nes.nes
OBJ_DIR = $(BUILD_DIR)/data
ASFLAGS = -g
else
$(error INPUT_ROLE must be data or operator)
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
	rm -rf $(BUILD_DIR) chipviz-nes.nes chipviz-nes-operator.nes
