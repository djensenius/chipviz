// The texture contains invented sRGB-encoded bytes, not measured C64 colours.
@group(0) @binding(0)
var pixels: texture_2d<f32>;

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> VertexOutput {
  let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
  var output: VertexOutput;
  output.position = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
  output.uv = vec2<f32>(corner.x, 1.0 - corner.y);
  return output;
}

fn encoded_pixel(uv: vec2<f32>) -> vec4<f32> {
  let dimensions = textureDimensions(pixels);
  let coordinate = vec2<i32>(clamp(floor(uv * vec2<f32>(dimensions)), vec2<f32>(0.0), vec2<f32>(dimensions - vec2<u32>(1u))));
  return textureLoad(pixels, coordinate, 0);
}

@fragment
fn fs_unorm(input: VertexOutput) -> @location(0) vec4<f32> {
  return encoded_pixel(input.uv);
}

@fragment
fn fs_srgb(input: VertexOutput) -> @location(0) vec4<f32> {
  let encoded = encoded_pixel(input.uv);
  let high = pow((encoded.rgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
  let low = encoded.rgb / vec3<f32>(12.92);
  let linear = select(high, low, encoded.rgb <= vec3<f32>(0.04045));
  return vec4<f32>(linear, encoded.a);
}
