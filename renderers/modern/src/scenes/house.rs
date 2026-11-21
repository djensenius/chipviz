//! A single six-way corridor junction, viewed from a continuous camera path.

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

fn world_edges() -> &'static [Edge] {
    static EDGES: OnceLock<Vec<Edge>> = OnceLock::new();
    EDGES.get_or_init(|| {
        let mut edges = Vec::new();
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
                            for index in 0..4 {
                                let mut edge = [corners[index], corners[(index + 1) % 4]];
                                edge.sort_unstable();
                                if !edges.contains(&edge) {
                                    edges.push(edge);
                                }
                            }
                        }
                    }
                }
            }
        }
        edges
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

/// Projects the same world at every time. Audio only changes line styling;
/// it never moves the camera or geometry. Inputs outside their documented
/// ranges are sanitized to keep the returned instances finite.
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
    for edge in world_edges() {
        if let Some((start, end, depth)) = project_edge(edge.map(world_point), camera, aspect) {
            let fade = (1.0 / (1.0 + depth * 0.055)).clamp(0.2, 1.0);
            visible.push((
                depth,
                ProjectedLine {
                    start,
                    end,
                    width: 0.003 + 0.001 * bass + 0.0005 * treble,
                    intensity: (0.48 + 0.16 * energy + 0.08 * treble) * fade,
                },
            ));
        }
    }
    for &(_, edge) in world_details() {
        if let Some((start, end, depth)) = project_edge(edge, camera, aspect) {
            let fade = (1.0 / (1.0 + depth * 0.055)).clamp(0.2, 1.0);
            visible.push((
                depth,
                ProjectedLine {
                    start,
                    end,
                    width: 0.0045 + 0.001 * bass + 0.0005 * treble,
                    intensity: (0.62 + 0.16 * energy + 0.08 * treble) * fade,
                },
            ));
        }
    }
    // Close structure wins tight budgets; stable sorting preserves ties.
    visible.sort_by(|a, b| a.0.total_cmp(&b.0));
    visible
        .into_iter()
        .take(max_lines)
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
    fn audio_is_deterministic_and_budgets_are_hard_limits() {
        for time in [0.0, 4.0, 11.0, 19.0, 27.0, 35.0, 43.0, 59.99, f32::NAN] {
            for aspect in [0.0, 0.3, 1.0, 16.0 / 9.0, 100.0, f32::INFINITY] {
                let quiet = project(time, aspect, [0.0; 3], 512);
                let loud = project(time, aspect, [1.0; 3], 512);
                assert!(!quiet.is_empty());
                assert_eq!(quiet, project(time, aspect, [0.0; 3], 512));
                assert_eq!(quiet.len(), loud.len());
                for (a, b) in quiet.iter().zip(&loud) {
                    assert_eq!((a.start, a.end), (b.start, b.end));
                    assert!(b.width > a.width && b.intensity > a.intensity);
                    assert!(
                        a.start
                            .into_iter()
                            .chain(a.end)
                            .all(|v| v.is_finite() && v.abs() <= 1.00001)
                    );
                    assert!((0.0..=1.0).contains(&a.intensity));
                    assert!((0.0..=1.0).contains(&b.intensity));
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
    }
}
