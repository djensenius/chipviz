#include "visual_math.h"

#include <stdio.h>
#include <stdlib.h>

static unsigned checks;
static unsigned failures;

static void require(int condition, const char *message) {
  checks++;
  if (!condition) {
    fprintf(stderr, "FAIL: %s\n", message);
    failures++;
  }
}

static int drawable(meter_rect_t r) {
  return r.x0 >= 0 && r.y0 >= 0 && r.x1 <= 320 && r.y1 <= 240 &&
         r.x0 < r.x1 && r.y0 < r.y1;
}

static void meter_bounds(void) {
  int bounded = 1;
  int spaced = 1;
  /* All byte scenes/energies and all four depth-motion phases. */
  static const int frames[] = {0, 8, 64, 72};
  for (int scene = 0; scene <= 255; scene++) {
    for (int energy = 0; energy <= 255; energy++) {
      for (unsigned f = 0; f < sizeof(frames) / sizeof(frames[0]); f++) {
        int previous_x = 0;
        int pitch = 0;
        for (int channel = 0; channel < 8; channel++) {
          meter_rect_t rects[4];
          channel_meter_rects(frames[f], (uint8_t)scene, channel, (uint8_t)energy, rects);
          for (int face = 0; face < 4; face++) {
            bounded &= drawable(rects[face]);
          }
          if (channel == 1) {
            pitch = rects[1].x0 - previous_x;
          }
          if (channel > 0) {
            spaced &= rects[1].x0 - previous_x == pitch && pitch >= 32 && pitch <= 37;
          }
          previous_x = rects[1].x0;
        }
      }
    }
  }
  require(bounded, "all eight complete meters are drawable for every byte scene/energy and depth phase");
  require(spaced, "meters retain uniform bounded pitch, not individually clamped positions");
}

static void reference_geometry(void) {
  int preserved = 1;
  /* Original formula; frames 0..127 cover every (frame >> 3) & 9 phase. */
  for (int scene = 0; scene <= 5; scene++) {
    for (int frame = 0; frame < 128; frame++) {
      for (int energy = 0; energy <= 255; energy++) {
        for (int channel = 0; channel < 8; channel++) {
          meter_rect_t r[4];
          channel_meter_rects(frame, (uint8_t)scene, channel, (uint8_t)energy, r);
          int height = 12 + energy / 3;
          int depth = channel * 5 + ((frame >> 3) & 9);
          int x = 22 + channel * 35 + (scene - 3) * channel;
          int y = 210 - height - depth;
          preserved &= r[0].x0 == x + 8 && r[0].y0 == y - 6 && r[0].x1 == x + 34 && r[0].y1 == y;
          preserved &= r[1].x0 == x && r[1].y0 == y && r[1].x1 == x + 26 && r[1].y1 == y + height;
          preserved &= r[2].x0 == x + 26 && r[2].y0 == y + 6 && r[2].x1 == x + 34 &&
                       r[2].y1 == y + height + 8;
          preserved &= r[3].x0 == x + 3 && r[3].y0 == y + height - 4 && r[3].x1 == x + 23 &&
                       r[3].y1 == y + height - 1;
        }
      }
    }
  }
  require(preserved, "scenes 0..5 preserve all four faces of every meter for every frame phase and energy");
  static const uint8_t scenes[] = {6, 13, 15, 255};
  int repaired = 1;
  for (unsigned s = 0; s < sizeof(scenes) / sizeof(scenes[0]); s++) {
    meter_rect_t r[4];
    channel_meter_rects(0, scenes[s], 7, 0, r);
    repaired &= r[1].x0 == 281 && r[2].x1 == 315;
  }
  require(repaired, "first constrained scene, scene 13, native max and byte max retain the last full meter");
}

static void scene_colors(void) {
  require(scene_sky_blue(0) == 58 && scene_sky_blue(10) == 238 &&
          scene_sky_blue(11) == 255 && scene_sky_blue(13) == 255 &&
          scene_sky_blue(15) == 255 && scene_sky_blue(255) == 255,
          "sky preserves its ramp and deliberately saturates at scene 11");
  require(scene_floor_green(0) == 5 && scene_floor_green(15) == 125 &&
          scene_floor_green(31) == 253 && scene_floor_green(32) == 255 &&
          scene_floor_green(255) == 255,
          "floor preserves native scenes and deliberately saturates at scene 32");
  int bounded = 1;
  for (int scene = 0; scene <= 255; scene++) {
    int sky = scene_sky_blue((uint8_t)scene);
    int floor = scene_floor_green((uint8_t)scene);
    bounded &= sky >= 58 && sky <= 255 && floor >= 5 && floor <= 255;
    if (scene > 0) {
      bounded &= sky >= scene_sky_blue((uint8_t)(scene - 1)) &&
                 floor >= scene_floor_green((uint8_t)(scene - 1));
    }
  }
  require(bounded, "scene color channels are byte-bounded and never wrap");
}

int main(void) {
  meter_bounds();
  reference_geometry();
  scene_colors();
  printf("n64 visual tests: %u checks, %u failures\n", checks, failures);
  return failures ? EXIT_FAILURE : EXIT_SUCCESS;
}
