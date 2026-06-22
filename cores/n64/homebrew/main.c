#include <libdragon.h>

typedef struct {
  int bands[8];
  int beat;
  int scene;
  int palette;
  int energy;
  int midi;
} visual_state_t;

static uint32_t color(int r, int g, int b) {
  return graphics_make_color(r, g, b, 255);
}

static void decode_transport_packet(const unsigned char packet[16], visual_state_t *state) {
  int i;

  state->bands[0] = packet[2];
  state->bands[1] = packet[3];
  state->bands[2] = packet[6];
  state->bands[3] = packet[7];
  state->bands[4] = packet[10];
  state->bands[5] = packet[11];
  state->bands[6] = (packet[14] + packet[15]) & 255;
  state->bands[7] = (packet[14] * 2 + packet[15]) & 255;
  state->beat = (packet[0] | packet[4] | packet[8] | packet[12]) & 3;
  state->scene = ((packet[12] >> 4) | (packet[13] >> 2)) & 7;
  state->palette = ((packet[12] >> 6) | packet[13]) & 7;
  state->energy = packet[14];
  state->midi = packet[15];
  for (i = 0; i < 8; i++) {
    if (state->bands[i] < 20) {
      state->bands[i] = 20;
    }
  }
}

static void procedural_packet(int frame, unsigned char packet[16]) {
  int i;

  for (i = 0; i < 16; i++) {
    packet[i] = 0;
  }
  for (i = 0; i < 6; i++) {
    int value = (frame * (5 + i) + i * 37) & 255;
    packet[2 + (i / 2) * 4 + (i & 1)] = (unsigned char)value;
  }
  packet[0] = (frame & 31) < 3 ? 3 : 0;
  packet[4] = (frame & 63) < 6 ? 2 : 0;
  packet[8] = (frame & 127) < 12 ? 1 : 0;
  packet[12] = (unsigned char)(((frame / 96) & 3) << 4);
  packet[13] = (unsigned char)((frame / 32) & 7);
  packet[14] = (unsigned char)(80 + ((frame * 3) & 127));
  packet[15] = (unsigned char)((frame * 11) & 255);
}

static void draw_plane_grid(display_context_t disp, int frame, const visual_state_t *state) {
  int i;
  int horizon = 68 + state->scene * 3;
  int center = 160 + ((state->midi - 128) / 12);

  for (i = 0; i < 9; i++) {
    int x = center + (i - 4) * (18 + (frame & 7));
    graphics_draw_line(disp, center, horizon, x, 224, color(32 + state->energy / 4, 96, 180 + state->palette * 8));
  }
  for (i = 0; i < 8; i++) {
    int y = horizon + i * i * 3;
    graphics_draw_line(disp, 20, y, 300, y + (state->scene - 3), color(40, 40 + i * 20, 120 + state->palette * 12));
  }
}

int main(void) {
  int frame = 0;
  debug_init_isviewer();
  display_init(RESOLUTION_320x240, DEPTH_32_BPP, 2, GAMMA_NONE, ANTIALIAS_RESAMPLE);

  while (1) {
    display_context_t disp = display_lock();
    int i;
    int phase = frame & 255;
    unsigned char packet[16];
    visual_state_t state;

    procedural_packet(frame, packet);
    decode_transport_packet(packet, &state);

    graphics_fill_screen(disp, color(0, 0, 12 + state.palette * 6 + (phase >> 5)));
    graphics_draw_text(disp, 20, 16, "chipviz N64 homebrew");
    graphics_draw_text(disp, 20, 28, "Joybus packet -> 3D planes");
    draw_plane_grid(disp, frame, &state);

    for (i = 0; i < 8; i++) {
      int energy = state.bands[i];
      int height = 12 + (energy / 4);
      int depth = i * 6 + ((phase >> 3) & 7);
      int x = 28 + i * 32 + (state.scene - 3) * i;
      int y = 206 - height - depth;
      graphics_draw_box(disp, x + (i & 1) * 4, y, 18 + i, height, color(energy, 255 - energy, 80 + state.palette * 18));
      graphics_draw_box(disp, x + 8, y - 8, 20, 6, color(220 + state.beat * 16, 220, 255));
    }

    for (i = 0; i < 18; i++) {
      int z = (phase + i * 23 + state.energy) & 127;
      int x = 160 + ((i * 37 + phase + state.midi) % 160) - 80;
      int y = 58 + ((i * 19 + phase / 2 + state.scene * 9) % 96);
      int size = 2 + (127 - z) / 18;
      graphics_draw_box(disp, x, y, size, size, color(80 + z, 80 + z + state.beat * 20, 255));
    }

    graphics_draw_box(disp, 8 + (phase / 2), 220, 20 + state.energy / 3, 4, color(255, 255, 255));
    display_show(disp);
    frame++;
  }
}
