struct Parameters {
  values: vec4<f32>,
};

@group(0) @binding(0)
var scene_image: texture_2d<f32>;
@group(0) @binding(1)
var previous_image: texture_2d<f32>;
@group(0) @binding(2)
var<uniform> parameters: Parameters;

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
};

struct DualOutput {
  @location(0) surface: vec4<f32>,
  @location(1) history: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
  var positions = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(3.0, -1.0),
    vec2<f32>(-1.0, 3.0)
  );
  var output: VertexOutput;
  output.position = vec4<f32>(positions[index], 0.0, 1.0);
  return output;
}

fn srgb_to_linear(value: vec3<f32>) -> vec3<f32> {
  let low = value / 12.92;
  let high = pow((value + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
  return select(high, low, value <= vec3<f32>(0.04045));
}

fn linear_to_srgb(value: vec3<f32>) -> vec3<f32> {
  let low = value * 12.92;
  let high = 1.055 * pow(max(value, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
  return select(high, low, value <= vec3<f32>(0.0031308));
}

fn output_pixel(position: vec4<f32>) -> vec4<f32> {
  let pixel = vec2<i32>(position.xy);
  var target = textureLoad(scene_image, pixel, 0).rgb;
  let srgb_target = parameters.values.z > 0.5;
  if (!srgb_target) {
    target = srgb_to_linear(target);
  }
  var locked = parameters.values.w > 0.5;
  if (parameters.values.y > 0.5) {
    let previous = textureLoad(previous_image, pixel, 0);
    if (locked || previous.a > 0.5) {
      let delta = vec3<f32>(parameters.values.x);
      let bounded = clamp(target, previous.rgb - delta, previous.rgb + delta);
      if (locked || any(abs(target - bounded) > vec3<f32>(0.000001))) {
        target = bounded;
        locked = true;
      }
    }
  }
  return vec4<f32>(target, select(0.0, 1.0, locked));
}

@fragment
fn fs_single(input: VertexOutput) -> @location(0) vec4<f32> {
  return output_pixel(input.position);
}

@fragment
fn fs_dual(input: VertexOutput) -> DualOutput {
  let history = output_pixel(input.position);
  var surface = history.rgb;
  if (parameters.values.z <= 0.5) {
    surface = linear_to_srgb(surface);
  }
  return DualOutput(vec4<f32>(surface, 1.0), history);
}
