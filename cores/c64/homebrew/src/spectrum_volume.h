/* Isolated C64 SpectrumVolume core. No scene selection or hardware access. */
#ifndef CHIPVIZ_C64_SPECTRUM_VOLUME_H
#define CHIPVIZ_C64_SPECTRUM_VOLUME_H

#include <stdint.h>

#define SV_BANDS 8u
#define SV_CELLS 1000u

typedef struct {
  uint8_t height[SV_BANDS];
  uint8_t palette[SV_BANDS];
  uint8_t next_palette;
} sv_state_t;

/* Reset before entry. The caller must provide an exclusively owned, blank
 * 40x25 screen (code 32) on a black background, in uppercase/graphics ROM
 * mode. This does not clear screen or colour RAM. */
void sv_init(sv_state_t *state, uint8_t palette);

/* Once per accepted display step, in the main loop, not in the raster IRQ.
 * screen and colour point to at least 1000 writable cells. No VIC registers
 * or cells outside the three bar slices are touched. The caller must prevent
 * concurrent writers, and reinitialise after any interruption or surface
 * change. palette is an arbitrary byte; only its low three bits are used. */
void sv_step(sv_state_t *state, const uint8_t spectrum[SV_BANDS], uint8_t energy, uint8_t palette,
             volatile uint8_t *screen, volatile uint8_t *colour);

#endif
