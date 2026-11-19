#ifndef CHIPVIZ_N64_VISUAL_MATH_H
#define CHIPVIZ_N64_VISUAL_MATH_H

#include <stdint.h>

typedef struct {
  int x0, y0, x1, y1;
} meter_rect_t;

/* Joybus currently decodes scenes 0..15; also bound the full shared u8 range.
 * Preserve the existing color ramps, deliberately saturating instead of
 * wrapping or relying on the final RGBA conversion to clip them.
 */
static inline int scene_sky_blue(uint8_t scene) {
  int blue = 58 + scene * 18;
  return blue > 255 ? 255 : blue;
}

static inline int scene_floor_green(uint8_t scene) {
  int green = 5 + scene * 8;
  return green > 255 ? 255 : green;
}

/* channel is 0..7; energy is a byte. Callers holding int scene/energy values
 * (decoded from packet bytes, so already 0..255) cast explicitly to uint8_t.
 * Rectangles have exclusive lower-right bounds, in draw order: cap, front,
 * side, highlight.
 */
static inline void channel_meter_rects(int frame, uint8_t scene, int channel,
                                       uint8_t energy, meter_rect_t rects[4]) {
  int height = 12 + energy / 3;
  int depth = channel * 5 + ((frame >> 3) & 9);
  int pitch = 32 + scene;
  /* Full meter width is 34, including the side. Largest integral pitch:
   * (320 - 22 - 34) / 7 = 37. Retain low-scene perspective overlap.
   */
  if (pitch > 37) {
    pitch = 37;
  }
  int x = 22 + channel * pitch;
  int y = 210 - height - depth;

  rects[0] = (meter_rect_t){x + 8, y - 6, x + 34, y};
  rects[1] = (meter_rect_t){x, y, x + 26, y + height};
  rects[2] = (meter_rect_t){x + 26, y + 6, x + 34, y + height + 8};
  rects[3] = (meter_rect_t){x + 3, y + height - 4, x + 23, y + height - 1};
}

#endif
