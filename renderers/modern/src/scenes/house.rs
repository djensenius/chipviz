//! A single six-way corridor junction, viewed from a continuous camera path.

use std::sync::OnceLock;

use super::{cos, sin_cos};

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
const PERIOD: f32 = 36.0;

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
    let progress = (1.0 - cos(time * std::f32::consts::TAU / PERIOD)) * 0.5;
    let side = cue(time, 6.0, 10.0) - cue(time, 11.0, 14.0);
    let back = cue(time, 14.0, 17.0) - cue(time, 20.0, 23.0);
    let up = cue(time, 22.0, 24.0) - cue(time, 25.0, 28.0);
    Camera {
        position: [0.0, 0.0, -5.0 + 6.0 * progress],
        yaw: -std::f32::consts::FRAC_PI_2 * side + std::f32::consts::PI * back,
        pitch: 1.15 * up,
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
        for i in -3600..=7200 {
            let time = i as f32 / 100.0;
            let a = camera(time);
            let b = camera(time + 0.01);
            assert!(distance(a.position, b.position) < 0.02);
            assert!((a.yaw - b.yaw).abs() < 0.02);
            assert!((a.pitch - b.pitch).abs() < 0.02);
            assert!(a.position[0].abs() < HALF_WIDTH && a.position[1].abs() < HALF_WIDTH);
            assert!(a.position[2].abs() < (2 * REACH + 1) as f32 * HALF_WIDTH);
        }
        assert!(camera(18.0).yaw > 3.0);
        assert!(camera(24.5).pitch > 1.0);
    }

    #[test]
    fn side_and_shaft_depth_edges_are_visible_at_the_look_cues() {
        // Longitudinal edges inside each branch, beyond its shared aperture.
        let side = [[-3.6, 1.2, -1.2], [-6.0, 1.2, -1.2]];
        let shaft = [[1.2, 3.6, -1.2], [1.2, 6.0, -1.2]];
        let traveled = [[-1.2, 1.2, -3.6], [-1.2, 1.2, -6.0]];
        for (time, edge) in [
            (10.5, side),
            (11.0, side),
            (18.0, traveled),
            (24.0, shaft),
            (24.5, shaft),
            (25.0, shaft),
        ] {
            let (a, b, _) =
                project_edge(edge, camera(time), 16.0 / 9.0).expect("branch depth must be visible");
            assert!((a[0] - b[0]).hypot(a[1] - b[1]) > 0.02);
            let near = |p: [f32; 2], q: [f32; 2]| (p[0] - q[0]).hypot(p[1] - q[1]) < 0.0001;
            assert!(
                project(time, 16.0 / 9.0, [0.0; 3], 64)
                    .iter()
                    .any(|line| (near(line.start, a) && near(line.end, b))
                        || (near(line.start, b) && near(line.end, a))),
                "missing branch depth at {time}s"
            );
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
        for time in [0.0, 10.5, 18.0, 24.5, 35.99, f32::NAN] {
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
