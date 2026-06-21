struct Uniforms {
  view_projection: mat4x4<f32>,
  time: f32,
  energy: f32,
  beat: f32,
  _padding: f32,
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
};

struct VertexOutput {
  @builtin(position) position: vec4<f32>,
  @location(0) color: vec4<f32>,
  @location(1) local: vec2<f32>,
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
  return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  let edge = max(abs(input.local.x), abs(input.local.y));
  let crisp = select(1.0, 0.78, edge > 0.46);
  let pulse = 1.0 + uniforms.beat * 0.35 + uniforms.energy * 0.18;
  return vec4<f32>(input.color.rgb * crisp * pulse, input.color.a);
}
