#include <libdragon.h>
#include <stdbool.h>
#include <stdint.h>

typedef struct {
  int bands[8];
  int beat;
  int scene;
  int palette;
  int energy;
  int bass;
  int midi;
} visual_state_t;

static color_t rgba(int r, int g, int b) {
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
  return RGBA32(r, g, b, 255);
}

static void fill_rect(float x0, float y0, float x1, float y1, color_t color) {
  if (x0 < 0.0f) {
    x0 = 0.0f;
  }
  if (y0 < 0.0f) {
    y0 = 0.0f;
  }
  if (x1 > 320.0f) {
    x1 = 320.0f;
  }
  if (y1 > 240.0f) {
    y1 = 240.0f;
  }
  if (x1 <= x0 || y1 <= y0) {
    return;
  }
  rdpq_set_mode_fill(color);
  rdpq_fill_rectangle(x0, y0, x1, y1);
}

static void decode_transport_packet(const uint8_t packet[16], visual_state_t *state) {
  state->bands[0] = packet[2];
  state->bands[1] = packet[3];
  state->bands[2] = packet[6];
  state->bands[3] = packet[7];
  state->bands[4] = packet[10];
  state->bands[5] = packet[11];
  state->bands[6] = (packet[14] + packet[15]) & 255;
  state->bands[7] = (packet[14] * 2 + packet[15]) & 255;
  state->beat = (packet[12] & 0x01) ? 3 : ((packet[0] & 0x03) | (packet[4] & 0x03) | (packet[8] & 0x03));
  state->scene = ((packet[12] >> 4) & 0x03) | (((packet[13] >> 4) & 0x03) << 2);
  state->palette = packet[13] & 0x0F;
  state->energy = packet[14];
  state->bass = state->bands[0];
  state->midi = packet[15];
  for (int i = 0; i < 8; i++) {
    if (state->bands[i] < 20) {
      state->bands[i] = 20;
    }
  }
}

static bool read_joybus_packet(uint8_t packet[16]) {
  bool has_data = false;

  joypad_poll();
  for (int p = 0; p < 4; p++) {
    joypad_inputs_t inputs = joypad_get_inputs((joypad_port_t)p);
    uint8_t buttons_a = 0;
    uint8_t buttons_b = 0;

    buttons_a |= inputs.btn.a ? 0x80 : 0;
    buttons_a |= inputs.btn.b ? 0x40 : 0;
    buttons_a |= inputs.btn.z ? 0x20 : 0;
    buttons_a |= inputs.btn.start ? 0x10 : 0;
    buttons_a |= inputs.btn.d_up ? 0x08 : 0;
    buttons_a |= inputs.btn.d_down ? 0x04 : 0;
    buttons_a |= inputs.btn.d_left ? 0x02 : 0;
    buttons_a |= inputs.btn.d_right ? 0x01 : 0;
    if (p == 3) {
      buttons_b |= inputs.btn.l ? 0x20 : 0;
      buttons_b |= inputs.btn.r ? 0x10 : 0;
      buttons_b |= inputs.btn.c_up ? 0x08 : 0;
      buttons_b |= inputs.btn.c_down ? 0x04 : 0;
      buttons_b |= inputs.btn.c_left ? 0x02 : 0;
      buttons_b |= inputs.btn.c_right ? 0x01 : 0;
    }

    packet[p * 4 + 0] = buttons_a;
    packet[p * 4 + 1] = buttons_b;
    packet[p * 4 + 2] = (uint8_t)inputs.stick_x;
    packet[p * 4 + 3] = (uint8_t)inputs.stick_y;
    if (buttons_a != 0 || buttons_b != 0 || inputs.stick_x != 0 || inputs.stick_y != 0) {
      has_data = true;
    }
  }
  return has_data;
}

static void procedural_packet(int frame, uint8_t packet[16]) {
  int phrase = (frame / 96) & 3;
  int pulse = frame & 255;

  for (int i = 0; i < 16; i++) {
    packet[i] = 0;
  }
  for (int i = 0; i < 6; i++) {
    int base = 48 + phrase * 23 + i * 31;
    int wave = (frame * (3 + i) + i * i * 17) & 127;
    int value = base + (wave < 64 ? wave : 127 - wave);
    packet[2 + (i / 2) * 4 + (i & 1)] = (uint8_t)value;
  }
  packet[0] = (pulse & 31) < 4 ? 3 : 0;
  packet[4] = (pulse & 63) < 8 ? 2 : 0;
  packet[8] = (pulse & 127) < 16 ? 1 : 0;
  packet[12] = (uint8_t)((phrase << 4) | ((pulse & 31) < 4 ? 1 : 0));
  packet[13] = (uint8_t)(((frame / 32) & 7) | (((frame / 384) & 3) << 4));
  packet[14] = (uint8_t)(96 + ((frame * 5 + phrase * 29) & 95));
  packet[15] = (uint8_t)(80 + ((frame * 7 + phrase * 41) & 127));
}

static void draw_perspective_grid(int frame, const visual_state_t *state) {
  int horizon = 52 + (state->scene & 3) * 6;
  int center = 160 + ((state->midi - 128) / 9);
  int flash = state->beat ? 36 : 0;

  fill_rect(0.0f, 0.0f, 320.0f, (float)horizon, rgba(8 + state->palette * 8, 18 + flash, 58 + state->scene * 18));
  fill_rect(0.0f, (float)horizon, 320.0f, 240.0f, rgba(3, 5 + state->scene * 8, 24 + state->palette * 6));

  for (int row = 0; row < 12; row++) {
    int y = horizon + row * row + ((frame >> 2) & 7);
    int thickness = 1 + row / 4;
    fill_rect(0.0f, (float)y, 320.0f, (float)(y + thickness), rgba(16 + row * 10, 72 + state->palette * 12, 116 + row * 8));
  }

  for (int lane = -6; lane <= 6; lane++) {
    for (int seg = 0; seg < 9; seg++) {
      int y0 = horizon + seg * seg * 3;
      int y1 = horizon + (seg + 1) * (seg + 1) * 3;
      int spread0 = 3 + seg * (11 + state->energy / 32);
      int spread1 = 3 + (seg + 1) * (11 + state->energy / 32);
      int x0 = center + lane * spread0;
      int x1 = center + lane * spread1;
      int min_x = x0 < x1 ? x0 : x1;
      int max_x = x0 > x1 ? x0 : x1;
      fill_rect((float)min_x, (float)y0, (float)(max_x + 2), (float)(y1 + 1), rgba(28 + state->energy / 4, 80 + seg * 10, 174 + state->beat * 18));
    }
  }
}

static void draw_channel_meters(int frame, const visual_state_t *state) {
  for (int i = 0; i < 8; i++) {
    int energy = state->bands[i];
    int height = 12 + energy / 3;
    int depth = i * 5 + ((frame >> 3) & 9);
    int x = 22 + i * 35 + (state->scene - 3) * i;
    int y = 210 - height - depth;
    color_t front = rgba(energy, 255 - energy / 2, 72 + state->palette * 16);
    color_t side = rgba(36 + energy / 3, 70 + state->palette * 11, 150 + i * 8);
    color_t cap = rgba(235, 200 + state->beat * 18, 255);

    fill_rect((float)(x + 8), (float)(y - 6), (float)(x + 34), (float)y, cap);
    fill_rect((float)x, (float)y, (float)(x + 26), (float)(y + height), front);
    fill_rect((float)(x + 26), (float)(y + 6), (float)(x + 34), (float)(y + height + 8), side);
    fill_rect((float)(x + 3), (float)(y + height - 4), (float)(x + 23), (float)(y + height - 1), rgba(255, 255, 255));
  }
}

static void draw_voice_particles(int frame, const visual_state_t *state) {
  for (int i = 0; i < 28; i++) {
    int lane = i & 7;
    int z = (frame + i * 23 + state->energy) & 127;
    int depth = 128 - z;
    int x = 160 + ((i * 37 + frame + state->midi + state->bands[lane]) % 210) - 105;
    int y = 44 + ((i * 19 + frame / 2 + state->scene * 11) % 118);
    int size = 2 + depth / 19 + (state->beat != 0 ? 1 : 0);
    fill_rect((float)x, (float)y, (float)(x + size), (float)(y + size), rgba(80 + z, 80 + state->bands[lane] / 2, 255 - lane * 12));
  }
}

static void draw_status_strips(int frame, const visual_state_t *state) {
  fill_rect(10.0f, 10.0f, 142.0f, 13.0f, rgba(255, 255, 255));
  fill_rect(10.0f, 17.0f, (float)(34 + state->energy), 22.0f, rgba(90 + state->bass, 230, 160));
  for (int i = 0; i < 4; i++) {
    int marker = 18 + i * 18;
    fill_rect(292.0f, (float)marker, 300.0f, (float)(marker + 8 + ((frame + i * 17) & 15)), rgba(120 + i * 30, 255 - i * 35, 120 + state->palette * 12));
  }
  if (state->beat != 0) {
    fill_rect(0.0f, 0.0f, 320.0f, 5.0f, rgba(255, 255, 255));
    fill_rect(0.0f, 235.0f, 320.0f, 240.0f, rgba(255, 255, 255));
  }
}

int main(void) {
  int frame = 0;

  debug_init_emulog();
  debug_init_usblog();
  display_init(RESOLUTION_320x240, DEPTH_16_BPP, 3, GAMMA_NONE, FILTERS_RESAMPLE);
  joypad_init();
  timer_init();
  rdpq_init();

  while (1) {
    uint8_t packet[16];
    visual_state_t state;
    surface_t *disp = display_get();

    if (frame < 30 || !read_joybus_packet(packet)) {
      procedural_packet(frame, packet);
    }
    decode_transport_packet(packet, &state);

    rdpq_attach_clear(disp, NULL);
    draw_perspective_grid(frame, &state);
    draw_voice_particles(frame, &state);
    draw_channel_meters(frame, &state);
    draw_status_strips(frame, &state);
    rdpq_detach_show();

    frame++;
  }
}
