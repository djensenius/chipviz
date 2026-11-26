//! A single six-way corridor junction with world-space flow and a continuous camera path.

use std::sync::OnceLock;

use super::sin_cos;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ProjectedLine {
    pub(crate) start: [f32; 2],
    pub(crate) end: [f32; 2],
    /// Thickness in NDC-y units.
    pub(crate) width: f32,
    pub(crate) intensity: f32,
}

type Point = [f32; 3];
type GridPoint = [i32; 3];
type Edge = [GridPoint; 2];

const HALF_WIDTH: f32 = 1.2;
const REACH: i32 = 5;
const NEAR: f32 = 0.18;
const PERIOD: f32 = 60.0;

// Each cell is a cube; their union is one cross-shaped place, not six
// overlapping closed tunnels. Faces between occupied cells are omitted.
fn occupied(cell: GridPoint) -> bool {
    cell.iter().filter(|&&coordinate| coordinate != 0).count() <= 1
        && cell.iter().all(|coordinate| coordinate.abs() <= REACH)
}

#[derive(Clone, Copy, Debug)]
struct Face {
    axis: usize,
    corners: [GridPoint; 4],
}

fn world_faces() -> &'static [Face] {
    static FACES: OnceLock<Vec<Face>> = OnceLock::new();
    FACES.get_or_init(|| {
        let mut faces = Vec::new();
        for x in -REACH..=REACH {
            for y in -REACH..=REACH {
                for z in -REACH..=REACH {
                    let cell = [x, y, z];
                    if !occupied(cell) {
                        continue;
                    }
                    for axis in 0..3 {
                        for side in [-1, 1] {
                            let mut neighbor = cell;
                            neighbor[axis] += side;
                            if occupied(neighbor) {
                                continue;
                            }
                            let u = (axis + 1) % 3;
                            let v = (axis + 2) % 3;
                            let mut corners = [[0; 3]; 4];
                            for (corner, (a, b)) in
                                corners.iter_mut().zip([(-1, -1), (1, -1), (1, 1), (-1, 1)])
                            {
                                corner[axis] = 2 * cell[axis] + side;
                                corner[u] = 2 * cell[u] + a;
                                corner[v] = 2 * cell[v] + b;
                            }
                            faces.push(Face { axis, corners });
                        }
                    }
                }
            }
        }
        faces
    })
}

fn world_edges() -> &'static [Edge] {
    static EDGES: OnceLock<Vec<Edge>> = OnceLock::new();
    EDGES.get_or_init(|| {
        let mut edges = Vec::new();
        for face in world_faces() {
            for index in 0..4 {
                let mut edge = [face.corners[index], face.corners[(index + 1) % 4]];
                edge.sort_unstable();
                if !edges.contains(&edge) {
                    edges.push(edge);
                }
            }
        }
        edges
    })
}

#[derive(Clone, Copy)]
struct Occluder {
    axis: usize,
    plane: i32,
    low: [i32; 2],
    high: [i32; 2],
}

// Merge coplanar neighboring faces so the numerical tolerance at an
// external silhouette does not open artificial slits at internal cell seams.
fn world_occluders() -> &'static [Occluder] {
    static OCCLUDERS: OnceLock<Vec<Occluder>> = OnceLock::new();
    OCCLUDERS.get_or_init(|| {
        let mut walls: Vec<_> = world_faces()
            .iter()
            .map(|face| {
                let u = (face.axis + 1) % 3;
                let v = (face.axis + 2) % 3;
                Occluder {
                    axis: face.axis,
                    plane: face.corners[0][face.axis],
                    low: [face.corners[0][u], face.corners[0][v]],
                    high: [face.corners[2][u], face.corners[2][v]],
                }
            })
            .collect();
        loop {
            let mut changed = false;
            'search: for i in 0..walls.len() {
                for j in i + 1..walls.len() {
                    let a = walls[i];
                    let b = walls[j];
                    let merge_axis = (0..2).find(|&axis| {
                        let other = 1 - axis;
                        a.axis == b.axis
                            && a.plane == b.plane
                            && a.low[other] == b.low[other]
                            && a.high[other] == b.high[other]
                            && (a.high[axis] == b.low[axis] || b.high[axis] == a.low[axis])
                    });
                    if let Some(axis) = merge_axis {
                        walls[i].low[axis] = a.low[axis].min(b.low[axis]);
                        walls[i].high[axis] = a.high[axis].max(b.high[axis]);
                        walls.remove(j);
                        changed = true;
                        break 'search;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        walls
    })
}

fn world_point(point: GridPoint) -> Point {
    point.map(|coordinate| coordinate as f32 * HALF_WIDTH)
}

// Fixed markings live on the branch surfaces, not on the camera. The two
// sides have different floor guides, the approach has an asymmetric rail,
// and the vertical branches have different wall structures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Branch {
    Left,
    Right,
    Forward,
    Back,
    Up,
    Down,
}

fn world_details() -> &'static [(Branch, [Point; 2])] {
    static DETAILS: OnceLock<Vec<(Branch, [Point; 2])>> = OnceLock::new();
    DETAILS.get_or_init(|| {
        let mut details = Vec::new();
        let mut add = |branch, a, b| details.push((branch, [a, b]));
        let floor = -HALF_WIDTH + 0.01;
        // Left: a pair of floor rails with closely spaced crosspieces.
        for z in [-0.48, 0.48] {
            add(Branch::Left, [-1.4, floor, z], [-10.8, floor, z]);
        }
        for i in 0..10 {
            let x = -2.0 - i as f32 * 0.85;
            add(Branch::Left, [x, floor, -0.48], [x, floor, 0.48]);
        }
        // Right: a center guide and repeated V-shaped floor inlays.
        add(Branch::Right, [1.4, floor, 0.0], [10.8, floor, 0.0]);
        for i in 0..7 {
            let x = 1.8 + i as f32 * 1.25;
            add(Branch::Right, [x, floor, -0.65], [x + 0.55, floor, 0.0]);
            add(Branch::Right, [x + 0.55, floor, 0.0], [x, floor, 0.65]);
        }
        // Matching markings on both z branches, with unequal rail offsets
        // and occasional short crosspieces, identify the traveled route.
        for (branch, sign) in [(Branch::Forward, 1.0), (Branch::Back, -1.0)] {
            for x in [-0.65, 0.30] {
                add(branch, [x, floor, 1.4 * sign], [x, floor, 10.8 * sign]);
            }
            for i in 0..5 {
                let z = (2.0 + i as f32 * 1.8) * sign;
                add(branch, [-0.65, floor, z], [0.30, floor, z]);
            }
        }
        // Above: ladder rungs on the back shaft wall.
        let wall = -HALF_WIDTH + 0.01;
        for x in [-0.48, 0.48] {
            add(Branch::Up, [x, 1.4, wall], [x, 10.8, wall]);
        }
        for i in 0..10 {
            let y = 2.0 + i as f32 * 0.85;
            add(Branch::Up, [-0.48, y, wall], [0.48, y, wall]);
        }
        // Below: a zigzag brace on a different shaft wall.
        let wall = HALF_WIDTH - 0.01;
        for z in [-0.55, 0.55] {
            add(Branch::Down, [wall, -1.4, z], [wall, -10.8, z]);
        }
        for i in 0..9 {
            let y = -1.8 - i as f32;
            let z = if i % 2 == 0 { -0.55 } else { 0.55 };
            add(Branch::Down, [wall, y, z], [wall, y - 1.0, -z]);
        }
        details
    })
}

const FLOW_STREAMS: usize = 3;
const TRACERS_PER_STREAM: usize = 6;
const TRAIL_SEGMENTS: usize = 2;
const MOTION_EDGES: usize = FLOW_STREAMS * TRACERS_PER_STREAM * (TRAIL_SEGMENTS + 1);

fn phase(elapsed: f32, period: f32) -> f32 {
    if elapsed.is_finite() {
        elapsed.rem_euclid(period) / period
    } else {
        0.0
    }
}

// Each circuit visits all six arms, returning through the junction between
// excursions. A cosine out-and-back has zero velocity at both the far turn
// and the central axis change: no teleports or corner cutting through walls.
// The circuits have different periods, lanes and phases, independent of the
// camera's 60-second tour. Audio gently widens the flow, not its clock, so
// changing levels cannot jump a tracer to a different part of its route.
fn flow_point(elapsed: f32, stream: usize, tracer: usize, audio: [f32; 3]) -> Point {
    let period = [73.0, 89.0, 107.0][stream];
    let route =
        (phase(elapsed, period) * 6.0 + tracer as f32 + stream as f32 * 0.37).rem_euclid(6.0);
    let leg = route.floor() as usize;
    let (_, turn) = sin_cos((route - leg as f32) * std::f32::consts::TAU);
    let axis = [2, 0, 1, 2, 0, 1][leg];
    let sign = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0][leg];
    let (sway, drift) = sin_cos(
        phase(elapsed, 31.0 + stream as f32 * 11.0) * std::f32::consts::TAU + tracer as f32 * 0.7,
    );
    let lane = [-0.42, 0.0, 0.42][stream];
    let [energy, bass, treble] = audio;
    let mut point = [
        lane + (0.10 + 0.035 * energy) * sway,
        lane * 0.6 + (0.12 + 0.035 * bass) * drift,
        -lane + (0.10 + 0.035 * treble) * sway,
    ];
    point[axis] += sign * 10.2 * (1.0 - turn) * 0.5;
    point
}

fn moving_edges(elapsed: f32, audio: [f32; 3]) -> Vec<[Point; 2]> {
    // Sanitize before subtracting tail delays, so invalid time is exactly the
    // same frame as zero (including its history), rather than collapsed tails.
    let time = if elapsed.is_finite() { elapsed } else { 0.0 };
    let mut edges = Vec::with_capacity(MOTION_EDGES);
    for stream in 0..FLOW_STREAMS {
        for tracer in 0..TRACERS_PER_STREAM {
            let mut previous = flow_point(time - 0.48, stream, tracer, audio);
            for segment in 1..=TRAIL_SEGMENTS {
                let next = flow_point(
                    time - 0.48 + segment as f32 * (0.48 / TRAIL_SEGMENTS as f32),
                    stream,
                    tracer,
                    audio,
                );
                edges.push([previous, next]);
                previous = next;
            }
            // A small diagonal head stays legible when the trail points into
            // the screen or slows at a turnaround. It remains strictly inside
            // the corridor, and goes through the same wall/near-plane clipping.
            let radius = [0.09, 0.07, 0.08];
            edges.push([
                std::array::from_fn(|i| previous[i] - radius[i]),
                std::array::from_fn(|i| previous[i] + radius[i]),
            ]);
        }
    }
    edges
}

#[derive(Clone, Copy, Debug)]
struct Camera {
    position: Point,
    yaw: f32,
    pitch: f32,
}

// Smoothstep gives zero velocity at each look cue and at the loop seam.
fn cue(time: f32, start: f32, end: f32) -> f32 {
    let t = ((time - start) / (end - start)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn camera(elapsed: f32) -> Camera {
    let time = if elapsed.is_finite() {
        elapsed.rem_euclid(PERIOD)
    } else {
        0.0
    };
    // Capture windows (seconds modulo 60): forward 2–5, left 10–12,
    // right 18–20, back 26–28, up 34–36, down 42–44. Approach the
    // junction, circle inside it while looking around, then retrace the same
    // corridor. Look cues and the loop seam have zero endpoint velocity.
    let approach = cue(time, 0.0, 10.0) - cue(time, 50.0, 60.0);
    let drift = cue(time, 8.0, 10.0) - cue(time, 50.0, 52.0);
    let (orbit_x, orbit_z) = sin_cos((time - 10.0) * std::f32::consts::TAU / 40.0);
    let left = cue(time, 7.0, 10.0) - cue(time, 12.0, 15.0);
    let right = cue(time, 15.0, 18.0) - cue(time, 20.0, 23.0);
    let back = cue(time, 23.0, 26.0) - cue(time, 28.0, 31.0);
    let up = cue(time, 31.0, 34.0) - cue(time, 36.0, 39.0);
    let down = cue(time, 39.0, 42.0) - cue(time, 44.0, 47.0);
    Camera {
        position: [
            0.8 * drift * orbit_x,
            0.12 * drift,
            -5.0 + 5.0 * approach + 0.8 * drift * orbit_z,
        ],
        yaw: 1.18 * (right - left) + (std::f32::consts::PI - 0.18) * back,
        pitch: 1.15 * (up - down),
    }
}

fn camera_space(point: Point, camera: Camera) -> Point {
    let [x, y, z] = std::array::from_fn(|i| point[i] - camera.position[i]);
    let (sy, cy) = sin_cos(camera.yaw);
    let (sp, cp) = sin_cos(camera.pitch);
    let right = cy * x - sy * z;
    let forward = sy * x + cy * z;
    [right, cp * y - sp * forward, sp * y + cp * forward]
}

// Restrict a parameter interval to the nonnegative side of a linear
// inequality whose values at the original segment endpoints are a and b.
fn clip_positive(interval: &mut [f32; 2], a: f32, b: f32) -> bool {
    if a < 0.0 && b < 0.0 {
        return false;
    }
    if a < 0.0 || b < 0.0 {
        let t = a / (a - b);
        if a < 0.0 {
            interval[0] = interval[0].max(t);
        } else {
            interval[1] = interval[1].min(t);
        }
    }
    interval[0] < interval[1]
}

// Each exposed boundary rectangle casts a pyramidal shadow from the camera.
// Its five linear halfspaces clip a world segment to the part behind that opaque
// face. Union those intervals and emit their complement, before projection.
// A target on its supporting face is not behind it; the small plane epsilon
// also protects shared edges from floating-point self-occlusion.
fn visible_segments(edge: [Point; 2], eye: Point) -> Vec<[Point; 2]> {
    const PLANE_EPSILON: f32 = 0.0001;
    let mut shadows: Vec<[f32; 2]> = Vec::new();
    for wall in world_occluders() {
        let axis = wall.axis;
        let u = (axis + 1) % 3;
        let v = (axis + 2) % 3;
        let plane = wall.plane as f32 * HALF_WIDTH;
        let distance = (plane - eye[axis]).abs();
        if distance <= PLANE_EPSILON {
            continue;
        }
        let side = (plane - eye[axis]).signum();
        let q = edge.map(|point| side * (point[axis] - eye[axis]));
        let mut interval = [0.0, 1.0];
        if !clip_positive(
            &mut interval,
            q[0] - distance - PLANE_EPSILON,
            q[1] - distance - PLANE_EPSILON,
        ) {
            continue;
        }
        let mut intersects = true;
        for (index, coordinate) in [u, v].into_iter().enumerate() {
            let low = wall.low[index] as f32 * HALF_WIDTH + PLANE_EPSILON - eye[coordinate];
            let high = wall.high[index] as f32 * HALF_WIDTH - PLANE_EPSILON - eye[coordinate];
            let offset = edge.map(|point| point[coordinate] - eye[coordinate]);
            if !clip_positive(
                &mut interval,
                distance * offset[0] - low * q[0],
                distance * offset[1] - low * q[1],
            ) || !clip_positive(
                &mut interval,
                high * q[0] - distance * offset[0],
                high * q[1] - distance * offset[1],
            ) {
                intersects = false;
                break;
            }
        }
        if intersects {
            if interval[0] <= 0.0 && interval[1] >= 1.0 {
                return Vec::new();
            }
            shadows.push(interval);
        }
    }
    shadows.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let point = |t: f32| std::array::from_fn(|i| edge[0][i] + t * (edge[1][i] - edge[0][i]));
    let mut visible = Vec::new();
    let mut start = 0.0_f32;
    for [low, high] in shadows {
        if low > start + 0.000001 {
            visible.push([point(start), point(low)]);
        }
        start = start.max(high);
    }
    if start < 1.0 - 0.000001 {
        visible.push([point(start), edge[1]]);
    }
    visible
}

// Clip in camera space before division, then clip to the viewport so a line
// crossing the eye or the screen cannot create enormous screen instances.
fn project_edge(
    edge: [Point; 2],
    camera: Camera,
    aspect: f32,
) -> Option<([f32; 2], [f32; 2], f32)> {
    let mut a = camera_space(edge[0], camera);
    let mut b = camera_space(edge[1], camera);
    if a[2] < NEAR && b[2] < NEAR {
        return None;
    }
    if a[2] < NEAR || b[2] < NEAR {
        let t = (NEAR - a[2]) / (b[2] - a[2]);
        let intersection = std::array::from_fn(|i| a[i] + t * (b[i] - a[i]));
        if a[2] < NEAR {
            a = intersection;
        } else {
            b = intersection;
        }
    }
    let depth = (a[2] + b[2]) * 0.5;
    let mut start = [a[0] / (a[2] * aspect), a[1] / a[2]];
    let mut end = [b[0] / (b[2] * aspect), b[1] / b[2]];
    let delta = [end[0] - start[0], end[1] - start[1]];
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    for axis in 0..2 {
        if delta[axis].abs() < 1e-7 {
            if start[axis].abs() > 1.0 {
                return None;
            }
        } else {
            let t0 = (-1.0 - start[axis]) / delta[axis];
            let t1 = (1.0 - start[axis]) / delta[axis];
            low = low.max(t0.min(t1));
            high = high.min(t0.max(t1));
        }
    }
    if low >= high {
        return None;
    }
    end = [start[0] + high * delta[0], start[1] + high * delta[1]];
    start = [start[0] + low * delta[0], start[1] + low * delta[1]];
    if (end[0] - start[0]).hypot(end[1] - start[1]) < 0.001 {
        return None;
    }
    Some((start, end, depth))
}

/// Projects a fixed connected structure and evolving world-space tracers.
/// Audio changes styling and gently widens flow, never the camera or walls.
/// Inputs outside their documented ranges are sanitized to keep instances finite.
pub(crate) fn project(
    elapsed_seconds: f32,
    aspect_ratio: f32,
    audio: [f32; 3],
    max_lines: usize,
) -> Vec<ProjectedLine> {
    if max_lines == 0 {
        return Vec::new();
    }
    let aspect = if aspect_ratio.is_finite() {
        aspect_ratio.clamp(0.1, 10.0)
    } else {
        1.0
    };
    let [energy, bass, treble] = audio.map(|value| {
        if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.0
        }
    });
    let camera = camera(elapsed_seconds);
    let mut visible = Vec::new();
    let mut activity = Vec::new();
    let mut add_edge = |edge: [Point; 2], width, intensity, moving| {
        // Fade the whole moving source edge before either endpoint reaches
        // NEAR. Use unclipped depths: clipping would pin depth to NEAR while
        // magnifying a bright head, then reject it in a single frame. The
        // zero-opacity margin also covers the last near-plane crossing frames.
        let proximity = if moving {
            let closest_depth = edge
                .map(|point| camera_space(point, camera)[2])
                .into_iter()
                .fold(f32::INFINITY, f32::min);
            cue(closest_depth, NEAR + 0.25, NEAR + 1.5)
        } else {
            1.0
        };
        // Fully transparent near-camera flow must not consume the motion
        // budget and displace still-visible tracers farther down a branch.
        if proximity == 0.0 {
            return;
        }
        // A wholly off-screen edge cannot contribute any visible subsegment.
        if project_edge(edge, camera, aspect).is_none() {
            return;
        }
        for segment in visible_segments(edge, camera.position) {
            let Some((start, end, depth)) = project_edge(segment, camera, aspect) else {
                continue;
            };
            let fade = (1.0 / (1.0 + depth * 0.055)).clamp(0.2, 1.0);
            let target = if moving { &mut activity } else { &mut visible };
            target.push((
                depth,
                ProjectedLine {
                    start,
                    end,
                    width,
                    intensity: intensity * fade * proximity,
                },
            ));
        }
    };
    for edge in world_edges() {
        add_edge(
            edge.map(world_point),
            0.003 + 0.001 * bass + 0.0005 * treble,
            0.48 + 0.16 * energy + 0.08 * treble,
            false,
        );
    }
    for &(_, edge) in world_details() {
        add_edge(
            edge,
            0.0045 + 0.001 * bass + 0.0005 * treble,
            0.62 + 0.16 * energy + 0.08 * treble,
            false,
        );
    }
    for edge in moving_edges(elapsed_seconds, [energy, bass, treble]) {
        add_edge(
            edge,
            0.0055 + 0.001 * bass + 0.0005 * treble,
            0.64 + 0.14 * energy + 0.06 * treble,
            true,
        );
    }
    // Preserve structural landmarks while giving flow a quarter of tight
    // budgets. Both layers use stable near-first order, independent of the
    // requested limit, so smaller budgets remain prefixes of larger ones.
    visible.sort_by(|a, b| a.0.total_cmp(&b.0));
    activity.sort_by(|a, b| a.0.total_cmp(&b.0));
    let count = max_lines.min(visible.len() + activity.len());
    let mut structure = visible.into_iter();
    let mut motion = activity.into_iter();
    (0..count)
        .filter_map(|index| {
            if index % 4 == 3 {
                motion.next().or_else(|| structure.next())
            } else {
                structure.next().or_else(|| motion.next())
            }
        })
        .map(|(_, line)| line)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashSet, VecDeque};

    fn distance(a: Point, b: Point) -> f32 {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    }

    // Independent ray/union check: split the ray at the cross's six
    // boundary planes and test each interval against the occupied volume.
    fn ray_inside_world(eye: Point, target: Point) -> bool {
        let mut cuts = vec![0.0_f32, 1.0];
        for axis in 0..3 {
            let delta = target[axis] - eye[axis];
            if delta.abs() < 0.000001 {
                continue;
            }
            for plane in [-13.2, -1.2, 1.2, 13.2] {
                let t = (plane - eye[axis]) / delta;
                if t > 0.0 && t < 1.0 {
                    cuts.push(t);
                }
            }
        }
        cuts.sort_by(f32::total_cmp);
        cuts.windows(2).all(|pair| {
            let t = (pair[0] + pair[1]) * 0.5;
            let p: Point = std::array::from_fn(|i| eye[i] + t * (target[i] - eye[i]));
            p.iter().all(|v| v.abs() <= 13.20001)
                && p.iter().filter(|v| v.abs() > 1.20001).count() <= 1
        })
    }

    #[test]
    fn walls_hide_branch_features_but_openings_reveal_them() {
        let feature = [[-3.7, -1.19, -0.48], [-3.7, -1.19, 0.48]];
        let approach = camera(3.0);
        let (a, b, _) = project_edge(feature, approach, 16.0 / 9.0).unwrap();
        assert!(visible_segments(feature, approach.position).is_empty());
        assert!(
            !project(3.0, 16.0 / 9.0, [0.0; 3], 512)
                .iter()
                .any(|line| line.start == a && line.end == b)
        );

        let junction = camera(11.0);
        let visible = visible_segments(feature, junction.position);
        assert!(!visible.is_empty());
        let junction_lines = project(11.0, 16.0 / 9.0, [0.0; 3], 512);
        let near = |p: [f32; 2], q: [f32; 2]| (p[0] - q[0]).hypot(p[1] - q[1]) < 0.0001;
        assert!(visible.iter().any(|&piece| {
            project_edge(piece, junction, 16.0 / 9.0).is_some_and(|(a, b, _)| {
                junction_lines.iter().any(|line| {
                    (near(line.start, a) && near(line.end, b))
                        || (near(line.start, b) && near(line.end, a))
                })
            })
        }));

        // The near part of the same side rail is seen through the aperture;
        // the remainder is hidden by the approach corridor's solid wall.
        let rail = [[-1.4, -1.19, 0.0], [-10.8, -1.19, 0.0]];
        let pieces = visible_segments(rail, [0.0, 0.0, -5.0]);
        assert_eq!(pieces.len(), 1);
        assert!((pieces[0][0][0] + 1.4).abs() < 0.001);
        assert!(pieces[0][1][0] < -1.5 && pieces[0][1][0] > -1.7);

        // A ray hitting the seam between two coplanar cells still hits a
        // solid wall, rather than leaking through a numerical crack.
        let seam = [[-30.0 / 7.0, -0.2, 0.0], [-30.0 / 7.0, 0.2, 0.0]];
        assert!(visible_segments(seam, [0.0, 0.0, -5.0]).is_empty());

        // A wall's own edge must not disappear, including when the eye is
        // very close to that wall or aligned with a cell boundary.
        let edge = [[-1.2, -1.2, -3.6], [-1.2, -1.2, -6.0]];
        for eye in [[0.0, 0.0, -2.4], [-1.199, -1.199, -2.4]] {
            assert_eq!(visible_segments(edge, eye), vec![edge]);
        }
    }

    #[test]
    fn visibility_agrees_with_independent_union_ray_checks() {
        for eye in [
            [0.0, 0.0, -5.0],
            [0.2, 0.12, 0.3],
            [0.0, 0.0, -2.4],
            [0.8, 0.12, -0.4],
            [-3.0, 0.0, 0.0],
            [0.0, 3.8, 0.0],
            [0.0, -3.8, 0.0],
        ] {
            for edge in world_edges()
                .iter()
                .map(|edge| edge.map(world_point))
                .chain(world_details().iter().map(|(_, edge)| *edge))
                .chain(
                    [0.0, 11.0, 35.0, 43.0]
                        .into_iter()
                        .flat_map(|time| moving_edges(time, [0.25, 0.5, 0.75])),
                )
            {
                let pieces = visible_segments(edge, eye);
                let axis = (0..3)
                    .max_by(|&a, &b| {
                        (edge[1][a] - edge[0][a])
                            .abs()
                            .total_cmp(&(edge[1][b] - edge[0][b]).abs())
                    })
                    .unwrap();
                let parameter =
                    |point: Point| (point[axis] - edge[0][axis]) / (edge[1][axis] - edge[0][axis]);
                for t in [0.13, 0.37, 0.63, 0.87] {
                    let target =
                        std::array::from_fn(|i| edge[0][i] + t * (edge[1][i] - edge[0][i]));
                    let shown = pieces.iter().any(|piece| {
                        t >= parameter(piece[0]) - 0.00001 && t <= parameter(piece[1]) + 0.00001
                    });
                    assert_eq!(
                        shown,
                        ray_inside_world(eye, target),
                        "eye {eye:?}, edge {edge:?}, t {t}"
                    );
                }
            }
        }
    }

    #[test]
    fn topology_is_one_open_six_way_place() {
        let mut visited = HashSet::from([[0, 0, 0]]);
        let mut queue = VecDeque::from([[0, 0, 0]]);
        while let Some(cell) = queue.pop_front() {
            for axis in 0..3 {
                for step in [-1, 1] {
                    let mut next = cell;
                    next[axis] += step;
                    if occupied(next) && visited.insert(next) {
                        queue.push_back(next);
                    }
                }
            }
        }
        assert_eq!(visited.len(), (1 + 6 * REACH) as usize);
        for axis in 0..3 {
            for direction in [-1, 1] {
                let mut end = [0; 3];
                end[axis] = direction * REACH;
                assert!(visited.contains(&end));
                // The shared aperture has perimeter edges, not a blocking
                // face grid across its interior.
                let mut a = [0; 3];
                let mut b = [0; 3];
                a[axis] = direction;
                b[axis] = direction;
                a[(axis + 1) % 3] = -1;
                b[(axis + 1) % 3] = 1;
                assert!(!world_edges().contains(&[a, b]));
                for fixed in [-1, 1] {
                    for along in [(axis + 1) % 3, (axis + 2) % 3] {
                        let other = 3 - axis - along;
                        let mut first = [0; 3];
                        let mut last = [0; 3];
                        first[axis] = direction;
                        last[axis] = direction;
                        first[other] = fixed;
                        last[other] = fixed;
                        first[along] = -1;
                        last[along] = 1;
                        assert!(world_edges().contains(&[first, last]));
                    }
                }
            }
        }
    }

    #[test]
    fn camera_path_and_orientation_are_continuous_and_inside_the_place() {
        for i in -6000..=12000 {
            let time = i as f32 / 100.0;
            let a = camera(time);
            let b = camera(time + 0.01);
            assert!(distance(a.position, b.position) < 0.02);
            assert!((a.yaw - b.yaw).abs() < 0.02);
            assert!((a.pitch - b.pitch).abs() < 0.02);
            assert!(a.position[0].abs() < HALF_WIDTH && a.position[1].abs() < HALF_WIDTH);
            assert!(a.position[2].abs() < (2 * REACH + 1) as f32 * HALF_WIDTH);
            if a.position[0].abs() > 0.001 {
                assert!(
                    a.position[2].abs() < HALF_WIDTH,
                    "junction orbit at {time}s"
                );
            }
            if a.pitch.abs() > 0.0001 {
                assert!(a.position[2].abs() < HALF_WIDTH, "shaft look at {time}s");
            }
        }
        // Check both sides of every cue boundary and the wrap explicitly.
        for time in [
            0.0, 7.0, 8.0, 10.0, 12.0, 15.0, 18.0, 20.0, 23.0, 26.0, 28.0, 31.0, 34.0, 36.0, 39.0,
            42.0, 44.0, 47.0, 50.0, 52.0, 60.0,
        ] {
            let before = camera(time - 0.001);
            let after = camera(time + 0.001);
            assert!(distance(before.position, after.position) < 0.004);
            assert!((before.yaw - after.yaw).abs() < 0.004);
            assert!((before.pitch - after.pitch).abs() < 0.004);

            let before = camera(time - 0.01);
            let at = camera(time);
            let after = camera(time + 0.01);
            let velocity_before =
                std::array::from_fn(|i| (at.position[i] - before.position[i]) / 0.01);
            let velocity_after =
                std::array::from_fn(|i| (after.position[i] - at.position[i]) / 0.01);
            assert!(distance(velocity_before, velocity_after) < 0.05, "{time}s");
            assert!(((after.yaw - 2.0 * at.yaw + before.yaw) / 0.01).abs() < 0.05);
            assert!(((after.pitch - 2.0 * at.pitch + before.pitch) / 0.01).abs() < 0.05);
        }
        // The views are not held at an almost fixed camera point: the camera
        // travels around the junction throughout the look sequence.
        for time in 10..50 {
            assert!(
                distance(
                    camera(time as f32).position,
                    camera(time as f32 + 1.0).position
                ) > 0.12
            );
        }
    }

    #[test]
    fn fixed_branch_landmarks_are_distinct_and_visible() {
        let near = |p: [f32; 2], q: [f32; 2]| (p[0] - q[0]).hypot(p[1] - q[1]) < 0.0001;
        for (branch, start, end) in [
            (Branch::Forward, 2.0, 5.0),
            (Branch::Left, 10.0, 12.0),
            (Branch::Right, 18.0, 20.0),
            (Branch::Back, 26.0, 28.0),
            (Branch::Up, 34.0, 36.0),
            (Branch::Down, 42.0, 44.0),
        ] {
            for time in [start, (start + end) * 0.5, end] {
                let lines = project(time, 16.0 / 9.0, [0.0; 3], 128);
                let mut count = 0;
                let mut length = 0.0;
                for &(_, edge) in world_details().iter().filter(|(kind, _)| {
                    *kind == branch || (branch == Branch::Forward && *kind == Branch::Back)
                }) {
                    if let Some((a, b, _)) = project_edge(edge, camera(time), 16.0 / 9.0)
                        && lines.iter().any(|line| {
                            (near(line.start, a) && near(line.end, b))
                                || (near(line.start, b) && near(line.end, a))
                        })
                    {
                        count += 1;
                        length += (a[0] - b[0]).hypot(a[1] - b[1]);
                    }
                }
                assert!(
                    count >= 3 && length > 0.15,
                    "{branch:?} at {time}s: {count}, {length}"
                );
            }
        }
        // Unlike a rotated copy of the same tunnel, the two side branches
        // and the two shafts have different fixed arrangements.
        let count = |branch| {
            world_details()
                .iter()
                .filter(|(kind, _)| *kind == branch)
                .count()
        };
        assert_ne!(count(Branch::Left), count(Branch::Right));
        assert_ne!(count(Branch::Up), count(Branch::Down));
    }

    #[test]
    fn approach_landmark_remains_visible_on_the_way_back() {
        let camera_before = camera(4.0);
        let camera_after = camera(27.0);
        let visible = |edge, time, camera| {
            let (a, b, _) = project_edge(edge, camera, 16.0 / 9.0)?;
            let lines = project(time, 16.0 / 9.0, [0.0; 3], 512);
            lines
                .iter()
                .find(|line| line.start == a && line.end == b)
                .map(|_| (a, b))
        };
        let shared = world_details()
            .iter()
            .filter(|(branch, _)| *branch == Branch::Back)
            .filter_map(|&(_, edge)| {
                let before = visible(edge, 4.0, camera_before)?;
                let after = visible(edge, 27.0, camera_after)?;
                Some((before, after))
            })
            .collect::<Vec<_>>();
        assert!(shared.len() >= 2);
        assert!(shared.iter().any(|(before, after)| {
            (before.0[0] - after.0[0]).hypot(before.0[1] - after.0[1]) > 0.05
        }));
    }

    #[test]
    fn all_six_branch_depths_are_visible_at_the_look_cues() {
        // Every sample tests four distinct real world edges beyond the
        // aperture, not just a forward vector or the existence of a branch.
        for (name, axis, sign, start, end) in [
            ("forward", 2, 1, 2.0, 5.0),
            ("left", 0, -1, 10.0, 12.0),
            ("right", 0, 1, 18.0, 20.0),
            ("back", 2, -1, 26.0, 28.0),
            ("up", 1, 1, 34.0, 36.0),
            ("down", 1, -1, 42.0, 44.0),
        ] {
            for sample in 0..=8 {
                let time = start + (end - start) * sample as f32 / 8.0;
                let camera = camera(time);
                let mut direction = camera.position;
                direction[axis] += sign as f32;
                assert!(
                    camera_space(direction, camera)[2] > 0.9,
                    "{name} at {time}s"
                );
                // The distant forward branch shares the frame with the
                // nearer approach corridor; the other looks need only 64.
                let budget = if name == "forward" { 128 } else { 64 };
                let lines = project(time, 16.0 / 9.0, [0.0; 3], budget);
                let near = |p: [f32; 2], q: [f32; 2]| (p[0] - q[0]).hypot(p[1] - q[1]) < 0.0001;
                let mut visible = 0;
                for u in [-1, 1] {
                    for v in [-1, 1] {
                        let mut first = [0; 3];
                        let mut last = [0; 3];
                        first[axis] = 3 * sign;
                        last[axis] = 5 * sign;
                        first[(axis + 1) % 3] = u;
                        last[(axis + 1) % 3] = u;
                        first[(axis + 2) % 3] = v;
                        last[(axis + 2) % 3] = v;
                        let mut edge = [first, last];
                        edge.sort_unstable();
                        assert!(world_edges().contains(&edge));
                        if let Some((a, b, _)) =
                            project_edge(edge.map(world_point), camera, 16.0 / 9.0)
                            && (a[0] - b[0]).hypot(a[1] - b[1]) > 0.02
                            && lines.iter().any(|line| {
                                (near(line.start, a) && near(line.end, b))
                                    || (near(line.start, b) && near(line.end, a))
                            })
                        {
                            visible += 1;
                        }
                    }
                }
                assert!(visible >= 2, "{name} at {time}s: {visible} depth edges");
            }
        }
    }

    #[test]
    fn projection_clips_eye_and_viewport() {
        let camera = Camera {
            position: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
        };
        assert!(project_edge([[0.0, 0.0, -2.0], [1.0, 0.0, -1.0]], camera, 1.0).is_none());
        let (a, b, _) = project_edge([[0.0, 0.0, -1.0], [0.2, 0.0, 1.0]], camera, 1.0).unwrap();
        assert!(
            a.into_iter()
                .chain(b)
                .all(|value| value.is_finite() && value.abs() <= 1.00001)
        );
        assert!(project_edge([[10.0, 0.0, 1.0], [11.0, 0.0, 1.0]], camera, 1.0).is_none());
    }

    #[test]
    fn flow_routes_connect_all_six_arms_and_stay_inside_the_world() {
        for audio in [[0.0; 3], [1.0; 3]] {
            for stream in 0..FLOW_STREAMS {
                for tracer in 0..TRACERS_PER_STREAM {
                    let mut visited = HashSet::new();
                    for sample in 0..=720 {
                        let time = sample as f32 * 0.2;
                        let point = flow_point(time, stream, tracer, audio);
                        assert!(point.iter().all(|v| v.is_finite() && v.abs() < 10.8));
                        assert!(point.iter().filter(|v| v.abs() > HALF_WIDTH).count() <= 1);
                        for (axis, coordinate) in point.iter().enumerate() {
                            if coordinate.abs() > 5.0 {
                                visited.insert((axis, coordinate.is_sign_positive()));
                            }
                        }
                        let next = flow_point(time + 0.01, stream, tracer, audio);
                        assert!(ray_inside_world(point, next), "{point:?} -> {next:?}");
                        assert!(distance(point, next) < 0.03);
                    }
                    assert_eq!(visited.len(), 6, "stream {stream}, tracer {tracer}");
                }
            }
            for sample in 0..=240 {
                let edges = moving_edges(sample as f32 * 0.5, audio);
                assert_eq!(edges.len(), MOTION_EDGES);
                for edge in edges {
                    assert!(ray_inside_world(edge[0], edge[1]));
                    assert!(distance(edge[0], edge[1]) < 0.7);
                }
            }
        }
    }

    #[test]
    fn flow_is_smooth_at_turns_axis_changes_and_camera_loop_seams() {
        for stream in 0..FLOW_STREAMS {
            let period = [73.0, 89.0, 107.0][stream];
            for tracer in 0..TRACERS_PER_STREAM {
                for audio in [[0.0; 3], [1.0; 3]] {
                    let mut times = vec![-60.0, 0.0, 60.0, 120.0, period];
                    // Every central axis change and far turnaround.
                    for half_leg in -12..=24 {
                        times.push(
                            (half_leg as f32 * 0.5 - tracer as f32 - stream as f32 * 0.37) * period
                                / 6.0,
                        );
                    }
                    for time in times {
                        let before = flow_point(time - 0.01, stream, tracer, audio);
                        let at = flow_point(time, stream, tracer, audio);
                        let after = flow_point(time + 0.01, stream, tracer, audio);
                        assert!(distance(before, after) < 0.06, "{time}s");
                        let velocity_before = std::array::from_fn(|i| (at[i] - before[i]) / 0.01);
                        let velocity_after = std::array::from_fn(|i| (after[i] - at[i]) / 0.01);
                        assert!(
                            distance(velocity_before, velocity_after) < 0.05,
                            "stream {stream}, tracer {tracer} at {time}s"
                        );
                    }
                }
            }
        }
        // Flow does not restart when the camera completes its tour.
        assert_ne!(moving_edges(0.0, [0.0; 3]), moving_edges(60.0, [0.0; 3]));
    }

    #[test]
    fn activity_moves_with_a_frozen_camera_and_reaches_budgeted_output() {
        for time in [
            2.0, 3.5, 5.0, 10.0, 11.0, 12.0, 18.0, 19.0, 20.0, 26.0, 27.0, 28.0, 34.0, 35.0, 36.0,
            42.0, 43.0, 44.0,
        ] {
            let eye = camera(time);
            for audio in [[0.0; 3], [1.0; 3]] {
                let projected = |time| {
                    moving_edges(time, audio)
                        .into_iter()
                        .flat_map(|edge| visible_segments(edge, eye.position))
                        .filter_map(|edge| project_edge(edge, eye, 16.0 / 9.0))
                        .collect::<Vec<_>>()
                };
                let before = projected(time);
                let after = projected(time + 0.25);
                assert!(!before.is_empty(), "no flow visible at {time}s");
                assert_ne!(before, after, "flow must move without camera displacement");
                // Check the actual caller-budgeted frame contains projected,
                // occluded world flow, not only the unbounded source geometry.
                for budget in [64, 128] {
                    let lines = project(time, 16.0 / 9.0, audio, budget);
                    let width = 0.0055 + 0.001 * audio[1] + 0.0005 * audio[2];
                    let motion: Vec<_> = lines
                        .iter()
                        .filter(|line| line.width == width && line.intensity > 0.01)
                        .collect();
                    assert!(!motion.is_empty(), "{time}s, budget {budget}");
                    for line in motion {
                        assert!(
                            before
                                .iter()
                                .any(|&(a, b, _)| { line.start == a && line.end == b })
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn audio_gently_varies_flow_without_phase_jumps_or_stopping_quiet_motion() {
        for time in [-60.0, 0.0, 11.0, 35.0, 60.0, 107.0] {
            let quiet = moving_edges(time, [0.0; 3]);
            let loud = moving_edges(time, [1.0; 3]);
            assert_ne!(quiet, loud);
            assert_ne!(quiet, moving_edges(time + 0.25, [0.0; 3]));
            assert_ne!(loud, moving_edges(time + 0.25, [1.0; 3]));
            assert_eq!(loud, moving_edges(time, [1.0; 3]));
            let nearby = moving_edges(time, [0.01; 3]);
            for ((a, b), c) in quiet.iter().zip(&loud).zip(&nearby) {
                for endpoint in 0..2 {
                    assert!(distance(a[endpoint], b[endpoint]) < 0.065);
                    assert!(distance(a[endpoint], c[endpoint]) < 0.00065);
                }
            }
        }
    }

    #[test]
    fn moving_head_fades_before_consecutive_near_plane_crossing_frames() {
        let aspect = 16.0 / 9.0;
        // Review repro: stream 1 / tracer 1's diagonal head becomes a large,
        // bright line at frame 518, then both endpoints cross NEAR at 519.
        let head_index = (TRACERS_PER_STREAM + 1) * (TRAIL_SEGMENTS + 1) + TRAIL_SEGMENTS;
        let mut previous_alpha = 0.0_f32;
        let mut peak_alpha = 0.0_f32;
        for frame in 480..=525 {
            let time = frame as f32 / 60.0;
            let edge = moving_edges(time, [0.0; 3])[head_index];
            let projected = project_edge(edge, camera(time), aspect);
            let lines = project(time, aspect, [0.0; 3], 512);
            let line = projected.and_then(|(start, end, _)| {
                lines
                    .iter()
                    .find(|line| line.width == 0.0055 && line.start == start && line.end == end)
            });
            let alpha = line.map_or(0.0, |line| line.intensity);
            if frame > 480 {
                assert!(
                    (alpha - previous_alpha).abs() < 0.05,
                    "frame {frame}: alpha {previous_alpha} -> {alpha}"
                );
            }
            if frame == 518 {
                assert!(projected.is_some(), "repro head must still project at 518");
                assert!(
                    alpha < 0.01,
                    "large near-plane head must already be faded: {line:?}"
                );
            }
            if frame == 519 {
                assert!(
                    projected.is_none(),
                    "repro must exercise near-plane rejection"
                );
            }
            previous_alpha = alpha;
            peak_alpha = peak_alpha.max(alpha);
        }
        assert!(
            peak_alpha > 0.1,
            "the head must fade, not be permanently hidden"
        );
    }

    #[test]
    fn audio_is_deterministic_and_budgets_are_hard_limits() {
        for time in [
            0.0,
            4.0,
            11.0,
            19.0,
            27.0,
            35.0,
            43.0,
            59.99,
            -60.0,
            f32::MAX,
            f32::MIN,
            f32::INFINITY,
            f32::NAN,
        ] {
            for aspect in [0.0, 0.3, 1.0, 16.0 / 9.0, 100.0, f32::INFINITY] {
                let quiet = project(time, aspect, [0.0; 3], 512);
                let loud = project(time, aspect, [1.0; 3], 512);
                assert!(!quiet.is_empty());
                assert!(!loud.is_empty());
                assert_eq!(quiet, project(time, aspect, [0.0; 3], 512));
                assert_eq!(loud, project(time, aspect, [1.0; 3], 512));
                assert_ne!(quiet, loud);
                // Walls and fixed landmarks keep identical projected geometry
                // and still become thicker/brighter, regardless of flow order.
                for a in quiet.iter().filter(|line| line.width < 0.0055) {
                    let b = loud
                        .iter()
                        .find(|line| line.start == a.start && line.end == a.end)
                        .expect("audio must not move or remove structural landmarks");
                    assert!(b.width > a.width && b.intensity > a.intensity);
                }
                // Audio now gently moves tracers, so clipping can change
                // their count/order; all lines retain their styling bounds.
                for a in quiet.iter().chain(&loud) {
                    assert!(
                        a.start
                            .into_iter()
                            .chain(a.end)
                            .all(|v| v.is_finite() && v.abs() <= 1.00001)
                    );
                    assert!(a.width.is_finite() && (0.003..=0.0071).contains(&a.width));
                    assert!((0.0..=1.0).contains(&a.intensity));
                }
                for budget in [0, 1, 2, 7] {
                    let limited = project(time, aspect, [0.0; 3], budget);
                    assert_eq!(limited, quiet[..quiet.len().min(budget)]);
                }
            }
        }
        assert_eq!(
            project(0.0, 1.0, [f32::NAN; 3], 10),
            project(0.0, 1.0, [0.0; 3], 10)
        );
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                project(invalid, 1.0, [0.5; 3], 512),
                project(0.0, 1.0, [0.5; 3], 512)
            );
        }
    }

    #[test]
    fn full_path_visibility_is_finite_repeatable_and_bounded() {
        // The fixed world bounds per-frame work independently of profile:
        // no more than one shadow interval per occluder per source edge.
        assert_eq!(world_faces().len(), 126);
        assert!(world_occluders().len() <= 32);
        assert!(world_edges().len() + world_details().len() <= 320);
        assert_eq!(MOTION_EDGES, 54);
        assert!(world_edges().len() + world_details().len() + MOTION_EDGES <= 374);
        for sample in 0..=120 {
            let time = sample as f32 * 0.5;
            let all = project(time, 16.0 / 9.0, [0.25, 0.5, 0.75], 512);
            assert!(!all.is_empty());
            assert_eq!(all, project(time, 16.0 / 9.0, [0.25, 0.5, 0.75], 512));
            for line in &all {
                assert!(
                    line.start
                        .into_iter()
                        .chain(line.end)
                        .all(|v| v.is_finite() && v.abs() <= 1.00001)
                );
                assert!(line.width.is_finite() && line.width > 0.0);
                assert!((0.0..=1.0).contains(&line.intensity));
            }
            for budget in [1, 7, 64, 128] {
                assert_eq!(
                    project(time, 16.0 / 9.0, [0.25, 0.5, 0.75], budget),
                    all[..all.len().min(budget)]
                );
            }
        }
    }
}
