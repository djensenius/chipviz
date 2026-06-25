#include <snes.h>

typedef struct {
  unsigned char spectrum[8];
  unsigned char scene;
  unsigned char palette;
  unsigned char energy;
  unsigned char beat;
} demo_state_t;

static unsigned char solid_tiles[9u * 32u];
static unsigned short tilemap[32u * 32u];

static unsigned short rgb5(unsigned char r, unsigned char g, unsigned char b) {
  return (unsigned short)((r & 31u) | ((g & 31u) << 5) | ((b & 31u) << 10));
}

static void make_demo_state(unsigned short frame, demo_state_t *state) {
  unsigned char band;
  unsigned char phrase = (unsigned char)((frame / 96u) & 3u);

  state->scene = phrase;
  state->palette = (unsigned char)((frame / 32u) & 7u);
  state->energy = (unsigned char)(80u + ((frame * 5u + phrase * 23u) & 95u));
  state->beat = (unsigned char)((frame & 31u) < 4u);
  for (band = 0; band < 8u; band++) {
    unsigned char wave = (unsigned char)((frame * (unsigned short)(band + 3u) + band * 29u) & 127u);
    state->spectrum[band] = (unsigned char)(48u + (wave < 64u ? wave : 127u - wave) + phrase * 16u);
  }
}

static void build_solid_tiles(void) {
  unsigned char tile;
  unsigned char row;

  for (tile = 0; tile < 9u; tile++) {
    for (row = 0; row < 8u; row++) {
      unsigned short offset = (unsigned short)tile * 32u + row * 2u;
      solid_tiles[offset] = (tile & 1u) ? 0xFFu : 0x00u;
      solid_tiles[offset + 1u] = (tile & 2u) ? 0xFFu : 0x00u;
      solid_tiles[offset + 16u] = (tile & 4u) ? 0xFFu : 0x00u;
      solid_tiles[offset + 17u] = (tile & 8u) ? 0xFFu : 0x00u;
    }
  }
}

static void update_tilemap(unsigned short frame, const demo_state_t *state) {
  unsigned char x;
  unsigned char y;

  for (y = 0; y < 28u; y++) {
    for (x = 0; x < 32u; x++) {
      unsigned char band = (unsigned char)((x / 4u) & 7u);
      unsigned char height = (unsigned char)(2u + (state->spectrum[band] >> 4));
      unsigned char tile = 0;
      if ((27u - y) < height) {
        tile = (unsigned char)(1u + band);
      } else if (((x + y + frame / 8u + state->scene * 3u) & 15u) < (state->beat ? 4u : 2u)) {
        tile = (unsigned char)(1u + ((band + state->palette) & 7u));
      }
      tilemap[(unsigned short)y * 32u + x] = tile;
    }
  }

  for (y = 28u; y < 32u; y++) {
    for (x = 0; x < 32u; x++) {
      tilemap[(unsigned short)y * 32u + x] = (unsigned short)(1u + ((x + frame / 4u) & 7u));
    }
  }
}

int main(void) {
  unsigned short frame = 0;
  demo_state_t state;

  build_solid_tiles();
  bgSetGfxPtr(0, 0x2000);
  bgSetMapPtr(0, 0x6800, SC_32x32);
  dmaCopyVram((unsigned char *)solid_tiles, 0x2000, sizeof(solid_tiles));
  setMode(BG_MODE1, 0);
  bgSetDisable(1);
  bgSetDisable(2);
  setScreenOn();

  while (1) {
    unsigned char band;
    unsigned short bg;

    make_demo_state(frame, &state);
    bg = rgb5(
        (unsigned char)((state.energy >> 3) + state.beat * 8u),
        (unsigned char)((state.palette * 3u + state.scene * 4u) & 31u),
        (unsigned char)((frame >> 3) & 31u));
    setPaletteColor(0x00, bg);

    for (band = 0; band < 8u; band++) {
      unsigned char level = (unsigned char)(state.spectrum[band] >> 3);
      setPaletteColor(
          (unsigned char)(1u + band),
          rgb5(
              (unsigned char)((level + band * 2u) & 31u),
              (unsigned char)((state.palette * 4u + level) & 31u),
              (unsigned char)(31u - ((level + state.scene * 3u) & 31u))));
    }

    update_tilemap(frame, &state);
    WaitForVBlank();
    dmaCopyVram((unsigned char *)tilemap, 0x6800, sizeof(tilemap));
    ++frame;
  }
  return 0;
}
