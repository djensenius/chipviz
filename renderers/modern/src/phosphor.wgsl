// Per-pixel phosphor for the mic layers; see `phosphor.rs`.

struct Decay {
  amount: f32,
  pad0: f32,
  pad1: f32,
  pad2: f32,
};

@group(0) @binding(0)
var emitted: texture_2d<f32>;
@group(0) @binding(1)
var glow: texture_2d<f32>;
@group(0) @binding(2)
var<uniform> decay: Decay;

// Keep in sync with `GRID` in phosphor.rs: the display holds multiples of
// 1 / GRID in 0..=1, which Rgba16Float stores exactly.
const GRID: f32 = 1024.0;

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
  let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
  return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

// out = max(emitted, previous - amount) per channel, in linear light: rises
// are instant, falls at most `amount` (a multiple of 1 / GRID) per frame.
// Keep in sync with `decay_pixel` in phosphor.rs.
@fragment
fn fs_decay(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let pixel = vec2<i32>(position.xy);
  let lit = clamp(textureLoad(emitted, pixel, 0).rgb, vec3<f32>(0.0), vec3<f32>(1.0));
  let quantised = floor(lit * GRID + 0.5) / GRID;
  let held = max(textureLoad(glow, pixel, 0).rgb - vec3<f32>(decay.amount), vec3<f32>(0.0));
  return vec4<f32>(max(quantised, held), 1.0);
}

// Added onto the scene (blend One + One), so the layer's contribution to a
// pixel changes no faster than the display texture.
@fragment
fn fs_composite(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  return vec4<f32>(textureLoad(glow, vec2<i32>(position.xy), 0).rgb, 0.0);
}
