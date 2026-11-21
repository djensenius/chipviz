#include "spectrum_volume.h"

/* Dark VIC-II colours, never black or white. The background stays black.
 * Three vertically separated, staggered slices suggest depth without a
 * custom character set. Code 160 is reverse space (a solid cell) in the
 * uppercase/graphics character ROM. */
static const uint8_t sv_colours[8] = {6u, 4u, 5u, 9u, 2u, 11u, 8u, 12u};
static const uint8_t sv_base[3] = {8u, 15u, 22u};
static const uint8_t sv_shift[3] = {2u, 1u, 0u};
static const uint8_t sv_width[3] = {1u, 1u, 2u};

void sv_init(sv_state_t *state, uint8_t palette) {
  uint8_t band;
  for (band = 0u; band < SV_BANDS; ++band) {
    state->height[band] = 0u;
    state->palette[band] = (uint8_t)(palette & 7u);
  }
  state->next_palette = 0u;
}

void sv_step(sv_state_t *state, const uint8_t spectrum[SV_BANDS], uint8_t energy, uint8_t palette,
             volatile uint8_t *screen, volatile uint8_t *colour) {
  uint8_t band;
  uint8_t slice;
  uint8_t row;
  uint8_t x;
  uint8_t old;
  uint8_t height;
  uint8_t desired;
  uint8_t changed;
  uint8_t ink;
  uint16_t offset;

  palette &= 7u;
  for (band = 0u; band < SV_BANDS; ++band) {
    old = state->height[band];
    desired = (uint8_t)(spectrum[band] >> 5);
    if (energy < 64u) {
      desired >>= 1;
    }
    if (desired > 5u) {
      desired = 5u;
    }
    height = old;
    if (height < desired) {
      ++height;
    } else if (height > desired) {
      --height;
    }
    changed = (uint8_t)(band == state->next_palette && state->palette[band] != palette);
    if (changed) {
      state->palette[band] = palette;
    }
    for (slice = 0u; slice < 3u; ++slice) {
      ink = sv_colours[(uint8_t)((state->palette[band] + band + slice) & 7u)];
      offset = (uint16_t)sv_base[slice] * 40u + 4u + (uint16_t)band * 4u + sv_shift[slice];
      /* Recolour only this step's selected band; at most 20 cells. */
      if (changed) {
        for (row = 0u; row < height && row < old; ++row) {
          for (x = 0u; x < sv_width[slice]; ++x) {
            colour[offset - (uint16_t)row * 40u + x] = ink;
          }
        }
      }
      if (height > old) {
        offset -= (uint16_t)old * 40u;
        for (x = 0u; x < sv_width[slice]; ++x) {
          colour[offset + x] = ink;
          screen[offset + x] = 160u;
        }
      } else if (height < old) {
        offset -= (uint16_t)height * 40u;
        for (x = 0u; x < sv_width[slice]; ++x) {
          screen[offset + x] = 32u;
        }
      }
    }
    state->height[band] = height;
  }
  state->next_palette = (uint8_t)((state->next_palette + 1u) & 7u);
}
