#include <libdragon.h>

static uint32_t color(int r, int g, int b) {
  return graphics_make_color(r, g, b, 255);
}

int main(void) {
  int frame = 0;
  debug_init_isviewer();
  display_init(RESOLUTION_320x240, DEPTH_32_BPP, 2, GAMMA_NONE, ANTIALIAS_RESAMPLE);

  while (1) {
    display_context_t disp = display_lock();
    int i;
    int phase = frame & 255;

    graphics_fill_screen(disp, color(0, 0, 16 + (phase >> 4)));
    graphics_draw_text(disp, 20, 16, "chipviz N64 homebrew");
    graphics_draw_text(disp, 20, 28, "3D plane / spectrum scaffold");

    for (i = 0; i < 8; i++) {
      int energy = (phase + i * 29) & 255;
      int height = 12 + (energy / 4);
      int x = 28 + i * 32;
      int y = 204 - height;
      graphics_draw_box(disp, x + (i & 1) * 4, y - i * 5, 20, height, color(energy, 255 - energy, 96 + i * 16));
      graphics_draw_box(disp, x + 8, y - i * 5 - 8, 20, 6, color(255, 255, 255));
    }

    for (i = 0; i < 10; i++) {
      int z = (phase + i * 23) & 127;
      int x = 160 + ((i * 37 + phase) % 120) - 60;
      int y = 72 + ((i * 19 + phase / 2) % 80);
      int size = 2 + (127 - z) / 18;
      graphics_draw_box(disp, x, y, size, size, color(80 + z, 80 + z, 255));
    }

    graphics_draw_box(disp, 8 + (phase / 2), 220, 40, 4, color(255, 255, 255));
    display_show(disp);
    frame++;
  }
}
