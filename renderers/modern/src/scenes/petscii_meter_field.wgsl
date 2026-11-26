// The texture contains invented sRGB-encoded bytes, not measured C64 colours.
@group(0) @binding(0)
var pixels: texture_2d<f32>;

struct Geometry {
  origin: vec2<u32>,
  scale_num: u32,
  scale_den: u32,
};

@group(0) @binding(1)
var<uniform> geometry: Geometry;

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
  let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
  return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

fn encoded_pixel(position: vec4<f32>) -> vec4<f32> {
  let pixel = vec2<u32>(position.xy);
  if any(pixel < geometry.origin) {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
  }
  let relative = pixel - geometry.origin;
  let coordinate = ((relative * 2u + vec2<u32>(1u)) * geometry.scale_den) / (2u * geometry.scale_num);
  let dimensions = textureDimensions(pixels);
  if any(coordinate >= dimensions) {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
  }
  return textureLoad(pixels, vec2<i32>(coordinate), 0);
}

@fragment
fn fs_unorm(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  return encoded_pixel(position);
}

@fragment
fn fs_srgb(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let encoded = encoded_pixel(position);
  let high = pow((encoded.rgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
  let low = encoded.rgb / vec3<f32>(12.92);
  let linear = select(high, low, encoded.rgb <= vec3<f32>(0.04045));
  return vec4<f32>(linear, encoded.a);
}
