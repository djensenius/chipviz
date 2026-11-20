// Prototype DMG shade texture: no filtering or audio-derived parameters.
@group(0) @binding(0)
var shades: texture_2d<f32>;

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> VertexOutput {
  let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
  var output: VertexOutput;
  output.position = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
  // The model's first row is the top of the display.
  output.uv = vec2<f32>(corner.x, 1.0 - corner.y);
  return output;
}

fn encoded_shade(uv: vec2<f32>) -> f32 {
  let dimensions = textureDimensions(shades);
  let pixel = vec2<i32>(clamp(floor(uv * vec2<f32>(dimensions)), vec2<f32>(0.0), vec2<f32>(dimensions - vec2<u32>(1u))));
  return textureLoad(shades, pixel, 0).r;
}

@fragment
fn fs_unorm(input: VertexOutput) -> @location(0) vec4<f32> {
  let encoded = encoded_shade(input.uv);
  return vec4<f32>(vec3<f32>(encoded), 1.0);
}

@fragment
fn fs_srgb(input: VertexOutput) -> @location(0) vec4<f32> {
  let encoded = encoded_shade(input.uv);
  let linear = select(pow((encoded + 0.055) / 1.055, 2.4), encoded / 12.92, encoded <= 0.04045);
  return vec4<f32>(vec3<f32>(linear), 1.0);
}
