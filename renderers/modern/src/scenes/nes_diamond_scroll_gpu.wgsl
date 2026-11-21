struct Params {
  // x, y, width, height in output pixels
  viewport: vec4<u32>,
  // x is 1 for an sRGB attachment; remaining components are padding
  flags: vec4<u32>,
};

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
  let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
  return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

fn decode_srgb(encoded: vec3<f32>) -> vec3<f32> {
  let low = encoded / 12.92;
  let high = pow((encoded + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
  return select(high, low, encoded <= vec3<f32>(0.04045));
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let xy = vec2<u32>(position.xy);
  let origin = params.viewport.xy;
  let size = params.viewport.zw;
  if any(xy < origin) || any(xy >= origin + size) {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
  }
  // Exact nearest-neighbour source coordinates even when downsampling.
  let source_xy = (xy - origin) * vec2<u32>(256u, 240u) / size;
  let encoded = textureLoad(source, vec2<i32>(source_xy), 0).rgb;
  if params.flags.x != 0u {
    return vec4<f32>(decode_srgb(encoded), 1.0);
  }
  return vec4<f32>(encoded, 1.0);
}
