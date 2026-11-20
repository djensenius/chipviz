#include <stdint.h>

#include "colour_field.h"
#include "input.h"
#include "native_prefetch.h"

#define REG_DISPCNT (*(volatile uint16_t *)0x04000000)
#define REG_VCOUNT (*(volatile uint16_t *)0x04000006)
#define REG_KEYINPUT (*(volatile uint16_t *)0x04000130)
#define MODE4 0x0004
#define BG2_ENABLE 0x0400
#define DISPLAY_PAGE 0x0010
#define KEY_A 0x0001
#define KEY_B 0x0002
#define KEY_SELECT 0x0004
#define KEY_START 0x0008
#define SCREEN_WIDTH 240
#define SCREEN_HEIGHT 160
#define HALFWORDS_PER_ROW (SCREEN_WIDTH / 2)
#define BLOCK_SIZE 4
#define BLOCK_COLUMNS (SCREEN_WIDTH / BLOCK_SIZE)
#define BLOCK_ROWS (SCREEN_HEIGHT / BLOCK_SIZE)
/* The original native demo presents approximately once per four VBlanks.
 * IWRAM rendering is faster, so retain that animation cadence separately. */
#define NATIVE_PRESENT_POLLS 4u
/* Only poll/present early in a newly reached VBlank, leaving ample time for
 * palette writes and keeping the poll-time jitter below a frame. */
#define NATIVE_LATEST_VBLANK_LINE 200

#ifdef __arm__
/* The block shader runs 2,400 times per native page; execute it from the
 * GBA's fast internal RAM to leave time for VBlank input polling. */
#define GBA_FAST_CODE __attribute__((section(".iwram"), long_call))
#else
#define GBA_FAST_CODE
#endif

static volatile uint16_t *const frontbuffer = (volatile uint16_t *)0x06000000;
static volatile uint16_t *const backbuffer = (volatile uint16_t *)0x0600A000;
static volatile uint16_t *const palette_ram = (volatile uint16_t *)0x05000000;

typedef struct {
  uint8_t spectrum[8];
  uint8_t scene;
  uint8_t palette;
  uint8_t energy;
  uint8_t beat;
  uint8_t note;
} demo_state_t;

static uint16_t rgb15(uint8_t r, uint8_t g, uint8_t b) {
  return (uint16_t)((r & 31u) | ((g & 31u) << 5) | ((b & 31u) << 10));
}

static uint8_t color_index(uint8_t r, uint8_t g, uint8_t b) {
  return (uint8_t)((r >> 2) | ((g >> 2) << 3) | ((b >> 3) << 6));
}

static void wait_vblank(void) {
  while (REG_VCOUNT >= 160) {
  }
  while (REG_VCOUNT < 160) {
  }
}

static void make_demo_state(uint16_t frame, uint8_t scene_override, uint8_t palette_override, demo_state_t *state) {
  int band;
  uint8_t phrase = (uint8_t)((frame / 96u) & 3u);
  uint8_t phase = (uint8_t)(frame & 255u);

  state->scene = (uint8_t)((phrase + scene_override) & 3u);
  state->palette = (uint8_t)(((frame / 32u) + palette_override) & 7u);
  state->energy = (uint8_t)(80u + ((frame * 5u + phrase * 23u) & 95u));
  state->beat = (uint8_t)((phase & 31u) < 4u);
  state->note = (uint8_t)(48u + ((frame / 16u) & 23u));
  for (band = 0; band < 8; band++) {
    uint8_t wave = (uint8_t)((frame * (uint16_t)(band + 3) + band * 29u) & 127u);
    state->spectrum[band] = (uint8_t)(48u + (wave < 64u ? wave : 127u - wave) + phrase * 16u);
  }
}

static void make_palette(uint16_t *palette, uint16_t frame, const demo_state_t *state) {
  uint16_t i;
  uint8_t warm = (uint8_t)((state->palette * 3u + (frame >> 3)) & 7u);
  for (i = 0; i < 256; i++) {
    uint8_t r = (uint8_t)((((i & 7u) * 4u) + warm) & 31u);
    uint8_t g = (uint8_t)(((((i >> 3) & 7u) * 4u) + state->scene * 3u) & 31u);
    uint8_t b = (uint8_t)(((((i >> 6) & 3u) * 10u) + (state->energy >> 4)) & 31u);
    palette[i] = rgb15(r, g, b);
  }
  palette[color_index(31, 31, 31)] = rgb15(31, 31, 31);
}

static void fill_rect(volatile uint16_t *target, int x, int y, int width, int height, uint8_t color) {
  uint16_t pair;
  int pairs;
  int row;
  int column;

  if (x < 0) {
    width += x;
    x = 0;
  }
  if (y < 0) {
    height += y;
    y = 0;
  }
  if (x + width > SCREEN_WIDTH) {
    width = SCREEN_WIDTH - x;
  }
  if (y + height > SCREEN_HEIGHT) {
    height = SCREEN_HEIGHT - y;
  }
  if (width <= 0 || height <= 0) {
    return;
  }
  if (x & 1) {
    x--;
    width++;
  }
  if (width & 1) {
    width++;
  }

  pair = (uint16_t)(color | ((uint16_t)color << 8));
  pairs = width >> 1;
  for (row = 0; row < height; row++) {
    volatile uint16_t *dest = target + (y + row) * HALFWORDS_PER_ROW + (x >> 1);
    for (column = 0; column < pairs; column++) {
      dest[column] = pair;
    }
  }
}

static void fill_block4(volatile uint16_t *target, int block_x, int block_y, uint8_t color) {
  uint16_t pair = (uint16_t)(color | ((uint16_t)color << 8));
  int offset = block_y * BLOCK_SIZE * HALFWORDS_PER_ROW + block_x * 2;
  volatile uint16_t *row = target + offset;

  row[0] = pair;
  row[1] = pair;
  row += HALFWORDS_PER_ROW;
  row[0] = pair;
  row[1] = pair;
  row += HALFWORDS_PER_ROW;
  row[0] = pair;
  row[1] = pair;
  row += HALFWORDS_PER_ROW;
  row[0] = pair;
  row[1] = pair;
}

static GBA_FAST_CODE uint8_t block_color(int bx, int by, uint16_t frame, const demo_state_t *state) {
  uint8_t band = (uint8_t)(state->spectrum[((bx >> 3) + (by >> 2)) & 7] >> 5);
  uint8_t phase = (uint8_t)(((frame >> 1) + bx * 3 + by * 5 + state->palette * 7u) & 31u);
  uint8_t r = (uint8_t)((phase + band + (bx >> 1)) & 31u);
  uint8_t g = (uint8_t)((by + band * 3u + state->scene * 4u) & 31u);
  uint8_t b = (uint8_t)(((bx ^ by) + state->palette * 2u) & 31u);

  if (state->scene == 1u) {
    int cx = bx - (BLOCK_COLUMNS / 2);
    int cy = by - (BLOCK_ROWS / 2);
    uint8_t tunnel = (uint8_t)(((cx * cx + cy * cy) >> 1) + (frame >> 2) + state->beat * 5u);
    r = (uint8_t)((tunnel + (state->energy >> 3)) & 31u);
    g = (uint8_t)(((tunnel >> 1) + state->palette * 3u) & 31u);
    b = (uint8_t)(31u - ((tunnel + band) & 31u));
  } else if (state->scene == 2u) {
    uint8_t stripe = (uint8_t)((bx + (frame >> 2) + (state->spectrum[by & 7] >> 4)) & 31u);
    r = (uint8_t)((stripe + state->palette * 2u) & 31u);
    g = (uint8_t)((31u - stripe + band * 2u) & 31u);
    b = (uint8_t)((phase + (stripe >> 1)) & 31u);
  } else if (state->scene == 3u) {
    uint8_t star = (uint8_t)((bx * 11 + by * 17 + frame * 3u + state->note) & 63u);
    r = star < 3u ? 31u : (uint8_t)((phase + band) & 15u);
    g = star < 5u ? (uint8_t)(16u + band * 2u) : (uint8_t)((state->palette * 4u + by) & 31u);
    b = star < 7u ? 31u : (uint8_t)((bx + (state->energy >> 3)) & 31u);
  }

  if ((bx & 7) == 0 || (by & 7) == 0) {
    r = (uint8_t)(24u + state->beat * 7u);
    g = (uint8_t)((band * 4u + state->palette * 2u) & 31u);
    b = phase;
  }

  return color_index(r, g, b);
}

static void render_background_rows(volatile uint16_t *target, uint16_t frame, const demo_state_t *state,
                                   int first, int end) {
  int by;
  int bx;

  for (by = first; by < end; by++) {
    for (bx = 0; bx < BLOCK_COLUMNS; bx++) {
      fill_block4(target, bx, by, block_color(bx, by, frame, state));
    }
  }
}

static void render_overlay_items(volatile uint16_t *target, uint16_t frame, const demo_state_t *state,
                                 uint16_t keys, unsigned first, unsigned end) {
  unsigned item;
  for (item = first; item < end; ++item) {
    if (item < 8u) {
      int x = (int)item;
      int bar_height = 10 + (state->spectrum[x] >> 2);
      uint8_t bar_color = color_index(31, (uint8_t)((x * 3 + state->palette) & 31), (uint8_t)(31 - x * 3));
      uint8_t tick_color = color_index((uint8_t)(state->energy >> 3), 31, 31);
      fill_rect(target, 16 + x * 26, 150 - bar_height, 16, bar_height, bar_color);
      fill_rect(target, 20 + x * 25, 32 + ((frame + x * 13) & 63), 12, 4, tick_color);
    } else if (item == 8u && state->beat) {
      fill_rect(target, 0, 0, SCREEN_WIDTH, 4, color_index(31, 31, 31));
      fill_rect(target, 0, SCREEN_HEIGHT - 4, SCREEN_WIDTH, 4, color_index(31, 31, 31));
    } else if (item == 9u && (keys & KEY_START)) {
      fill_rect(target, 0, 78, SCREEN_WIDTH, 4, color_index(31, 31, 31));
    } else if (item == 10u && (keys & KEY_SELECT)) {
      fill_rect(target, 118, 0, 4, SCREEN_HEIGHT, color_index(31, 31, 31));
    } else if (item >= 11u && item < 25u && (keys & KEY_A)) {
      fill_rect(target, 0, (int)(item - 11u) * 12, SCREEN_WIDTH, 2, color_index(31, 31, 0));
    } else if (item >= 25u && (keys & KEY_B)) {
      fill_rect(target, (int)(item - 25u) * 12, 0, 2, SCREEN_HEIGHT, color_index(0, 31, 31));
    }
  }
}

static void render_overlay(volatile uint16_t *target, uint16_t frame, const demo_state_t *state, uint16_t keys) {
  render_overlay_items(target, frame, state, keys, 0u, GBA_PREFETCH_OVERLAYS);
}

static void make_field_palette(uint16_t *palette, const gba_cf_view_t *view) {
  uint8_t i;
  for (i = 0; i < 16u; ++i) {
    palette[i] = gba_cf_index_colour(view, i);
  }
}

static void present_palette(const uint16_t *palette, unsigned count) {
  unsigned i;
  for (i = 0; i < count; ++i) {
    palette_ram[i] = palette[i];
  }
}

int main(void) {
  uint16_t frame = 0;
  uint8_t scene = 0;
  uint8_t native_scene = 0;
  uint8_t palette = 0;
  uint8_t display_page = 0;
  uint8_t displaying_field = 0;
  uint8_t native_polls = 0;
  int native_row = 0;
  uint16_t prev_keys = 0;
  uint16_t native_keys = 0;
  uint16_t staged_palette[256];
  uint16_t prefetch_palette[256];
  demo_state_t native_state;
  demo_state_t prefetch_state;
  gba_cf_t field;
  gba_native_prefetch_t prefetch;

  gba_cf_init(&field);
  gba_native_prefetch_init(&prefetch);
  REG_DISPCNT = MODE4 | BG2_ENABLE;
  /* Do not take a pre-VBlank boot sample in addition to the frame samples. */
  wait_vblank();
  for (;;) {
    uint16_t keys = (uint16_t)(~REG_KEYINPUT & 0x03FF);
    uint8_t show_field;
    volatile uint16_t *draw_target;
    update_input(keys, &prev_keys, &scene, &palette);
    if (scene != 7u) {
      native_scene = scene;
    }
    show_field = gba_cf_step(&field, scene, palette, keys);
    draw_target = display_page ? frontbuffer : backbuffer;
    if (show_field) {
      if (!displaying_field) {
        /* Four x and four y classes encode all layout boundaries; only the
         * 16 palette entries change while this prepared page is shown. */
        gba_cf_render_bitmap(draw_target);
        make_field_palette(staged_palette, &field.view);
        wait_vblank();
        present_palette(staged_palette, 16u);
        display_page ^= 1u;
        REG_DISPCNT = (uint16_t)(MODE4 | BG2_ENABLE | (display_page ? DISPLAY_PAGE : 0u));
        displaying_field = 1u;
        gba_native_prefetch_init(&prefetch);
      } else {
        uint8_t first, end, work;
        if (gba_native_prefetch_observe(&prefetch, scene, palette, keys)) {
          make_demo_state(frame, prefetch.scene, prefetch.palette, &prefetch_state);
          make_palette(prefetch_palette, frame, &prefetch_state);
        }
        work = gba_native_prefetch_next(&prefetch, &first, &end);
        if (work == GBA_PREFETCH_BACKGROUND) {
          render_background_rows(draw_target, frame, &prefetch_state, first, end);
        } else if (work == GBA_PREFETCH_OVERLAY) {
          render_overlay_items(draw_target, frame, &prefetch_state, prefetch.keys, first, end);
        }
        make_field_palette(staged_palette, &field.view);
        wait_vblank();
        present_palette(staged_palette, 16u);
      }
      native_row = 0;
      native_polls = 0u;
    } else if (displaying_field && prefetch.ready) {
      /* The committed candidate is already complete in the hidden page.
       * Its palette and page flip are presented together at this step. */
      wait_vblank();
      present_palette(prefetch_palette, 256u);
      display_page ^= 1u;
      REG_DISPCNT = (uint16_t)(MODE4 | BG2_ENABLE | (display_page ? DISPLAY_PAGE : 0u));
      displaying_field = 0u;
      native_row = 0;
      native_polls = 0u;
      gba_native_prefetch_init(&prefetch);
      frame++;
    } else {
      uint8_t seen_visible = 0u;
      uint8_t reached_vblank = 0u;
      uint16_t line = REG_VCOUNT;
      /* If the preparation invariant ever fails, finish a page off-screen
       * rather than expose partial VRAM. A stable 30-step candidate supplies
       * at least 29 work polls for the 25-poll prefetch. */
      /* Native overlays use the baseline's one snapshot per page. Scene and
       * palette direction edges are still processed on every poll. */
      if (native_row == 0) {
        make_demo_state(frame, native_scene, palette, &native_state);
        native_keys = keys;
      }
      if (native_polls < NATIVE_PRESENT_POLLS) {
        ++native_polls;
      }
      /* Render whole block rows until the next VBlank, sampling the scanline
       * between rows so a slow native page does not hide pad polls. */
      while (native_row < BLOCK_ROWS && !reached_vblank) {
        render_background_rows(draw_target, frame, &native_state, native_row, native_row + 1);
        ++native_row;
        line = REG_VCOUNT;
        if (line < SCREEN_HEIGHT) {
          seen_visible = 1u;
        } else if (seen_visible) {
          reached_vblank = 1u;
        }
      }
      if (native_row == BLOCK_ROWS && native_polls >= NATIVE_PRESENT_POLLS) {
        uint16_t before_overlay = line;
        render_overlay(draw_target, frame, &native_state, native_keys);
        make_palette(staged_palette, frame, &native_state);
        line = REG_VCOUNT;
        if (seen_visible && before_overlay < SCREEN_HEIGHT && line >= SCREEN_HEIGHT) {
          reached_vblank = 1u;
        }
        if (!reached_vblank || line < SCREEN_HEIGHT || line >= NATIVE_LATEST_VBLANK_LINE) {
          wait_vblank();
        }
        present_palette(staged_palette, 256u);
        display_page ^= 1u;
        REG_DISPCNT = (uint16_t)(MODE4 | BG2_ENABLE | (display_page ? DISPLAY_PAGE : 0u));
        displaying_field = 0u;
        native_row = 0;
        native_polls = 0u;
        frame++;
      } else {
        if (!reached_vblank || line >= NATIVE_LATEST_VBLANK_LINE) {
          wait_vblank();
        }
      }
    }
  }
}
