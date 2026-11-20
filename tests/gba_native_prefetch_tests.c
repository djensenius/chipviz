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

static void prepare(gba_native_prefetch_t *p, uint8_t scene, uint8_t palette, uint16_t keys,
                    uint16_t stable) {
  uint8_t first = 0, end = 0;
  uint8_t kind;
  gba_native_prefetch_observe(p, scene, palette, keys, stable);
  kind = gba_native_prefetch_next(p, &first, &end);
  if (kind == GBA_PREFETCH_BACKGROUND) {
    require(end - first == 10 && end <= 40, "bounded background work");
    require(!p->ready, "page is not ready before the overlay is drawn");
    if (end == GBA_PREFETCH_ROWS) {
      /* Production draws the overlay before marking the page complete. */
      gba_native_prefetch_complete(p);
    }
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
    /* A field-entry page flip can consume the first poll. */
    if (step > 1) {
      prepare(&p, 3, palette, keys, field.scene_count);
    }
    if (step < 30) {
      require(field.shown, "exit cannot happen before 30 stable polls");
      require(!p.ready, "hidden page cannot complete before the fourth batch");
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
    prepare(&p, 3, 0, 0, field.scene_count);
  }
  require(p.active && !p.ready && p.rows == 30, "first candidate has three prepared batches");
  require(gba_cf_step(&field, 2, 4, 2), "changed 30th scene poll cancels exit");
  prepare(&p, 2, 4, 2, field.scene_count);
  require(field.shown && !p.active && !p.ready, "changed candidate discards partial page");
  require(gba_cf_step(&field, 7, 0, 0), "return to field cancels exit");
  prepare(&p, 7, 0, 0, field.scene_count);
  require(!p.active && !p.ready, "cancelled page is discarded");
  for (step = 0; step < 30; ++step) {
    require(gba_cf_step(&field, 1, 5, 0), "new candidate remains field for 30 polls");
    prepare(&p, 1, 5, 0, field.scene_count);
  }
  require(!field.shown && p.ready && p.scene == 1, "new stable candidate exits with a prepared page");
  require(!gba_cf_step(&field, 1, 5, 0), "new candidate is shown exactly on next step");
}

static void snapshot_chords(void) {
  gba_native_prefetch_t p;
  gba_native_prefetch_init(&p);
  prepare(&p, 3, 1, 1, 27);
  prepare(&p, 3, 6, 0, 28);
  prepare(&p, 3, 6, 2, 29);
  require(p.keys == 1 && p.palette == 1, "sequential presses cannot become a chord");
  prepare(&p, 2, 4, 3, 1);
  prepare(&p, 2, 4, 3, 27);
  prepare(&p, 2, 7, 0, 28);
  require(p.keys == 3 && p.palette == 4, "real chord is retained as a page-start snapshot");
  prepare(&p, 7, 0, 0, 1);
  prepare(&p, 2, 7, 0, 27);
  require(p.keys == 0 && p.palette == 7, "cancelled input cannot replay on a later page");
}

static void late_snapshot_regression(void) {
  gba_cf_t field;
  gba_native_prefetch_t p;
  uint16_t previous = 0;
  uint8_t scene = 7, palette = 0;
  unsigned step;
  gba_cf_init(&field);
  gba_native_prefetch_init(&p);
  gba_cf_step(&field, scene, palette, 0);
  for (step = 1; step <= 30; ++step) {
    uint16_t keys = step == 1 ? KEY_DOWN | 1u : (step == 3 ? KEY_RIGHT : 0u);
    update_input(keys, &previous, &scene, &palette);
    require(gba_cf_step(&field, scene, palette, keys), "field stays visible for all 30 exit polls");
    prepare(&p, scene, palette, keys, field.scene_count);
  }
  printf("late snapshot regression: current keys %u palette %u; prefetched keys %u palette %u\n",
         0u, palette, p.keys, p.palette);
  require(p.ready, "candidate page is ready");
  require(p.keys == 0u, "released A before the native page-start window must not reappear");
  require(p.palette == palette, "palette changed before the native page-start window must be used");
  update_input(KEY_DOWN, &previous, &scene, &palette);
  require(scene == 2 && !gba_cf_step(&field, scene, palette, KEY_DOWN),
          "a native scene pulse on the exit step is observed without another field debounce");
  require(p.scene == 3, "the current implementation still presents the earlier candidate");
}

static void baseline_page_trace(void) {
  gba_cf_t field;
  gba_native_prefetch_t p;
  uint16_t previous = 0, baseline_previous = 0;
  uint8_t scene = 7, palette = 0, baseline_scene = 3, baseline_palette = 0;
  uint8_t baseline_pages[2], actual_pages[2];
  uint16_t baseline_keys[2], actual_keys[2];
  unsigned step;
  gba_cf_init(&field);
  gba_native_prefetch_init(&p);
  gba_cf_step(&field, scene, palette, 0);
  for (step = 1; step <= 31; ++step) {
    uint16_t keys = step == 1 ? KEY_DOWN : step == 27 ? 1u : step == 31 ? KEY_DOWN | 2u : 0u;
    uint8_t shown;
    update_input(keys, &previous, &scene, &palette);
    shown = gba_cf_step(&field, scene, palette, keys);
    if (step <= 30) {
      require(shown, "field shown through final debounce poll");
      prepare(&p, scene, palette, keys, field.scene_count);
    } else {
      require(!shown && p.ready, "completed native page is presented on next step");
      actual_pages[0] = p.scene;
      actual_keys[0] = p.keys;
    }
    if (step == 27) {
      /* Baseline starts this page four polls before presentation. */
      update_input(keys, &baseline_previous, &baseline_scene, &baseline_palette);
      baseline_pages[0] = baseline_scene;
      baseline_keys[0] = keys;
    }
  }
  /* The held display-poll change is sampled at the next native page start. */
  update_input(KEY_DOWN | 2u, &previous, &scene, &palette);
  update_input(KEY_DOWN | 2u, &baseline_previous, &baseline_scene, &baseline_palette);
  actual_pages[1] = scene;
  actual_keys[1] = KEY_DOWN | 2u;
  baseline_pages[1] = baseline_scene;
  baseline_keys[1] = KEY_DOWN | 2u;
  require(baseline_pages[0] == 3 && baseline_pages[1] == 2,
          "baseline has one-page scene lag after the display-poll change");
  require(memcmp(actual_pages, baseline_pages, sizeof(actual_pages)) == 0 &&
          memcmp(actual_keys, baseline_keys, sizeof(actual_keys)) == 0,
          "exit and next page snapshots match baseline trace");
  printf("native page trace: baseline %u,%u; exit path %u,%u (one-page lag)\n",
         baseline_pages[0], baseline_pages[1], actual_pages[0], actual_pages[1]);
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
        prepare(&p, scene, palette, keys, field.scene_count);
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
  late_snapshot_regression();
  baseline_page_trace();
  ring_sequences();
  printf("gba native prefetch tests: %u checks, %u failures\n", checks, failures);
  return failures ? EXIT_FAILURE : EXIT_SUCCESS;
}
