#include <libdragon.h>

typedef struct {
  int bands[8];
  int beat;
  int scene;
  int palette;
  int energy;
  int bass;
  int midi;
} visual_state_t;

static uint32_t color(int r, int g, int b) {
  if (r < 0) {
    r = 0;
  } else if (r > 255) {
    r = 255;
  }
  if (g < 0) {
    g = 0;
  } else if (g > 255) {
    g = 255;
  }
  if (b < 0) {
    b = 0;
  } else if (b > 255) {
    b = 255;
  }
  return graphics_make_color(r, g, b, 255);
}

static void decode_transport_packet(const unsigned char packet[16], visual_state_t *state) {
  int i;
  int global_beat;
  int band_beats;

  state->bands[0] = packet[2];
  state->bands[1] = packet[3];
  state->bands[2] = packet[6];
  state->bands[3] = packet[7];
  state->bands[4] = packet[10];
  state->bands[5] = packet[11];
  state->bands[6] = (packet[14] + packet[15]) & 255;
  state->bands[7] = (packet[14] * 2 + packet[15]) & 255;
  global_beat = packet[12] & 0x01;
  band_beats = (packet[0] & 0x03) | (packet[4] & 0x03) | (packet[8] & 0x03);
  state->beat = global_beat ? 3 : band_beats;
  state->scene = ((packet[12] >> 4) & 0x03) | (((packet[13] >> 4) & 0x03) << 2);
  state->palette = packet[13] & 0x0F;
  state->energy = packet[14];
  state->bass = state->bands[0];
  state->midi = packet[15];
  for (i = 0; i < 8; i++) {
    if (state->bands[i] < 20) {
      state->bands[i] = 20;
    }
  }
}

static int read_joybus_packet(unsigned char packet[16]) {
  struct controller_data output;
  int p;
  int has_data = 0;

  controller_scan();
  controller_read(&output);
  for (p = 0; p < 4; p++) {
    unsigned int data = output.c[p].data;
    packet[p * 4 + 0] = (unsigned char)((data >> 24) & 0xFF);
    packet[p * 4 + 1] = (unsigned char)((data >> 16) & 0xFF);
    packet[p * 4 + 2] = (unsigned char)((data >> 8) & 0xFF);
    packet[p * 4 + 3] = (unsigned char)(data & 0xFF);
    if (data != 0) {
      has_data = 1;
    }
  }
  return get_controllers_present() != 0 && has_data;
}

static void procedural_packet(int frame, unsigned char packet[16]) {
  int i;
  int phrase = (frame / 96) & 3;
  int pulse = frame & 255;

  for (i = 0; i < 16; i++) {
    packet[i] = 0;
  }
  for (i = 0; i < 6; i++) {
    int base = 48 + phrase * 23 + i * 31;
    int wave = (frame * (3 + i) + i * i * 17) & 127;
    int value = base + (wave < 64 ? wave : 127 - wave);
    packet[2 + (i / 2) * 4 + (i & 1)] = (unsigned char)value;
  }
  packet[0] = (pulse & 31) < 4 ? 3 : 0;
  packet[4] = (pulse & 63) < 8 ? 2 : 0;
  packet[8] = (pulse & 127) < 16 ? 1 : 0;
  packet[12] = (unsigned char)((phrase << 4) | ((pulse & 31) < 4 ? 1 : 0));
  packet[13] = (unsigned char)(((frame / 32) & 7) | (((frame / 384) & 3) << 4));
  packet[14] = (unsigned char)(96 + ((frame * 5 + phrase * 29) & 95));
  packet[15] = (unsigned char)(80 + ((frame * 7 + phrase * 41) & 127));
}

static void draw_plane_grid(display_context_t disp, int frame, const visual_state_t *state) {
  int i;
  int horizon = 58 + (state->scene & 3) * 6 + ((frame >> 3) & 7);
  int center = 160 + ((state->midi - 128) / 8);

  for (i = 0; i < 13; i++) {
    int spread = 13 + (frame & 15) + (state->energy / 32);
    int x = center + (i - 6) * spread;
    graphics_draw_line(disp, center, horizon, x, 224, color(24 + state->energy / 3, 64 + state->palette * 16, 150 + state->beat * 24));
  }
  for (i = 0; i < 9; i++) {
    int y = horizon + i * i * 3;
    int skew = (state->scene - 3) * i + ((state->midi - 128) / 24);
    graphics_draw_line(disp, 16, y, 304, y + skew, color(40, 40 + i * 18, 110 + state->palette * 14));
  }
}

static void draw_tunnel(display_context_t disp, int frame, const visual_state_t *state) {
  int i;
  int cx = 160 + ((state->midi - 128) / 10);
  int cy = 112 + ((state->beat - 1) * 4);

  for (i = 0; i < 12; i++) {
    int z = ((frame * 2 + i * 19) & 127);
    int radius = 10 + z + state->energy / 12;
    int wobble = ((frame >> 2) + i * 7 + state->palette * 5) & 31;
    int w = radius + wobble;
    int h = radius / 2 + (state->bass / 16);
    graphics_draw_line(disp, cx - w, cy - h, cx + w, cy - h / 2, color(60 + z, 50 + state->palette * 20, 220));
    graphics_draw_line(disp, cx + w, cy - h / 2, cx + w / 2, cy + h, color(80 + z, 220 - state->palette * 10, 170));
    graphics_draw_line(disp, cx + w / 2, cy + h, cx - w, cy - h, color(220, 80 + state->beat * 25, 90 + z));
  }
}

static void draw_voice_particles(display_context_t disp, int frame, const visual_state_t *state) {
  int i;

  for (i = 0; i < 28; i++) {
    int z = (frame + i * 23 + state->energy) & 127;
    int lane = i & 7;
    int x = 160 + ((i * 37 + frame + state->midi + state->bands[lane]) % 200) - 100;
    int y = 48 + ((i * 19 + frame / 2 + state->scene * 11) % 118);
    int size = 2 + (127 - z) / 18 + (state->beat != 0 ? 1 : 0);
    graphics_draw_box(disp, x, y, size, size, color(70 + z, 70 + state->bands[lane] / 2, 255 - lane * 12));
  }
}

static void draw_channel_meters(display_context_t disp, int frame, const visual_state_t *state) {
  int i;

  for (i = 0; i < 8; i++) {
    int energy = state->bands[i];
    int height = 10 + energy / 4;
    int depth = i * 5 + ((frame >> 3) & 9);
    int x = 24 + i * 34 + (state->scene - 3) * i;
    int y = 208 - height - depth;
    int cap = state->beat != 0 ? 10 : 4;
    graphics_draw_box(disp, x + (i & 1) * 4, y + 8, 24, height, color(energy, 255 - energy / 2, 70 + state->palette * 18));
    graphics_draw_box(disp, x + 8, y, 22, cap, color(235, 200 + state->beat * 18, 255));
    graphics_draw_line(disp, x + 24, y + 8, x + 32, y + 2, color(70, 90, 160 + state->palette * 10));
    graphics_draw_line(disp, x + 32, y + 2, x + 32, y + height, color(50, 70, 120 + state->palette * 12));
  }
}

int main(void) {
  int frame = 0;
  debug_init_isviewer();
  controller_init();
  display_init(RESOLUTION_320x240, DEPTH_32_BPP, 2, GAMMA_NONE, ANTIALIAS_RESAMPLE);

  while (1) {
    display_context_t disp = display_lock();
    int i;
    int phase = frame & 255;
    unsigned char packet[16];
    visual_state_t state;

    if (!read_joybus_packet(packet)) {
      procedural_packet(frame, packet);
    }
    decode_transport_packet(packet, &state);

    graphics_fill_screen(disp, color(0, 0, 10 + state.palette * 7 + (phase >> 5)));
    graphics_draw_text(disp, 18, 14, "chipviz N64 demo");
    graphics_draw_text(disp, 18, 26, "baked frames + Joybus live override");
    draw_plane_grid(disp, frame, &state);
    if ((state.scene & 1) != 0) {
      draw_tunnel(disp, frame, &state);
    }
    draw_voice_particles(disp, frame, &state);
    draw_channel_meters(disp, frame, &state);

    graphics_draw_box(disp, 10, 224, 24 + state.energy, 5, color(255, 255, 255));
    for (i = 0; i < 4; i++) {
      int marker = 16 + i * 18;
      graphics_draw_box(disp, 292, marker, 8, 8 + ((phase + i * 17) & 15), color(120 + i * 30, 255 - i * 35, 120 + state.palette * 12));
    }
    display_show(disp);
    frame++;
  }
}
