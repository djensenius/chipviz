#ifndef CHIPVIZ_GBA_COLOUR_FIELD_H
#define CHIPVIZ_GBA_COLOUR_FIELD_H

#include <stdint.h>

/* Data-role, digital-only gamepad transport. Scene and palette are relative
 * rings, so neither can arm flash. Each call is one sampled VBlank step. */
typedef struct {
  uint8_t mode, family, layout, combo, rot, level[3];
} gba_cf_view_t;

typedef struct {
  gba_cf_view_t view;
  uint8_t started, shown, entering, scene_value;
  uint16_t scene_count, since_switch;
  uint16_t previous_keys;
  uint8_t palette, wire_value, wire_used, request, request_valid;
  uint16_t wire_count;
  uint16_t energy_acc;
  uint8_t transient_armed, layout, combo, rot, action, level[3];
  uint16_t since_cut, level_gap;
} gba_cf_t;

void gba_cf_init(gba_cf_t *cf);
/* Returns whether the field is shown on this step. view is the displayed
 * trace, before any palette request applied at the end of the step. */
uint8_t gba_cf_step(gba_cf_t *cf, uint8_t scene, uint8_t palette, uint16_t keys);

/* The bitmap encodes four x and four y classes as y_class*4+x_class.
 * The 16 colours select the appropriate region for any of the four layouts.
 * All can therefore be updated together during VBlank without redrawing. */
uint8_t gba_cf_pixel_index(unsigned x, unsigned y);
uint16_t gba_cf_index_colour(const gba_cf_view_t *view, uint8_t index);
void gba_cf_render_bitmap(volatile uint16_t *target);

#endif
