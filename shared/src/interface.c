#include "chipviz/interface.h"

static uint8_t normalized_scene_count(uint8_t scene_count) {
  return scene_count == 0u ? CHIPVIZ_INTERFACE_DEFAULT_SCENE_COUNT : scene_count;
}

void chipviz_interface_init(ChipvizInterfaceState *state, uint8_t scene_count) {
  if (state == 0) {
    return;
  }

  state->screen = CHIPVIZ_INTERFACE_SCREEN_TITLE;
  state->mode = CHIPVIZ_INTERFACE_MODE_MANUAL;
  state->selected_scene = 0;
  state->scene_count = normalized_scene_count(scene_count);
  state->ticks = 0;
  state->auto_period = 240;
}

void chipviz_interface_apply_input(ChipvizInterfaceState *state, uint8_t input_flags) {
  if (state == 0) {
    return;
  }

  if ((input_flags & CHIPVIZ_INTERFACE_INPUT_START) != 0u) {
    if (state->screen == CHIPVIZ_INTERFACE_SCREEN_TITLE) {
      state->screen = CHIPVIZ_INTERFACE_SCREEN_SCENE_SELECT;
    } else if (state->screen == CHIPVIZ_INTERFACE_SCREEN_SCENE_SELECT) {
      state->screen = CHIPVIZ_INTERFACE_SCREEN_VISUALIZER;
    } else {
      state->screen = CHIPVIZ_INTERFACE_SCREEN_SCENE_SELECT;
    }
  }

  if ((input_flags & CHIPVIZ_INTERFACE_INPUT_NEXT_SCENE) != 0u) {
    state->selected_scene = (uint8_t)((state->selected_scene + 1u) % state->scene_count);
    state->screen = CHIPVIZ_INTERFACE_SCREEN_SCENE_SELECT;
  }

  if ((input_flags & CHIPVIZ_INTERFACE_INPUT_PREV_SCENE) != 0u) {
    state->selected_scene = (uint8_t)((state->selected_scene + state->scene_count - 1u) % state->scene_count);
    state->screen = CHIPVIZ_INTERFACE_SCREEN_SCENE_SELECT;
  }

  if ((input_flags & CHIPVIZ_INTERFACE_INPUT_AUTO) != 0u) {
    state->mode = CHIPVIZ_INTERFACE_MODE_AUTO;
    state->screen = CHIPVIZ_INTERFACE_SCREEN_VISUALIZER;
  }

  if ((input_flags & CHIPVIZ_INTERFACE_INPUT_DEMO) != 0u) {
    state->mode = CHIPVIZ_INTERFACE_MODE_DEMO;
    state->screen = CHIPVIZ_INTERFACE_SCREEN_VISUALIZER;
  }

  if ((input_flags & CHIPVIZ_INTERFACE_INPUT_MANUAL) != 0u) {
    state->mode = CHIPVIZ_INTERFACE_MODE_MANUAL;
    state->screen = CHIPVIZ_INTERFACE_SCREEN_VISUALIZER;
  }
}

void chipviz_interface_tick(ChipvizInterfaceState *state) {
  if (state == 0) {
    return;
  }

  state->ticks++;
  if (state->mode == CHIPVIZ_INTERFACE_MODE_AUTO && state->auto_period != 0u &&
      (state->ticks % state->auto_period) == 0u) {
    state->selected_scene = (uint8_t)((state->selected_scene + 1u) % state->scene_count);
  }
}

void chipviz_interface_apply_to_frame(
    const ChipvizInterfaceState *state,
    ChipvizFrame *frame) {
  if (state == 0 || frame == 0) {
    return;
  }

  frame->scene = state->selected_scene;
}

void chipviz_interface_make_demo_frame(
    const ChipvizInterfaceState *state,
    ChipvizFrame *frame) {
  if (state == 0 || frame == 0) {
    return;
  }

  chipviz_frame_make_procedural(state->ticks, frame);
  chipviz_interface_apply_to_frame(state, frame);
}

uint8_t chipviz_interface_mode_code(const ChipvizInterfaceState *state) {
  if (state == 0) {
    return 0;
  }
  return (uint8_t)state->mode;
}
