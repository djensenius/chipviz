#include <conio.h>
#include <stdint.h>
#include <string.h>

#include "colour_field.h"

#define CHIPVIZ_BORDER_COLOR (*(volatile uint8_t *)0xD020)
#define CHIPVIZ_BACKGROUND_COLOR (*(volatile uint8_t *)0xD021)
#define CHIPVIZ_RASTER_LINE (*(volatile uint8_t *)0xD012)
#define CHIPVIZ_MEMORY_POINTERS (*(volatile uint8_t *)0xD018)
#define CHIPVIZ_SCREEN_RAM ((volatile uint8_t *)0x0400)
#define CHIPVIZ_COLOR_RAM ((volatile uint8_t *)0xD800)
#define CHIPVIZ_VIC_CONTROL1 (*(volatile uint8_t *)0xD011)
#define CHIPVIZ_VIC_BACKGROUND ((volatile uint8_t *)0xD021)
#define CHIPVIZ_CIA1_PORT_A (*(volatile uint8_t *)0xDC00)
#define CHIPVIZ_CIA1_PORT_B (*(volatile uint8_t *)0xDC01)
#define CHIPVIZ_CIA1_INTERRUPTS (*(volatile uint8_t *)0xDC0D)
/* D011: display on, 25 rows, scroll 3; bit 6 adds extended colour mode. */
#define CHIPVIZ_D011_NATIVE 0x1Bu
#define CHIPVIZ_ECM 0x40u
/* Four prepared colour-field screens, one per layout, at $3000-$3FFF in VIC
 * bank 0 (the Makefile checks the program ends below $3000). */
#define CHIPVIZ_FIELD_SCREENS ((uint8_t *)0x3000)
/* D018 for screen $0400 / $3000 + layout * $400, character ROM at $1000. */
#define CHIPVIZ_D018_NATIVE 0x14u
#define CHIPVIZ_D018_FIELD 0xC4u

/* Register values written together at the next VBlank. */
typedef struct {
  uint8_t control1;
  uint8_t memory;
  uint8_t border;
  uint8_t background[3];
} vic_frame_t;

static vic_frame_t pending;
static uint8_t key_columns[8];
static uint8_t latched_scene = 0xFFu;
static uint8_t latched_palette;
static uint8_t previous_beat_keys;

/* usb-hid-transport-v0 keyboard keys as a CIA1 port A column and a port B
 * row mask: 1..8 select scenes 0..7, Q W E R T Y U I select palettes 0..7.
 * Entries 0-7 are the scene keys, 8-15 the palette keys. */
static const uint8_t key_column[16] = {
  7u, 7u, 1u, 1u, 2u, 2u, 3u, 3u, 7u, 1u, 1u, 2u, 2u, 3u, 3u, 4u,
};
static const uint8_t key_row_mask[16] = {
  0x01u, 0x08u, 0x01u, 0x08u, 0x01u, 0x08u, 0x01u, 0x08u,
  0x40u, 0x02u, 0x40u, 0x02u, 0x40u, 0x02u, 0x40u, 0x02u,
};

typedef struct {
  uint8_t spectrum[8];
  uint8_t scene;
  uint8_t palette;
  uint8_t energy;
  uint8_t beat;
  uint8_t note;
} demo_state_t;

static uint8_t demo_scene(uint16_t frame) {
  return (uint8_t)((frame / 96u) & 3u);
}

static void make_demo_state(uint16_t frame, demo_state_t *state) {
  uint8_t band;
  uint8_t phrase = demo_scene(frame);

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
  /* cc65 lowercase literals encode unshifted PETSCII: uppercase in graphics mode. */
  textcolor(COLOR_LIGHTGREEN);
  cputsxy(1, 0, "chipviz c64 sdk demo");
  textcolor(COLOR_CYAN);
  cputsxy(1, 1, "cc65 vic-ii / petscii");
}

static void seed_petscii_grid(void) {
  uint8_t x;
  uint8_t y;

  for (y = 0; y < 18u; y++) {
    for (x = 0; x < 40u; x++) {
      uint16_t offset = (uint16_t)(y + 4u) * 40u + x;
      /* Screen codes 96-111 are PETSCII graphics 160-175, not reverse punctuation. */
      CHIPVIZ_SCREEN_RAM[offset] = (uint8_t)(96u + ((x + y) & 15u));
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

static void scan_keyboard(void) {
  uint8_t column;

  __asm__("sei");
  for (column = 0; column < 8u; column++) {
    CHIPVIZ_CIA1_PORT_A = (uint8_t)~(1u << column);
    key_columns[column] = (uint8_t)~CHIPVIZ_CIA1_PORT_B;
  }
  CHIPVIZ_CIA1_PORT_A = 0x7Fu;
  __asm__("cli");
}

/* Returns the single held key 0..7 of the group starting at first (0 scene
 * keys, 8 palette keys), or 0xFF when none or several are held (several can
 * be matrix ghosts). */
static uint8_t single_key(uint8_t first) {
  uint8_t index;
  uint8_t found = 0xFFu;

  for (index = 0; index < 8u; index++) {
    if (key_columns[key_column[first]] & key_row_mask[first]) {
      if (found != 0xFFu) {
        return 0xFFu;
      }
      found = index;
    }
    ++first;
  }
  return found;
}

/* Keyboard HID is a reduced, digital-only transport with no energy key:
 * energy and bands are HID_E_LO, the spectrum is 0, beat is Space and bar is
 * Return. F (fill/drop) is ignored, so there are no X events. */
static void read_cf_in(uint16_t frame) {
  uint8_t scene = single_key(0u);
  uint8_t palette = single_key(8u);
  /* Space is (7, 4) and Return is (0, 1). */
  uint8_t beat_keys = (uint8_t)(((key_columns[7] >> 4) & 1u) | ((key_columns[0] & 2u)));

  if (scene != 0xFFu) {
    latched_scene = scene;
  }
  if (palette != 0xFFu) {
    latched_palette = palette;
  }
  memset(&cf_in, 0, sizeof(cf_in));
  /* Until a scene key arrives the procedural scene is used; it is never 7. */
  cf_in.scene = latched_scene != 0xFFu ? latched_scene : demo_scene(frame);
  /* Flash can arm only from reports holding exactly one scene key and one
   * palette key together, as usb_hid.py sends them. */
  cf_in.scene_abs = (uint8_t)(scene != 0xFFu && palette != 0xFFu);
  cf_in.palette = latched_palette;
  cf_in.wire = 1u;
  cf_in.energy = CF_HID_E_LO;
  memset(cf_in.band, CF_HID_E_LO, sizeof(cf_in.band));
  if (beat_keys & (uint8_t)~previous_beat_keys) {
    cf_in.events = CF_EV_B;
  }
  previous_beat_keys = beat_keys;
}

static void prepare_field_screens(void) {
  uint8_t layout;
  uint8_t row;
  uint8_t column;
  uint8_t code;
  uint8_t pattern[40];
  uint8_t *cell = CHIPVIZ_FIELD_SCREENS;

  /* ECM: screen code bits 6-7 pick $D021-$D023 per region; code 32 is the
   * blank glyph, so every pixel shows its region's background colour.
   * Vertical layouts repeat one row pattern; horizontal ones fill each row. */
  for (layout = 0; layout < 4u; layout++) {
    for (column = 0; column < 40u; column++) {
      pattern[column] = (uint8_t)(32u + (cf_cell_region(layout, column, 0u) << 6));
    }
    for (row = 0; row < 25u; row++) {
      if (layout & 1u) {
        code = (uint8_t)(32u + (cf_cell_region(layout, 0u, row) << 6));
        memset(cell, code, 40u);
      } else {
        memcpy(cell, pattern, 40u);
      }
      cell += 40;
    }
    cell += 24;
  }
}

static void present(void) {
  CHIPVIZ_VIC_CONTROL1 = pending.control1;
  CHIPVIZ_MEMORY_POINTERS = pending.memory;
  CHIPVIZ_BORDER_COLOR = pending.border;
  CHIPVIZ_VIC_BACKGROUND[0] = pending.background[0];
  CHIPVIZ_VIC_BACKGROUND[1] = pending.background[1];
  CHIPVIZ_VIC_BACKGROUND[2] = pending.background[2];
}

static void stage_field(void) {
  pending.control1 = CHIPVIZ_D011_NATIVE | CHIPVIZ_ECM;
  pending.memory = (uint8_t)(CHIPVIZ_D018_FIELD + (cf.view.layout << 4));
  pending.border = 0u;
  pending.background[0] = cf_colour(0u);
  pending.background[1] = cf_colour(1u);
  pending.background[2] = cf_colour(2u);
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
      uint8_t cell = (uint8_t)(96u + ((x + y + frame / 4u + state->scene * 3u) & 15u));
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

  /* D018 bits 1-3: uppercase/graphics ROM at $1000; screen at $0400. */
  CHIPVIZ_MEMORY_POINTERS = CHIPVIZ_D018_NATIVE;
  draw_title();
  seed_petscii_grid();
  prepare_field_screens();
  cf_init();
  /* The program scans the keyboard matrix itself. Stopping the KERNAL's CIA1
   * timer IRQ keeps one step per VBlank: the IRQ costs several raster lines
   * and could hide raster line 255 from wait_frame. */
  CHIPVIZ_CIA1_INTERRUPTS = 0x7Fu;
  (void)CHIPVIZ_CIA1_INTERRUPTS;
  /* The first present() repeats the boot registers until a step is staged. */
  pending.control1 = CHIPVIZ_D011_NATIVE;
  pending.memory = CHIPVIZ_D018_NATIVE;
  pending.border = CHIPVIZ_BORDER_COLOR;
  pending.background[0] = CHIPVIZ_BACKGROUND_COLOR;
  for (;;) {
    wait_frame();
    /* All of the previous step's register changes land in one VBlank. */
    present();
    scan_keyboard();
    read_cf_in(frame);
    if (cf_tick()) {
      stage_field();
    } else {
      make_demo_state(frame, &state);
      pending.control1 = CHIPVIZ_D011_NATIVE;
      pending.memory = CHIPVIZ_D018_NATIVE;
      pending.border = (uint8_t)((state.palette + state.beat * 8u) & 15u);
      pending.background[0] = (uint8_t)((state.scene * 2u + state.energy / 32u) & 15u);
      update_petscii_rows(frame, &state);
      draw_channel_meters(&state);
    }
    ++frame;
  }
  return 0;
}
