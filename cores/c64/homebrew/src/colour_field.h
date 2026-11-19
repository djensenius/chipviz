/* colour-field-v0 state machine for the C64 port (shared/specs/colour-field-v0.md).
 *
 * Portable C89-style code: cc65 builds it into the .prg and the host C tests
 * build it with -std=c99 -pedantic. It has no hardware access. The C64 is a
 * `data` input role build only: no pin control, no operator pulses, no warning
 * screen. Flash/gate arms only through an absolute, P_STABLE wire value with an
 * absolute incoming scene 7 on every step of the window (route (b)), and only
 * in builds with CF_UNMEASURED_FLASH; by default cf_tick never arms (#96).
 */
#ifndef CHIPVIZ_C64_COLOUR_FIELD_H
#define CHIPVIZ_C64_COLOUR_FIELD_H

#include <stdint.h>

#define CF_SCENE 7u
#define CF_P_STABLE 30u
#define CF_CUT_MIN 20u
#define CF_LEVEL_OPP 10u
#define CF_HID_E_LO 64u
#define CF_HID_E_HI 200u

/* Onsets since the previous step, detected by the caller against the frame or
 * poll received just before (so held flags never re-trigger). */
#define CF_EV_B 1u /* beat or bar rising edge */
#define CF_EV_X 2u /* drop rising edge */
#define CF_EV_N 4u /* note onset */

/* What happened on the last cf_tick (cf_state_t.fx); used by tests/tracing. */
#define CF_FX_CUT 1u     /* hard cut accepted */
#define CF_FX_PULSE 2u   /* flash/gate pulse started */
#define CF_FX_T 4u       /* transient detected */
#define CF_FX_LAYOUT 8u  /* next_layout ran */
#define CF_FX_APPLY 16u  /* palette request applied under the budget */
#define CF_FX_ENTER 32u  /* scene-7 entry step */
#define CF_FX_EXIT 64u   /* scene-7 exit takes effect after this step */

typedef struct {
  uint8_t scene;      /* incoming scene, raw (before the scene-7 debounce) */
  uint8_t scene_abs;  /* 1 when scene and palette are absolute in this report */
  uint8_t palette;    /* wire value, or the local selection when !wire */
  uint8_t wire;       /* 1: palette goes through the P_STABLE gate */
  uint8_t pre_armed;  /* local operator pre-arm (fixtures only; C64 passes 0) */
  uint8_t energy;
  uint8_t band[3];    /* bass, mid, treble */
  uint8_t spectrum[8];
  uint8_t events;     /* CF_EV_* */
} cf_input_t;

/* The step's displayed trace tuple. A request applied at the end of a step
 * changes the state but shows from the next step, so this is snapshotted
 * before the request is applied. */
typedef struct {
  uint8_t mode, family, layout, combo, rot;
  uint8_t lvl[3];
} cf_view_t;

typedef struct {
  cf_view_t view;
  /* scene-7 debounce */
  uint8_t started, shown, entering, deb_val;
  uint16_t deb_n, since_sw;
  /* palette: effective value (3/7 stored as 2/6 unless the latch is set) */
  uint8_t p_eff, flash;
  uint8_t w_val, w_ok, req_valid, req_p, req_armed;
  uint16_t w_n;
  /* detector */
  uint16_t acc[4];
  uint8_t prev_spec[8];
  uint8_t armed, silent, dom, cand;
  uint16_t quiet, cand_n;
  /* field */
  uint8_t layout, combo, rot, act, pulse, hyst, dim;
  uint8_t lvl[3];
  uint16_t since_cut, since_layout, gate_off, since_pulse, lg;
  uint8_t fx;
} cf_state_t;

/* One global instance: cc65 addresses globals directly, which keeps the
 * code small enough to end below the colour-field screens. */
extern cf_state_t cf;
extern cf_input_t cf_in;

void cf_init(void);
/* Runs one step on cf_in. Returns 1 when scene 7 is shown on this step. */
uint8_t cf_tick(void);
uint8_t cf_mode(void);    /* current effective mode 0..3 */
uint8_t cf_regions(void); /* current region count, 2 or 3 */
/* Displayed level and C64 colour index of a region on the last shown step. */
uint8_t cf_level(uint8_t region);
uint8_t cf_colour(uint8_t region);
/* Region of a 40x25 character cell for a layout (cells are 8x8 pixels). */
uint8_t cf_cell_region(uint8_t layout, uint8_t column, uint8_t row);

/* Keyboard HID poll: cf_keys[c] has bit r set while the key at CIA1 port A
 * column c, port B row r is held (the target scans; the joystick ports share
 * these lines). cf_read_keys fills cf_in from it, using fallback_scene until a
 * scene key has been pressed. cf_init clears the key latches. */
extern uint8_t cf_keys[8];
void cf_read_keys(uint8_t fallback_scene);

/* Raster IRQ lateness check. d012 and d011 are the raster register reads
 * (D011 bit 7 is line bit 8, read after D012, so the line may have moved on
 * between them). Returns 1 when the 9-bit line is a real line of the frame
 * and fewer than CF_IRQ_WINDOW lines after irq_line, counting across the
 * frame wrap. On-time IRQs reach the check about 6 lines late; the window
 * stays in the vertical blank (PAL 300-311 and 0-3, NTSC 16-31). */
#define CF_IRQ_WINDOW 16u
uint8_t cf_irq_on_time(uint8_t d012, uint8_t d011, uint16_t irq_line, uint16_t frame_lines);

/* One raster IRQ. A late IRQ (cf_irq_on_time false) returns CF_IRQ_LATE
 * without calling on_time, sampling input or stepping: the step is dropped,
 * not caught up, so steps stay at most one per frame (spec: Step rate) and
 * the staged output waits for the next on-time IRQ. An on-time IRQ calls
 * on_time (present the staged step, scan the keyboard), then
 * cf_read_keys(fallback_scene) and cf_tick, and returns cf_tick's result.
 * Key edges compare against the last sampled keys, so a key still held at
 * the next on-time IRQ is not lost. */
#define CF_IRQ_LATE 0xFFu
typedef void (*cf_irq_hook_t)(void);
uint8_t cf_irq_frame(uint8_t d012, uint8_t d011, uint16_t irq_line, uint16_t frame_lines,
                     cf_irq_hook_t on_time, uint8_t fallback_scene);

#endif
