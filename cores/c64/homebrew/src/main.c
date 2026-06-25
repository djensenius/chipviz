#include <conio.h>
#include <stdint.h>

#define CHIPVIZ_BORDER_COLOR (*(volatile uint8_t *)0xD020)
#define CHIPVIZ_BACKGROUND_COLOR (*(volatile uint8_t *)0xD021)
#define CHIPVIZ_RASTER_LINE (*(volatile uint8_t *)0xD012)
#define CHIPVIZ_SCREEN_RAM ((volatile uint8_t *)0x0400)
#define CHIPVIZ_COLOR_RAM ((volatile uint8_t *)0xD800)

typedef struct {
  uint8_t spectrum[8];
  uint8_t scene;
  uint8_t palette;
  uint8_t energy;
  uint8_t beat;
  uint8_t note;
} demo_state_t;

static void make_demo_state(uint16_t frame, demo_state_t *state) {
  uint8_t band;
  uint8_t phrase = (uint8_t)((frame / 96u) & 3u);

  state->scene = phrase;
  state->palette = (uint8_t)((frame / 32u) & 7u);
  state->energy = (uint8_t)(80u + ((frame * 5u + phrase * 23u) & 95u));
  state->beat = (uint8_t)((frame & 31u) < 4u);
  state->note = (uint8_t)(48u + ((frame / 16u) & 23u));
  for (band = 0; band < 8u; band++) {
    uint8_t wave = (uint8_t)((frame * (uint16_t)(band + 3u) + band * 29u) & 127u);
    state->spectrum[band] = (uint8_t)(48u + (wave < 64u ? wave : 127u - wave) + phrase * 16u);
  }
}

static void draw_title(void) {
  clrscr();
  textcolor(COLOR_LIGHTGREEN);
  cputsxy(1, 0, "CHIPVIZ C64 SDK DEMO");
  textcolor(COLOR_CYAN);
  cputsxy(1, 1, "CC65 VIC-II / PETSCII");
}

static void seed_petscii_grid(void) {
  uint8_t x;
  uint8_t y;

  for (y = 0; y < 18u; y++) {
    for (x = 0; x < 40u; x++) {
      uint16_t offset = (uint16_t)(y + 4u) * 40u + x;
      CHIPVIZ_SCREEN_RAM[offset] = (uint8_t)(160u + ((x + y) & 15u));
      CHIPVIZ_COLOR_RAM[offset] = (uint8_t)(1u + ((x / 5u + y) & 15u));
    }
  }
}

static void wait_frame(void) {
  while (CHIPVIZ_RASTER_LINE != 0xFFu) {
  }
  while (CHIPVIZ_RASTER_LINE == 0xFFu) {
  }
}

static void update_petscii_rows(uint16_t frame, const demo_state_t *state) {
  uint8_t x;
  uint8_t y;
  uint8_t pass = (uint8_t)(frame & 3u);

  for (y = 0; y < 18u; y++) {
    if ((y & 3u) != pass) {
      continue;
    }
    for (x = 0; x < 40u; x++) {
      uint16_t offset = (uint16_t)(y + 4u) * 40u + x;
      uint8_t band = state->spectrum[(x / 5u) & 7u];
      uint8_t cell = (uint8_t)(160u + ((x + y + frame / 4u + state->scene * 3u) & 15u));
      if (((x * 3u + y * 5u + frame + band) & 31u) < (state->beat ? 8u : 4u)) {
        cell = (uint8_t)(81u + (state->note & 7u));
      }
      CHIPVIZ_SCREEN_RAM[offset] = cell;
      CHIPVIZ_COLOR_RAM[offset] = (uint8_t)(1u + ((band / 16u + state->palette + y) & 15u));
    }
  }
}

static void draw_channel_meters(const demo_state_t *state) {
  uint8_t band;
  uint8_t y;

  for (band = 0; band < 8u; band++) {
    uint8_t height = (uint8_t)(1u + (state->spectrum[band] >> 5));
    for (y = 0; y < 5u; y++) {
      uint16_t offset = (uint16_t)(23u - y) * 40u + 2u + band * 4u;
      CHIPVIZ_SCREEN_RAM[offset] = y < height ? 102u : 32u;
      CHIPVIZ_SCREEN_RAM[offset + 1u] = y < height ? 102u : 32u;
      CHIPVIZ_COLOR_RAM[offset] = (uint8_t)(1u + ((band + state->palette) & 15u));
      CHIPVIZ_COLOR_RAM[offset + 1u] = CHIPVIZ_COLOR_RAM[offset];
    }
  }
}

int main(void) {
  uint16_t frame = 0;
  demo_state_t state;

  draw_title();
  seed_petscii_grid();
  for (;;) {
    wait_frame();
    make_demo_state(frame, &state);
    CHIPVIZ_BORDER_COLOR = (uint8_t)((state.palette + state.beat * 8u) & 15u);
    CHIPVIZ_BACKGROUND_COLOR = (uint8_t)((state.scene * 2u + state.energy / 32u) & 15u);
    update_petscii_rows(frame, &state);
    draw_channel_meters(&state);
    ++frame;
  }
  return 0;
}
