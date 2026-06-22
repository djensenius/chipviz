#ifndef CHIPVIZ_INTERFACE_H
#define CHIPVIZ_INTERFACE_H

#include <stdint.h>

#include "chipviz/control_frame.h"

#define CHIPVIZ_INTERFACE_DEFAULT_SCENE_COUNT 6u

typedef enum {
  CHIPVIZ_INTERFACE_SCREEN_TITLE = 0,
  CHIPVIZ_INTERFACE_SCREEN_SCENE_SELECT,
  CHIPVIZ_INTERFACE_SCREEN_VISUALIZER
} ChipvizInterfaceScreen;

typedef enum {
  CHIPVIZ_INTERFACE_MODE_MANUAL = 0,
  CHIPVIZ_INTERFACE_MODE_AUTO,
  CHIPVIZ_INTERFACE_MODE_DEMO
} ChipvizInterfaceMode;

typedef enum {
  CHIPVIZ_INTERFACE_INPUT_NONE = 0,
  CHIPVIZ_INTERFACE_INPUT_START = 1u << 0,
  CHIPVIZ_INTERFACE_INPUT_NEXT_SCENE = 1u << 1,
  CHIPVIZ_INTERFACE_INPUT_PREV_SCENE = 1u << 2,
  CHIPVIZ_INTERFACE_INPUT_AUTO = 1u << 3,
  CHIPVIZ_INTERFACE_INPUT_DEMO = 1u << 4
} ChipvizInterfaceInput;

typedef struct {
  ChipvizInterfaceScreen screen;
  ChipvizInterfaceMode mode;
  uint8_t selected_scene;
  uint8_t scene_count;
  uint16_t ticks;
  uint16_t auto_period;
} ChipvizInterfaceState;

void chipviz_interface_init(ChipvizInterfaceState *state, uint8_t scene_count);
void chipviz_interface_apply_input(ChipvizInterfaceState *state, uint8_t input_flags);
void chipviz_interface_tick(ChipvizInterfaceState *state);
void chipviz_interface_apply_to_frame(
    const ChipvizInterfaceState *state,
    ChipvizFrame *frame);
void chipviz_interface_make_demo_frame(
    const ChipvizInterfaceState *state,
    ChipvizFrame *frame);

#endif
