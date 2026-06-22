#include <libdragon.h>

int main(void) {
  console_init();
  debug_init_isviewer();
  joypad_init();

  console_set_render_mode(RENDER_MANUAL);
  console_clear();
  printf("chipviz N64 homebrew\n\n");
  printf("Controller ports are the live data bus.\n");
  printf("Baked frames drive the same scene model.\n");
  printf("Pi -> Pico -> Joybus comes next.\n");
  console_render();

  while (1) {
    joypad_poll();
  }
}
