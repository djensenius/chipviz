#include <6502.h>
#include <cbm.h>
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
#define CHIPVIZ_VIC_IRQ_STATUS (*(volatile uint8_t *)0xD019)
#define CHIPVIZ_VIC_IRQ_ENABLE (*(volatile uint8_t *)0xD01A)
#define CHIPVIZ_IRQ_VECTOR (*(volatile uint16_t *)0x0314)
/* Raster IRQ line, chosen at boot from the KERNAL's PAL/NTSC flag: both are
 * in the vertical blank, below the visible bottom border on PAL (312 lines,
 * also Drean) and above the visible top border on NTSC (263 lines, where
 * line 300 does not exist). Bit 8 of the line is D011 bit 7. */
#define CHIPVIZ_IRQ_LINE_PAL 300u
#define CHIPVIZ_IRQ_LINE_NTSC 16u
/* CPU clocks. NTSC has 65 cycles a line (6567R8, 263 lines) or 64 (early
 * 6567R56A, 262 lines); PAL has 63 (Drean: 65 at a faster clock, also under
 * 60 frames a second). The program counts the lines at boot. */
#define CHIPVIZ_CLOCK_PAL 985248ul
#define CHIPVIZ_CLOCK_NTSC 1022727ul
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
/* Shared with the raster IRQ. The main loop publishes the native border,
 * background and procedural scene; the IRQ publishes whether scene 7 is shown
 * and counts frames. Each is one byte, and the two colours change together
 * with interrupts off, so neither side reads a torn value. */
static volatile uint8_t native_border;
static volatile uint8_t native_background;
static volatile uint8_t native_scene;
static volatile uint8_t field_shown;
static volatile uint8_t irq_frames;
static uint8_t irq_stack[128];
static uint16_t irq_line;
static uint8_t irq_line_bit8;

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

/* Waits for the next raster IRQ. */
static void wait_frame(void) {
  uint8_t seen = irq_frames;
  while (irq_frames == seen) {
  }
}

/* Runs inside the IRQ, which already has interrupts off. */
static void scan_keyboard(void) {
  static const uint8_t select[8] = {0xFEu, 0xFDu, 0xFBu, 0xF7u, 0xEFu, 0xDFu, 0xBFu, 0x7Fu};
  uint8_t column;

  for (column = 0; column < 8u; column++) {
    CHIPVIZ_CIA1_PORT_A = select[column];
    cf_keys[column] = (uint8_t)~CHIPVIZ_CIA1_PORT_B;
  }
  CHIPVIZ_CIA1_PORT_A = 0x7Fu;
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

/* The border goes first so it changes inside the vertical blank. Bit 7 of
 * D011 keeps raster compare bit 8 of the IRQ line. */
static void present(void) {
  CHIPVIZ_BORDER_COLOR = pending.border;
  CHIPVIZ_VIC_CONTROL1 = (uint8_t)(pending.control1 | irq_line_bit8);
  CHIPVIZ_MEMORY_POINTERS = pending.memory;
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

static void stage_native(void) {
  pending.control1 = CHIPVIZ_D011_NATIVE;
  pending.memory = CHIPVIZ_D018_NATIVE;
  pending.border = native_border;
  pending.background[0] = native_background;
}

/* Raster IRQ in the vertical blank (line 300 PAL, 16 NTSC): one colour-field
 * step per VBlank. The previous step's registers land first, then the
 * keyboard is sampled and the next step is staged. Only this handler calls
 * these functions, so cc65's static locals are never shared with the main
 * loop; cc65's IRQ wrapper saves the runtime zero page and uses irq_stack. */
static uint8_t raster_irq(void) {
  uint8_t raster_low;
  uint8_t raster_high;
  uint8_t raster_limit;
  uint8_t on_time;
  uint8_t shown;

  if (!(CHIPVIZ_VIC_IRQ_STATUS & 1u)) {
    return IRQ_NOT_HANDLED;
  }
  /* Acknowledge first, so an IRQ that fires while a step overruns is still
   * taken; it is then late and dropped. */
  CHIPVIZ_VIC_IRQ_STATUS = 1u;
  /* A late IRQ (one that starts CF_IRQ_WINDOW lines after its line or more,
   * after a step overran a frame) drops this frame's step entirely: no
   * register writes (so they never change mid-screen), no input sampling and
   * no tick, so steps never come closer than a frame. On the 6567R56A
   * (60.99 frames a second) cf_irq_frame also drops one IRQ's step in 31, so
   * there are never more than 60 steps in a second. Nothing new is staged on
   * a dropped step, so the next on-time IRQ writes the same registers again. */
  raster_low = CHIPVIZ_RASTER_LINE; /* D012 first, as cf_irq_on_time expects */
  raster_high = CHIPVIZ_VIC_CONTROL1;
  /* Inline (CF_IRQ_ON_TIME, not cf_irq_on_time): a call costs about 7 lines
   * before the border write. */
  CF_IRQ_ON_TIME(on_time, raster_low, raster_high, raster_limit);
  if (on_time) {
    present();
  }
  shown = cf_irq_frame(on_time, scan_keyboard, native_scene);
  if (shown == CF_IRQ_LATE) {
    return IRQ_HANDLED;
  }
  if (shown) {
    stage_field();
    field_shown = 1u;
  } else {
    stage_native();
    field_shown = 0u;
  }
  ++irq_frames;
  return IRQ_HANDLED;
}

/* IRQ entry for the $0314 vector: cc65's handler chain, then the KERNAL's
 * register restore and RTI at $EA81. It skips the KERNAL's own IRQ work
 * (clock, cursor, keyboard buffer), which would only slow the native demo. */
/* Lines per frame: the highest line with bit 8 set, plus 257. Run with
 * interrupts off; one pass of the loop is well under a line (63 cycles). */
static uint16_t count_frame_lines(void) {
  uint8_t last = 0;
  uint8_t line;

  while (!(CHIPVIZ_VIC_CONTROL1 & 0x80u)) {
  }
  while (CHIPVIZ_VIC_CONTROL1 & 0x80u) {
    line = CHIPVIZ_RASTER_LINE;
    if (line > last) {
      last = line;
    }
  }
  return (uint16_t)(257u + last);
}

static void irq_entry(void) {
  __asm__("cld");
  __asm__("jsr callirq");
  __asm__("jmp $EA81");
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
  uint16_t lines;
  demo_state_t state;

  /* D018 bits 1-3: uppercase/graphics ROM at $1000; screen at $0400. */
  CHIPVIZ_MEMORY_POINTERS = CHIPVIZ_D018_NATIVE;
  draw_title();
  seed_petscii_grid();
  prepare_field_screens();
  cf_init();
  /* The program scans the keyboard matrix itself. Stopping the KERNAL's CIA1
   * timer IRQ leaves the raster IRQ as the only interrupt source. */
  CHIPVIZ_CIA1_INTERRUPTS = 0x7Fu;
  (void)CHIPVIZ_CIA1_INTERRUPTS;
  /* The first IRQ repeats the boot registers until a step is staged. */
  pending.control1 = CHIPVIZ_D011_NATIVE;
  pending.memory = CHIPVIZ_D018_NATIVE;
  pending.border = CHIPVIZ_BORDER_COLOR;
  pending.background[0] = CHIPVIZ_BACKGROUND_COLOR;
  native_border = pending.border;
  native_background = pending.background[0];
  native_scene = demo_scene(frame);
  __asm__("sei");
  lines = count_frame_lines();
  if (get_tv() == TV_NTSC) {
    irq_line = CHIPVIZ_IRQ_LINE_NTSC;
    /* 262 lines is the 6567R56A (64 cycles a line, about 60.99 frames a
     * second); anything but the 6567R8's 263 is treated the same way, which
     * only ever drops more steps. */
    (void)cf_irq_rate(lines, lines == 263u ? 65u : 64u, CHIPVIZ_CLOCK_NTSC);
  } else {
    irq_line = CHIPVIZ_IRQ_LINE_PAL;
    (void)cf_irq_rate(lines, 63u, CHIPVIZ_CLOCK_PAL);
  }
  (void)cf_irq_init(irq_line, lines);
  set_irq(raster_irq, irq_stack, sizeof(irq_stack));
  CHIPVIZ_IRQ_VECTOR = (uint16_t)irq_entry;
  irq_line_bit8 = (uint8_t)((irq_line >> 1) & 0x80u);
  CHIPVIZ_RASTER_LINE = (uint8_t)irq_line;
  CHIPVIZ_VIC_CONTROL1 = (uint8_t)(CHIPVIZ_D011_NATIVE | irq_line_bit8);
  CHIPVIZ_VIC_IRQ_STATUS = 0x0Fu;
  CHIPVIZ_VIC_IRQ_ENABLE = 1u;
  __asm__("cli");
  for (;;) {
    wait_frame();
    if (!field_shown) {
      make_demo_state(frame, &state);
      __asm__("sei");
      native_border = (uint8_t)((state.palette + state.beat * 8u) & 15u);
      native_background = (uint8_t)((state.scene * 2u + state.energy / 32u) & 15u);
      __asm__("cli");
      update_petscii_rows(frame, &state);
      draw_channel_meters(&state);
    }
    ++frame;
    native_scene = demo_scene(frame);
  }
  return 0;
}
