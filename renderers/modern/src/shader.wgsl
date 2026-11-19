struct Uniforms {
  view_projection: mat4x4<f32>,
  time: f32,
  energy: f32,
  beat: f32,
  screen_space: f32,
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

struct VertexInput {
  @location(0) position: vec2<f32>,
  @location(1) center: vec3<f32>,
  @location(2) size: vec2<f32>,
  @location(3) color: vec4<f32>,
  @location(4) rotation: f32,
  @location(5) scene_mix: f32,
  @location(6) pattern: vec2<f32>,
};

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) color: vec4<f32>,
  @location(1) local: vec2<f32>,
  @location(2) @interpolate(flat) pattern: vec2<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
  let c = cos(input.rotation);
  let s = sin(input.rotation);
  let local = input.position * input.size;
  let rotated = vec2<f32>(
    local.x * c - local.y * s,
    local.x * s + local.y * c
  );
  let beat_push = uniforms.beat * 0.12;
  let world = vec4<f32>(
    input.center.x + rotated.x,
    input.center.y + rotated.y,
    input.center.z + beat_push,
    1.0
  );

  var output: VertexOutput;
  output.position = uniforms.view_projection * world;
  output.color = input.color;
  output.local = input.position;
  output.pattern = input.pattern;
  return output;
}

// Same integer finalizer as `scenes::hash`.
fn mix32(value: u32) -> u32 {
  var v = value;
  v = v ^ (v >> 16u);
  v = v * 0x7feb352du;
  v = v ^ (v >> 15u);
  v = v * 0x846ca68bu;
  v = v ^ (v >> 16u);
  return v;
}

fn unit(value: u32) -> f32 {
  return f32(value >> 8u) / 16777216.0;
}

// Static glyph: 5x8 bitmap, bit (row * 5 + column), rows 0..3 in pattern.x.
fn glyph_lit(local: vec2<f32>, pattern: vec2<f32>) -> bool {
  let column = u32(clamp(floor((local.x + 0.5) * 5.0), 0.0, 4.0));
  let row = u32(clamp(floor((0.5 - local.y) * 8.0), 0.0, 7.0));
  let half = select(u32(pattern.x + 0.5), u32(pattern.y + 0.5), row >= 4u);
  return ((half >> ((row % 4u) * 5u + column)) & 1u) == 1u;
}

// Static per-pixel noise. pattern.x = -(1 + 2 * rainbow + blank chance);
// pattern.y = clock * 16 + (longest change period - 1), where the clock's low
// 16 bits are the frame counter and the rest reshuffles every pixel per beat.
fn pixel_noise(pixel: vec2<f32>, color: vec4<f32>, pattern: vec2<f32>) -> vec4<f32> {
  let code = -pattern.x - 1.0;
  let rainbow = code >= 2.0;
  let blank = code - select(0.0, 2.0, rainbow);
  let packed = u32(pattern.y + 0.5);
  let longest = (packed & 15u) + 1u;
  let clock = packed >> 4u;
  let cell = vec2<u32>(pixel);
  let seed = mix32(cell.x ^ mix32(cell.y + 0x9e3779b9u));
  let period = 1u + seed % longest;
  let step = ((clock & 0xffffu) + (seed >> 8u) % period) / period;
  let value = mix32(seed ^ mix32(step + (clock >> 16u) * 0x85ebca6bu));
  if (unit(value) < blank) {
    discard;
  }
  let level = unit(mix32(value));
  if (rainbow) {
    let hue = unit(mix32(value ^ 0x27d4eb2fu)) + vec3<f32>(0.0, 2.0 / 3.0, 1.0 / 3.0);
    let rgb = clamp(abs(fract(hue) * 6.0 - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(rgb, color.a * level);
  }
  return vec4<f32>(color.rgb, color.a * level);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  // Flat screen-space fields must tile without seams or global pulsing.
  if (uniforms.screen_space > 0.5) {
    if (input.pattern.x < -0.5) {
      return pixel_noise(input.position.xy, input.color, input.pattern);
    }
    if (input.pattern.x > 0.5 || input.pattern.y > 0.5) {
      if (!glyph_lit(input.local, input.pattern)) {
        discard;
      }
    }
    return input.color;
  }
  let edge = max(abs(input.local.x), abs(input.local.y));
  let crisp = select(1.0, 0.78, edge > 0.46);
  let pulse = 1.0 + uniforms.beat * 0.35 + uniforms.energy * 0.18;
  return vec4<f32>(input.color.rgb * crisp * pulse, input.color.a);
}
