/* colour-field-v0 state machine for the C64 port (shared/specs/colour-field-v0.md).
 *
 * Portable C89-style code: cc65 builds it into the .prg and the host C tests
 * build it with -std=c99 -pedantic. It has no hardware access. The C64 is a
 * `data` input role build only: no pin control, no operator pulses, no warning
 * screen. Flash/gate arms only through an absolute, P_STABLE wire value with an
 * absolute incoming scene 7 on every step of the window (route (b)).
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

#endif
