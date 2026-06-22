#include "chipviz/interface.h"

#include <stdio.h>
#include <stdlib.h>

static void require(int condition, const char *message) {
  if (!condition) {
    fprintf(stderr, "FAIL: %s\n", message);
    exit(1);
  }
}

static void title_and_scene_selection_flow(void) {
  ChipvizInterfaceState state;

  chipviz_interface_init(&state, 4);
  require(state.screen == CHIPVIZ_INTERFACE_SCREEN_TITLE, "starts on title");
  require(state.mode == CHIPVIZ_INTERFACE_MODE_MANUAL, "starts manual");
  require(state.scene_count == 4, "scene count set");

  chipviz_interface_apply_input(&state, CHIPVIZ_INTERFACE_INPUT_START);
  require(state.screen == CHIPVIZ_INTERFACE_SCREEN_SCENE_SELECT, "start opens scene select");

  chipviz_interface_apply_input(&state, CHIPVIZ_INTERFACE_INPUT_NEXT_SCENE);
  require(state.selected_scene == 1, "next scene");
  require(state.screen == CHIPVIZ_INTERFACE_SCREEN_SCENE_SELECT, "scene select stays visible");

  chipviz_interface_apply_input(&state, CHIPVIZ_INTERFACE_INPUT_PREV_SCENE);
  require(state.selected_scene == 0, "previous scene");

  chipviz_interface_apply_input(&state, CHIPVIZ_INTERFACE_INPUT_START);
  require(state.screen == CHIPVIZ_INTERFACE_SCREEN_VISUALIZER, "start opens visualizer");
}

static void auto_mode_advances_scene(void) {
  ChipvizInterfaceState state;

  chipviz_interface_init(&state, 3);
  state.auto_period = 2;
  chipviz_interface_apply_input(&state, CHIPVIZ_INTERFACE_INPUT_AUTO);
  require(state.mode == CHIPVIZ_INTERFACE_MODE_AUTO, "auto mode selected");
  require(state.screen == CHIPVIZ_INTERFACE_SCREEN_VISUALIZER, "auto jumps to visualizer");

  chipviz_interface_tick(&state);
  require(state.selected_scene == 0, "scene unchanged before period");
  chipviz_interface_tick(&state);
  require(state.selected_scene == 1, "scene advances on period");
}

static void demo_mode_generates_visualizer_input(void) {
  ChipvizInterfaceState state;
  ChipvizFrame frame;

  chipviz_interface_init(&state, 6);
  chipviz_interface_apply_input(&state, CHIPVIZ_INTERFACE_INPUT_DEMO);
  state.selected_scene = 5;
  state.ticks = 16;
  chipviz_interface_make_demo_frame(&state, &frame);

  require(state.mode == CHIPVIZ_INTERFACE_MODE_DEMO, "demo mode selected");
  require(frame.scene == 5, "demo applies selected scene");
  require((frame.flags & 0x08u) != 0u, "demo flag set");
  require(frame.energy > 0, "demo frame has energy");
}

int main(void) {
  title_and_scene_selection_flow();
  auto_mode_advances_scene();
  demo_mode_generates_visualizer_input();
  printf("interface tests passed\n");
  return 0;
}
