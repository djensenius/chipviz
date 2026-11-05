#ifndef CHIPVIZ_GBA_INPUT_H
#define CHIPVIZ_GBA_INPUT_H

#include <stdint.h>

#define KEY_RIGHT 0x0010
#define KEY_LEFT 0x0020
#define KEY_UP 0x0040
#define KEY_DOWN 0x0080
#define GBA_SCENE_COUNT 4u
#define GBA_PALETTE_COUNT 8u

/* HID v0: Up/Down select scenes, Right/Left select palettes.
 * Apply pressed edges only, with no held repeat. Opposing held directions
 * cancel on their axis, including when one was already held. Still record
 * every key sample so releasing one side does not synthesize a press.
 */
static void update_input(uint16_t keys, uint16_t *prev_keys, uint8_t *scene, uint8_t *palette) {
  uint16_t pressed = (uint16_t)(keys & ~*prev_keys);
  *prev_keys = keys;

  if ((keys & (KEY_UP | KEY_DOWN)) != (KEY_UP | KEY_DOWN)) {
    if (pressed & KEY_UP) {
      *scene = (uint8_t)((*scene + 1u) % GBA_SCENE_COUNT);
    } else if (pressed & KEY_DOWN) {
      *scene = (uint8_t)((*scene + GBA_SCENE_COUNT - 1u) % GBA_SCENE_COUNT);
    }
  }
  if ((keys & (KEY_RIGHT | KEY_LEFT)) != (KEY_RIGHT | KEY_LEFT)) {
    if (pressed & KEY_RIGHT) {
      *palette = (uint8_t)((*palette + 1u) % GBA_PALETTE_COUNT);
    } else if (pressed & KEY_LEFT) {
      *palette = (uint8_t)((*palette + GBA_PALETTE_COUNT - 1u) % GBA_PALETTE_COUNT);
    }
  }
}

#endif
