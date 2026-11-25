#ifndef CHIPVIZ_GBA_LINES_H
#define CHIPVIZ_GBA_LINES_H

#include <stddef.h>
#include <stdint.h>

#define LINES_WIDTH 240u
#define LINES_HEIGHT 160u
#define LINES_PAGE_HALFWORDS (LINES_WIDTH * LINES_HEIGHT / 2u)
#define LINES_BATCH_HALFWORDS 240u
#define LINES_MAX_STEP_DELTA 8u
#define LINES_KEY_A 0x0001u
#define LINES_KEY_B 0x0002u
#define LINES_KEY_L 0x0200u

typedef struct {
  uint32_t step;
  uint16_t keys, phase, beat_age, section_age;
  uint8_t initialized, fold, target, energy, phrase_beats;
} lines_state_t;

typedef struct {
  uint16_t phase;
  uint8_t fold, energy;
  uint16_t palette[256];
} lines_snapshot_t;

typedef struct {
  lines_snapshot_t snapshot;
  size_t next;
  uint8_t active;
} lines_render_t;

void lines_init(lines_state_t *state);
/* First sample establishes held keys; duplicate or delta > 8 samples do
 * nothing and return 0. Unsigned subtraction permits a counter wrap. For an
 * accepted gap, the newly sampled L level applies to every elapsed step and
 * an A rising edge to the final step; unsampled edges cannot be recovered. */
int lines_observe(lines_state_t *state, uint32_t step, uint16_t keys);
void lines_snapshot(const lines_state_t *state, lines_snapshot_t *snapshot);
/* Zero-initialize a fresh render before calling batch or cancel. Begin
 * restarts at zero; cancel never touches the destination and leaves a
 * completed render complete. */
void lines_render_begin(lines_render_t *render, const lines_snapshot_t *snapshot);
void lines_render_cancel(lines_render_t *render);
/* Writes at most min(budget, LINES_BATCH_HALFWORDS) halfwords. Returns -1
 * on invalid destination/size, 0 while incomplete (including a fresh or
 * cancelled partial render), 1 when complete, including subsequent calls.
 * The same aligned destination must be supplied until completion or restart. */
int lines_render_batch(lines_render_t *render, volatile uint16_t *page,
                       size_t page_halfwords, size_t budget);

#endif
