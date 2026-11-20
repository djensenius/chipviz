#include "colour_field.h"
#include "input.h"
#include "native_prefetch.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static unsigned checks, failures;

static void require(int condition, const char *message) {
  ++checks;
  if (!condition) {
    fprintf(stderr, "FAIL: %s\n", message);
    ++failures;
  }
}

static void prepare(gba_native_prefetch_t *p, uint8_t scene, uint8_t palette, uint16_t keys) {
  uint8_t first = 0, end = 0;
  uint8_t kind;
  gba_native_prefetch_observe(p, scene, palette, keys);
  kind = gba_native_prefetch_next(p, &first, &end);
  if (kind == GBA_PREFETCH_BACKGROUND) {
    require(end - first == 2 && end <= 40, "bounded background work");
  } else if (kind == GBA_PREFETCH_OVERLAY) {
    require(end - first == 9 && end <= 45, "bounded overlay work");
  }
}

static void stable_exit_and_snapshot(void) {
  gba_cf_t field, reference;
  gba_native_prefetch_t p;
  unsigned step;
  gba_cf_init(&field);
  gba_cf_init(&reference);
  gba_native_prefetch_init(&p);
  require(gba_cf_step(&field, 7, 0, 0) && gba_cf_step(&reference, 7, 0, 0), "power-on field");
  for (step = 1; step <= 30; ++step) {
    uint8_t palette = step == 1 ? 1u : 6u;
    uint16_t keys = step == 1 ? 1u : (step >= 10 && step < 20 ? 0x0202u : 0u);
    uint8_t shown = gba_cf_step(&field, 3, palette, keys);
    uint8_t ref_shown = gba_cf_step(&reference, 3, palette, keys);
    require(shown && ref_shown && memcmp(&field.view, &reference.view, sizeof(field.view)) == 0,
            "field remains unchanged by native preparation");
    /* A field-entry page flip can consume the first poll; 29 still suffice. */
    if (step > 1) {
      prepare(&p, 3, palette, keys);
    }
    if (step < 30) {
      require(field.shown, "exit cannot happen before 30 stable polls");
    }
  }
  require(!field.shown && p.ready, "hidden native page ready at end of 30th poll");
  require(p.scene == 3 && p.palette == 6 && p.keys == 0u, "page keeps its initial snapshot");
  require(!gba_cf_step(&field, 2, 7, 3), "native begins on the next step despite new input");
  require(p.scene == 3 && p.palette == 6 && p.keys == 0u, "post-commit changes do not replace the page");
}

static void final_poll_change_and_cancel(void) {
  gba_cf_t field;
  gba_native_prefetch_t p;
  unsigned step;
  gba_cf_init(&field);
  gba_native_prefetch_init(&p);
  gba_cf_step(&field, 7, 0, 0);
  for (step = 0; step < 29; ++step) {
    require(gba_cf_step(&field, 3, 0, 0), "first candidate remains field");
    prepare(&p, 3, 0, 0);
  }
  require(p.ready, "first candidate was prepared");
  require(gba_cf_step(&field, 2, 4, 2), "changed 30th scene poll cancels exit");
  prepare(&p, 2, 4, 2);
  require(field.shown && !p.ready && p.scene == 2 && p.palette == 4 && p.keys == 2,
          "changed candidate restarts with a new snapshot");
  require(gba_cf_step(&field, 7, 0, 0), "return to field cancels exit");
  prepare(&p, 7, 0, 0);
  require(!p.active && !p.ready, "cancelled page is discarded");
  for (step = 0; step < 30; ++step) {
    require(gba_cf_step(&field, 1, 5, 0), "new candidate remains field for 30 polls");
    prepare(&p, 1, 5, 0);
  }
  require(!field.shown && p.ready && p.scene == 1, "new stable candidate exits with a prepared page");
  require(!gba_cf_step(&field, 1, 5, 0), "new candidate is shown exactly on next step");
}

static void snapshot_chords(void) {
  gba_native_prefetch_t p;
  gba_native_prefetch_init(&p);
  prepare(&p, 3, 1, 1);
  prepare(&p, 3, 6, 0);
  prepare(&p, 3, 6, 2);
  require(p.keys == 1 && p.palette == 1, "sequential presses cannot become a chord");
  prepare(&p, 2, 4, 3);
  prepare(&p, 2, 7, 0);
  require(p.keys == 3 && p.palette == 4, "real chord is retained as a page-start snapshot");
  prepare(&p, 7, 0, 0);
  prepare(&p, 2, 7, 0);
  require(p.keys == 0 && p.palette == 7, "cancelled input cannot replay on a later page");
}

static void ring_sequences(void) {
  gba_cf_t field;
  gba_native_prefetch_t p;
  uint16_t previous = 0;
  uint8_t scene = 0, palette = 0, displaying_field = 0, committed_scene = 0;
  uint32_t random = 0x12569u;
  unsigned step, exits = 0;
  gba_cf_init(&field);
  gba_native_prefetch_init(&p);
  for (step = 0; step < 100000; ++step) {
    uint16_t keys = 0;
    uint8_t shown, was_shown = field.shown;
    random ^= random << 13;
    random ^= random >> 17;
    random ^= random << 5;
    if ((random & 63u) == 0u) {
      keys = (random & 64u) ? 0x0040u : 0x0080u;
    }
    if ((random & 255u) == 1u) {
      keys |= (random & 256u) ? 0x0010u : 0x0020u;
    }
    keys |= (uint16_t)((random >> 16) & 3u);
    update_input(keys, &previous, &scene, &palette);
    shown = gba_cf_step(&field, scene, palette, keys);
    if (shown) {
      if (!displaying_field) {
        displaying_field = 1u;
        gba_native_prefetch_init(&p);
      } else {
        prepare(&p, scene, palette, keys);
      }
      if (was_shown && !field.shown) {
        committed_scene = scene;
        require(p.ready && p.scene == committed_scene, "committed ring candidate is ready");
      }
    } else if (displaying_field) {
      require(p.ready && p.scene == committed_scene, "ring exits on next step with committed page");
      displaying_field = 0u;
      gba_native_prefetch_init(&p);
      ++exits;
    }
  }
  require(exits > 10, "random ring sequence exercised exits");
}

int main(void) {
  stable_exit_and_snapshot();
  final_poll_change_and_cancel();
  snapshot_chords();
  ring_sequences();
  printf("gba native prefetch tests: %u checks, %u failures\n", checks, failures);
  return failures ? EXIT_FAILURE : EXIT_SUCCESS;
}
