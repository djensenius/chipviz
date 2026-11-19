// Per-pixel phosphor for the mic layers; see `phosphor.rs`.

struct Params {
  // Decay this frame, in whole steps of 1 / GRID.
  steps: f32,
  // 1 when the target is not sRGB, so the composite encodes it itself.
  encode: f32,
  // `RED_SHARE` and `MASK_FROM` from phosphor.rs.
  red_share: f32,
  mask_from: f32,
};

// Binding 0 is the emitted light for the decay pass and the scene copy for
// the composite; binding 1 is the display.
@group(0) @binding(0)
var source: texture_2d<f32>;
@group(0) @binding(1)
var glow: texture_2d<f32>;
@group(0) @binding(2)
var<uniform> params: Params;

// Keep in sync with phosphor.rs: the display holds multiples of 1 / GRID
// in 0..=1, which Rgba8Unorm stores exactly.
const GRID: f32 = 255.0;

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
  let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
  return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

// out = max(emitted, previous - steps) per channel, in linear light and
// grid steps: rises are instant, falls at most `steps` per frame. Then
// green plus blue are lifted to at least a third of red, so the display's
// red share stays at most 3/4, `RED_SHARE` (lifting is never a fall).
// Keep in sync with `decay_pixel` in phosphor.rs.
@fragment
fn fs_decay(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let pixel = vec2<i32>(position.xy);
  let lit = floor(clamp(textureLoad(source, pixel, 0).rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * GRID + 0.5);
  let held = max(round(textureLoad(glow, pixel, 0).rgb * GRID) - vec3<f32>(params.steps), vec3<f32>(0.0));
  let out = max(lit, held);
  // ceil(red / 3) for whole red, robust to division rounding.
  let deficit = max(floor((out.r + 2.5) / 3.0) - out.g - out.b, 0.0);
  let lifted = vec3<f32>(out.r, out.g + ceil(deficit / 2.0), out.b + floor(deficit / 2.0));
  return vec4<f32>(lifted / GRID, 1.0);
}

fn decode(encoded: vec3<f32>) -> vec3<f32> {
  let low = encoded / 12.92;
  let high = pow((encoded + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
  return select(high, low, encoded <= vec3<f32>(0.04045));
}

fn encode(linear: vec3<f32>) -> vec3<f32> {
  let low = linear * 12.92;
  let high = 1.055 * pow(linear, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
  return select(high, low, linear <= vec3<f32>(0.0031308));
}

// The layers' share of a scene pixel (linear): 1 up to a red share of
// `mask_from`, 0 from `red_share`. Keep in sync with `mask` in phosphor.rs.
fn mask(scene: vec3<f32>) -> f32 {
  let sum = scene.r + scene.g + scene.b;
  let share = select(0.0, scene.r / max(sum, 1e-20), sum > 0.0);
  return clamp((params.red_share - share) / (params.red_share - params.mask_from), 0.0, 1.0);
}

// scene + mask(scene) * display, saturating, in linear light; the scene
// unchanged where the display is black. Keep in sync with
// `composite_pixel` in phosphor.rs.
@fragment
fn fs_composite(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let pixel = vec2<i32>(position.xy);
  let stored = textureLoad(source, pixel, 0);
  let display = textureLoad(glow, pixel, 0).rgb;
  if all(display == vec3<f32>(0.0)) {
    return stored;
  }
  let coded = params.encode > 0.5;
  let scene = select(stored.rgb, decode(stored.rgb), coded);
  let out = min(scene + mask(scene) * display, vec3<f32>(1.0));
  return vec4<f32>(select(out, encode(out), coded), stored.a);
}
