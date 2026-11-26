#ifndef CHIPVIZ_NES_STATIC_SCENE_H
#define CHIPVIZ_NES_STATIC_SCENE_H

/* Isolated scene core, not connected to the legacy scene selector. */
/* Internal looks only: these are not extended-catalog variant identifiers. */
#define NES_STATIC_LOOK_PIXEL 0
#define NES_STATIC_LOOK_GLYPH 1
#define NES_STATIC_LOOK_MIXED 2
#define NES_STATIC_NO_PATCH 0
#define NES_STATIC_PATCH_READY 1
#define NES_STATIC_INVALID_LOOK 2
#define NES_STATIC_PATCH_SIZE 8
#define NES_STATIC_BLANK_TILE 16

typedef struct {
  unsigned int cursor; /* tile offset, 0..952 in multiples of eight */
  unsigned char random;
  unsigned char wait;
  unsigned char beat_held;
} NesStaticState;

typedef struct {
  unsigned int address; /* PPU nametable address */
  unsigned char tiles[NES_STATIC_PATCH_SIZE];
} NesStaticPatch;

/* Pattern table entries 0..16, two NES bitplanes per tile. */
extern const unsigned char nes_static_chr[17 * 16];
/* Four illustrative background palette entries. */
extern const unsigned char nes_static_palette[4];

void nes_static_init(NesStaticState *state);
/* Returns PATCH_READY, NO_PATCH, or INVALID_LOOK (all prefixed NES_STATIC_).
   An invalid look changes neither state nor patch; NO_PATCH leaves the patch
   untouched but advances state. Beat is a level, not a pulse queue. */
unsigned char nes_static_step(NesStaticState *state, unsigned char look,
                              unsigned char energy, unsigned char band,
                              unsigned char beat, NesStaticPatch *patch);

#endif
