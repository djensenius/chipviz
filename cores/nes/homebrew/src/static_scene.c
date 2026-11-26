#include "static_scene.h"

/* Every nonblank tile has eight set bits in its first plane and a zero
   second plane. */
#define TILE(a,b,c,d,e,f,g,h) a,b,c,d,e,f,g,h, 0,0,0,0,0,0,0,0
const unsigned char nes_static_chr[17 * 16] = {
  TILE(0x80,0x04,0x20,0x01,0x08,0x40,0x02,0x10),
  TILE(0x02,0x40,0x10,0x01,0x80,0x08,0x20,0x04),
  TILE(0x10,0x01,0x40,0x08,0x02,0x20,0x04,0x80),
  TILE(0x04,0x20,0x01,0x80,0x10,0x02,0x40,0x08),
  TILE(0x40,0x02,0x08,0x20,0x01,0x10,0x80,0x04),
  TILE(0x08,0x80,0x02,0x10,0x40,0x04,0x01,0x20),
  TILE(0x01,0x10,0x04,0x40,0x20,0x80,0x08,0x02),
  TILE(0x20,0x08,0x80,0x04,0x02,0x01,0x10,0x40),
  TILE(0x38,0x10,0x10,0x38,0,0,0,0), /* I */
  TILE(0x20,0x20,0x20,0x20,0x20,0x38,0,0), /* L */
  TILE(0x7c,0x10,0x10,0x10,0,0,0,0), /* T */
  TILE(0x10,0x10,0x7c,0x10,0,0,0,0), /* + */
  TILE(0x22,0x14,0x14,0x22,0,0,0,0), /* X */
  TILE(0x18,0x24,0x24,0x18,0,0,0,0), /* box */
  TILE(0x30,0x18,0x0c,0x06,0,0,0,0), /* zigzag */
  TILE(0x80,0x40,0x20,0x10,0x08,0x04,0x02,0x01),
  TILE(0,0,0,0,0,0,0,0)
};
#undef TILE

const unsigned char nes_static_palette[4] = {0x0f, 0x10, 0x0f, 0x0f};

void nes_static_init(NesStaticState *state) {
  state->cursor = 0;
  state->random = 0xa5;
  state->wait = 0;
  state->beat_held = 0;
}

unsigned char nes_static_step(NesStaticState *state, unsigned char look,
                              unsigned char energy, unsigned char band,
                              unsigned char beat, NesStaticPatch *patch) {
  unsigned char i;
  unsigned char value;
  unsigned char rising;
  if (look > NES_STATIC_LOOK_MIXED) return NES_STATIC_INVALID_LOOK;
  rising = (beat != 0 && state->beat_held == 0);
  state->beat_held = (beat != 0);
  /* Observe even while waiting, but never let beats force extra patches. */
  if (rising) {
    state->random ^= 0x5a;
    if (!state->random) state->random = 0xa5;
  }
  if (state->wait) {
    --state->wait;
    return NES_STATIC_NO_PATCH;
  }
  state->wait = 3 - (energy >> 6); /* periods 4, 3, 2, 1 */
  patch->address = 0x2000 + state->cursor;
  band ^= (band >> 3) ^ (band >> 6);
  for (i = 0; i < NES_STATIC_PATCH_SIZE; ++i) {
    value = state->random;
    value ^= (unsigned char)(value << 3);
    value ^= value >> 5;
    value ^= (unsigned char)(value << 1);
    state->random = value;
    value ^= band;
    if (look == NES_STATIC_LOOK_GLYPH)
      patch->tiles[i] = 8 + (value & 7);
    else if (look == NES_STATIC_LOOK_MIXED)
      patch->tiles[i] = value & 15;
    else
      patch->tiles[i] = value & 7;
  }
  state->cursor += NES_STATIC_PATCH_SIZE;
  if (state->cursor == 960) state->cursor = 0;
  return NES_STATIC_PATCH_READY;
}
