//! Perspective linework fixed to the four interior surfaces of a corridor.

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ProjectedLine {
    pub(crate) start: [f32; 2],
    pub(crate) end: [f32; 2],
    /// Thickness in NDC-y units.
    pub(crate) width: f32,
    pub(crate) intensity: f32,
}

const HALF_WIDTH: f32 = 2.4;
const HALF_HEIGHT: f32 = 1.5;
const NEAR: f32 = 0.5;
const FOCAL: f32 = 1.2;
const DEPTH_ROWS: usize = 10;
const SEGMENTS_PER_STROKE: usize = 4;
const EDGE_DEPTHS: [f32; 6] = [0.0, 4.0, 9.0, 18.0, 36.0, 64.0];
const LANES: [f32; 3] = [-0.62, 0.0, 0.62];
const TRANSVERSE_DEPTHS: [[f32; 2]; 4] = [[4.2, 21.0], [13.0, 32.0], [6.4, 25.0], [10.0, 39.0]];
const TRANSVERSE_RUNS: [f32; 4] = [3.0, 3.7, 4.3, 2.8];

#[derive(Clone, Copy, Debug)]
enum Surface {
    Left,
    Right,
    Floor,
    Ceiling,
}

const SURFACES: [Surface; 4] = [
    Surface::Left,
    Surface::Right,
    Surface::Floor,
    Surface::Ceiling,
];

impl Surface {
    fn extent(self) -> f32 {
        match self {
            Self::Left | Self::Right => HALF_HEIGHT,
            Self::Floor | Self::Ceiling => HALF_WIDTH,
        }
    }

    // u runs across a single flat surface; its endpoints are corridor edges.
    fn point(self, u: f32, z: f32) -> [f32; 3] {
        match self {
            Self::Left => [-HALF_WIDTH, u, z],
            Self::Right => [HALF_WIDTH, u, z],
            Self::Floor => [u, -HALF_HEIGHT, z],
            Self::Ceiling => [u, HALF_HEIGHT, z],
        }
    }
}

// Corner seams are shared by adjacent planes. Splitting them in world depth
// keeps the perspective taper well sampled from the viewport to the distance.
fn edge_line(corner: usize, interval: usize) -> [[f32; 3]; 2] {
    let x = if corner.is_multiple_of(2) {
        -HALF_WIDTH
    } else {
        HALF_WIDTH
    };
    let y = if corner < 2 {
        -HALF_HEIGHT
    } else {
        HALF_HEIGHT
    };
    [
        [x, y, EDGE_DEPTHS[interval]],
        [x, y, EDGE_DEPTHS[interval + 1]],
    ]
}

// Interrupted longitudinal paint tracks remain in fixed world coordinates.
// The different phase and length of each lane avoid aligned cross sections.
fn stripe(surface: Surface, surface_index: usize, row: usize, lane: usize) -> [[f32; 3]; 2] {
    let u = surface.extent() * LANES[lane];
    let z = 1.5 + row as f32 * 6.0 + surface_index as f32 * 0.55 + lane as f32 * 0.85;
    let length = 4.0 + lane as f32 * 0.35;
    [surface.point(u, z), surface.point(u, z + length)]
}

// Two deliberately staggered diagonal marks cross each plane, ending at
// its two real corner seams. Different depths on neighbouring surfaces
// keep the marks from joining into a circumferential ring.
fn transverse(surface: Surface, surface_index: usize, mark: usize) -> [[f32; 3]; 2] {
    let z = TRANSVERSE_DEPTHS[surface_index][mark];
    [
        surface.point(-surface.extent(), z),
        surface.point(surface.extent(), z + TRANSVERSE_RUNS[surface_index]),
    ]
}

// Sparse short ticks start exactly on a corner and stop inside that plane.
// They never span a plane, so they cannot make circumferential rings.
fn edge_mark(surface: Surface, surface_index: usize, row: usize) -> [[f32; 3]; 2] {
    let sign = if (surface_index + row).is_multiple_of(2) {
        -1.0
    } else {
        1.0
    };
    let u = surface.extent() * sign;
    let z = 2.0 + row as f32 * 6.0 + surface_index as f32 * 1.35;
    [surface.point(u, z), surface.point(u * 0.55, z + 1.4)]
}

// A bounded 48-second loop drifts along and across the corridor. Its
// movement is visible over a short clip without recycling the fixed marks.
fn camera(elapsed: f32) -> [f32; 3] {
    let phase = elapsed * (std::f32::consts::TAU / 48.0);
    [
        0.55 * (2.0 * phase).sin(),
        0.26 * (2.0 * phase + 0.5).sin(),
        1.5 + 1.4 * phase.sin(),
    ]
}

fn clip_near(a: &mut [f32; 3], b: &mut [f32; 3]) -> bool {
    if a[2] < NEAR && b[2] < NEAR {
        return false;
    }
    if a[2] < NEAR || b[2] < NEAR {
        let t = (NEAR - a[2]) / (b[2] - a[2]);
        let intersection = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, NEAR];
        if a[2] < NEAR {
            *a = intersection;
        } else {
            *b = intersection;
        }
    }
    true
}

// Clip the centreline to the viewport before creating a screen-space quad.
// Return the retained screen-space parameter interval for depth interpolation.
fn clip_screen(a: [f32; 2], b: [f32; 2]) -> Option<(f32, f32)> {
    let mut lo: f32 = 0.0;
    let mut hi: f32 = 1.0;
    for axis in 0..2 {
        let d = b[axis] - a[axis];
        if d.abs() < f32::EPSILON {
            if !(-1.0..=1.0).contains(&a[axis]) {
                return None;
            }
        } else {
            let t0 = (-1.0 - a[axis]) / d;
            let t1 = (1.0 - a[axis]) / d;
            lo = lo.max(t0.min(t1));
            hi = hi.min(t0.max(t1));
        }
    }
    (hi > lo).then_some((lo, hi))
}

struct VisibleStroke {
    start: [f32; 2],
    end: [f32; 2],
    inverse_depth: [f32; 2],
}

fn project_line(world: [[f32; 3]; 2], eye: [f32; 3], aspect: f32) -> Option<VisibleStroke> {
    let mut a = std::array::from_fn(|i| world[0][i] - eye[i]);
    let mut b = std::array::from_fn(|i| world[1][i] - eye[i]);
    if !clip_near(&mut a, &mut b) {
        return None;
    }
    let screen = |p: [f32; 3]| [FOCAL * p[0] / (aspect * p[2]), FOCAL * p[1] / p[2]];
    let start = screen(a);
    let end = screen(b);
    let (lo, hi) = clip_screen(start, end)?;
    let point = |t: f32| {
        [
            start[0] + (end[0] - start[0]) * t,
            start[1] + (end[1] - start[1]) * t,
        ]
    };
    // Reciprocal depth is linear in screen space, including after clipping.
    let inverse_depth = |t: f32| (1.0 - t) / a[2] + t / b[2];
    Some(VisibleStroke {
        start: point(lo),
        end: point(hi),
        inverse_depth: [inverse_depth(lo), inverse_depth(hi)],
    })
}

fn append_stroke(
    stroke: &VisibleStroke,
    thickness: f32,
    intensity: f32,
    max_lines: usize,
    lines: &mut Vec<ProjectedLine>,
) {
    // If the budget is tight, cover the whole stroke with fewer segments
    // rather than leaving its last end floating in the middle of a surface.
    let count = SEGMENTS_PER_STROKE.min(max_lines - lines.len());
    if count == 0 {
        return;
    }
    let point = |t: f32| {
        if t == 0.0 {
            stroke.start
        } else if t == 1.0 {
            stroke.end
        } else {
            [
                stroke.start[0] + (stroke.end[0] - stroke.start[0]) * t,
                stroke.start[1] + (stroke.end[1] - stroke.start[1]) * t,
            ]
        }
    };
    for i in 0..count {
        let lo = i as f32 / count as f32;
        let hi = (i + 1) as f32 / count as f32;
        let midpoint = (lo + hi) * 0.5;
        let inverse_depth =
            (1.0 - midpoint) * stroke.inverse_depth[0] + midpoint * stroke.inverse_depth[1];
        lines.push(ProjectedLine {
            start: point(lo),
            end: point(hi),
            width: FOCAL * thickness * inverse_depth,
            intensity,
        });
    }
}

/// Project fixed surface strokes into screen NDC, bounded by `max_lines`.
pub(crate) fn project(
    elapsed_seconds: f32,
    aspect_ratio: f32,
    audio: [f32; 3],
    max_lines: usize,
) -> Vec<ProjectedLine> {
    let elapsed = if elapsed_seconds.is_finite() {
        elapsed_seconds
    } else {
        0.0
    };
    let aspect = if aspect_ratio.is_finite() && aspect_ratio > 0.0 {
        aspect_ratio.clamp(0.1, 10.0)
    } else {
        1.0
    };
    let [energy, bass, treble] = audio.map(|v| {
        if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        }
    });
    let eye = camera(elapsed);
    let thickness = 0.025 + 0.008 * treble;
    let intensity = 0.35 + 0.20 * energy + 0.10 * bass;
    let mut lines = Vec::new();
    // Visit each corner at every depth before adding surface marks, so even
    // a limited budget favours the corridor's actual perspective boundaries.
    for interval in 0..EDGE_DEPTHS.len() - 1 {
        for corner in 0..4 {
            if lines.len() == max_lines {
                return lines;
            }
            if let Some(stroke) = project_line(edge_line(corner, interval), eye, aspect) {
                append_stroke(&stroke, thickness, intensity, max_lines, &mut lines);
            }
        }
    }
    // Interleave the four surfaces before visiting the more distant marks,
    // making the planes legible without allowing marks to close into hoops.
    for mark in 0..2 {
        for (index, surface) in SURFACES.into_iter().enumerate() {
            if lines.len() == max_lines {
                return lines;
            }
            if let Some(stroke) = project_line(transverse(surface, index, mark), eye, aspect) {
                append_stroke(
                    &stroke,
                    thickness * 1.35,
                    (intensity + 0.12).min(1.0),
                    max_lines,
                    &mut lines,
                );
            }
        }
    }
    for row in 0..DEPTH_ROWS {
        for (index, surface) in SURFACES.into_iter().enumerate() {
            for lane in 0..LANES.len() {
                if lines.len() == max_lines {
                    return lines;
                }
                if let Some(stroke) = project_line(stripe(surface, index, row, lane), eye, aspect) {
                    append_stroke(&stroke, thickness, intensity, max_lines, &mut lines);
                }
            }
            if row.is_multiple_of(3) {
                if lines.len() == max_lines {
                    return lines;
                }
                if let Some(stroke) = project_line(edge_mark(surface, index, row), eye, aspect) {
                    append_stroke(&stroke, thickness, intensity, max_lines, &mut lines);
                }
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 0.00001, "{a} != {b}");
    }

    fn segments(world: [[f32; 3]; 2], eye: [f32; 3], count: usize) -> Vec<ProjectedLine> {
        let stroke = project_line(world, eye, 1.0).unwrap();
        let mut lines = Vec::new();
        append_stroke(&stroke, 0.025, 0.35, count, &mut lines);
        lines
    }

    #[test]
    fn stripes_are_longitudinal_staggered_and_confined_to_planes() {
        let mut stripe_count = 0;
        let mut mark_count = 0;
        for (index, surface) in SURFACES.into_iter().enumerate() {
            for row in 0..DEPTH_ROWS {
                let (fixed, value, u) = match surface {
                    Surface::Left => (0, -HALF_WIDTH, 1),
                    Surface::Right => (0, HALF_WIDTH, 1),
                    Surface::Floor => (1, -HALF_HEIGHT, 0),
                    Surface::Ceiling => (1, HALF_HEIGHT, 0),
                };
                for lane in 0..LANES.len() {
                    let [a, b] = stripe(surface, index, row, lane);
                    stripe_count += 1;
                    close(a[fixed], value);
                    close(b[fixed], value);
                    close(a[u], b[u]);
                    assert!(a[u].abs() < surface.extent());
                    assert!(b[2] > a[2]);
                    if row + 1 < DEPTH_ROWS {
                        assert!(b[2] < stripe(surface, index, row + 1, lane)[0][2]);
                    }
                    if lane + 1 < LANES.len() {
                        assert_ne!(a[2], stripe(surface, index, row, lane + 1)[0][2]);
                    }
                }
                if row.is_multiple_of(3) {
                    let [a, b] = edge_mark(surface, index, row);
                    mark_count += 1;
                    close(a[fixed], value);
                    close(b[fixed], value);
                    close(a[u].abs(), surface.extent());
                    assert!(b[u].abs() < surface.extent());
                    assert_eq!(a[u].signum(), b[u].signum());
                    assert!(b[2] > a[2]);
                }
            }
        }
        // Most markings are longitudinal rather than short corner ticks.
        assert!(stripe_count > 7 * mark_count);
        assert_ne!(
            edge_mark(Surface::Left, 0, 3)[0][2],
            edge_mark(Surface::Floor, 2, 3)[0][2]
        );
    }

    #[test]
    fn transverse_marks_reach_seams_without_forming_hoops() {
        let aspect = 16.0 / 9.0;
        let output = project(2.0, aspect, [0.0; 3], 1000);
        for (index, surface) in SURFACES.into_iter().enumerate() {
            for mark in 0..2 {
                let world = transverse(surface, index, mark);
                let (fixed, value, u) = match surface {
                    Surface::Left => (0, -HALF_WIDTH, 1),
                    Surface::Right => (0, HALF_WIDTH, 1),
                    Surface::Floor => (1, -HALF_HEIGHT, 0),
                    Surface::Ceiling => (1, HALF_HEIGHT, 0),
                };
                for (endpoint, sign) in world.into_iter().zip([-1.0, 1.0]) {
                    close(endpoint[fixed], value);
                    close(endpoint[u], sign * surface.extent());
                    assert!(endpoint[2] >= EDGE_DEPTHS[0]);
                    assert!(endpoint[2] <= EDGE_DEPTHS[EDGE_DEPTHS.len() - 1]);
                    // Any mark on a neighbouring plane meeting this same
                    // corner is separated substantially in world depth.
                    for (other_index, other) in SURFACES.into_iter().enumerate() {
                        if other_index == index {
                            continue;
                        }
                        for other_mark in 0..2 {
                            for other_end in transverse(other, other_index, other_mark) {
                                if endpoint[0] == other_end[0] && endpoint[1] == other_end[1] {
                                    assert!((endpoint[2] - other_end[2]).abs() > 2.0);
                                }
                            }
                        }
                    }
                }
                assert!(world[1][2] > world[0][2]);
                let eye = camera(2.0);
                let stroke = project_line(world, eye, aspect).unwrap();
                for (point, projected) in world.into_iter().zip([stroke.start, stroke.end]) {
                    close(
                        projected[0],
                        FOCAL * (point[0] - eye[0]) / (aspect * (point[2] - eye[2])),
                    );
                    close(
                        projected[1],
                        FOCAL * (point[1] - eye[1]) / (point[2] - eye[2]),
                    );
                }
                let mut parts = Vec::new();
                append_stroke(&stroke, 0.025 * 1.35, 0.35 + 0.12, 4, &mut parts);
                assert!(
                    parts
                        .windows(2)
                        .all(|pair| pair[0].end == pair[1].start && pair[0].width > pair[1].width)
                );
                assert_eq!(
                    output
                        .windows(parts.len())
                        .filter(|window| *window == parts)
                        .count(),
                    1
                );

                let next = project_line(world, camera(2.001), aspect).unwrap();
                assert!((stroke.start[0] - next.start[0]).abs() < 0.001);
                assert!((stroke.start[1] - next.start[1]).abs() < 0.001);
            }
        }
    }

    #[test]
    fn corridor_edges_are_shared_plane_corners_and_continuous() {
        for corner in 0..4 {
            for interval in 0..EDGE_DEPTHS.len() - 1 {
                let [a, b] = edge_line(corner, interval);
                close(a[0].abs(), HALF_WIDTH);
                close(a[1].abs(), HALF_HEIGHT);
                close(a[0], b[0]);
                close(a[1], b[1]);
                assert!(b[2] > a[2]);
                if interval + 1 < EDGE_DEPTHS.len() - 1 {
                    assert_eq!(b, edge_line(corner, interval + 1)[0]);
                }
            }
        }
        let eye = camera(0.0);
        let edge = segments(edge_line(0, 2), eye, SEGMENTS_PER_STROKE);
        assert!(
            project(0.0, 1.0, [0.0; 3], 1000)
                .windows(edge.len())
                .any(|window| window == edge)
        );
    }

    #[test]
    fn projection_has_perspective_and_near_clipping() {
        let eye = [0.0; 3];
        let near = [[0.0, -1.0, 4.0], [1.0, -1.0, 4.0]];
        let far = [[0.0, -1.0, 8.0], [1.0, -1.0, 8.0]];
        let n = segments(near, eye, 1)[0];
        let f = segments(far, eye, 1)[0];
        close(n.end[0], 2.0 * f.end[0]);
        close(n.start[1], 2.0 * f.start[1]);
        close(n.width, 2.0 * f.width);
        let mut a = [0.0, 0.0, 0.1];
        let mut b = [1.0, 0.0, 1.0];
        assert!(clip_near(&mut a, &mut b));
        close(a[2], NEAR);
        close(a[0], 4.0 / 9.0);
        let mut a = [1.0, 0.0, 1.0];
        let mut b = [0.0, 0.0, 0.1];
        assert!(clip_near(&mut a, &mut b));
        close(b[2], NEAR);
        close(b[0], 4.0 / 9.0);
        assert!(!clip_near(&mut [0.0, 0.0, 0.1], &mut [0.0, 0.0, 0.2]));
        let clipped = segments([[-4.0, 0.0, 2.0], [4.0, 0.0, 2.0]], eye, 1)[0];
        close(clipped.start[0], -1.0);
        close(clipped.end[0], 1.0);
    }

    #[test]
    fn one_surface_stroke_tapers_continuously_with_depth() {
        let eye = camera(0.0);
        let world = stripe(Surface::Floor, 2, 1, 0);
        let lines = segments(world, eye, SEGMENTS_PER_STROKE);
        assert_eq!(lines.len(), SEGMENTS_PER_STROKE);
        for pair in lines.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
            assert!(pair[0].width > pair[1].width);
        }
        let stroke = project_line(world, eye, 1.0).unwrap();
        assert_eq!(lines[0].start, stroke.start);
        assert_eq!(lines.last().unwrap().end, stroke.end);
        let near_inverse = 1.0 / (world[0][2] - eye[2]);
        let far_inverse = 1.0 / (world[1][2] - eye[2]);
        close(
            lines[0].width,
            FOCAL * 0.025 * (0.875 * near_inverse + 0.125 * far_inverse),
        );
        close(
            lines[3].width,
            FOCAL * 0.025 * (0.125 * near_inverse + 0.875 * far_inverse),
        );
        assert!(
            project(0.0, 1.0, [0.0; 3], 1000)
                .windows(lines.len())
                .any(|window| window == lines)
        );

        // Even a partially available budget retains both ends of the mark.
        for count in 1..SEGMENTS_PER_STROKE {
            let reduced = segments(world, eye, count);
            assert_eq!(reduced.len(), count);
            assert_eq!(reduced[0].start, stroke.start);
            assert_eq!(reduced.last().unwrap().end, stroke.end);
        }
    }

    #[test]
    fn camera_loops_smoothly_inside_corridor_with_visible_surface_motion() {
        let period = 48.0;
        for i in 0..=2880 {
            let t = i as f32 / 30.0;
            let eye = camera(t);
            assert!(eye[0].abs() < HALF_WIDTH && eye[1].abs() < HALF_HEIGHT);
            assert!(eye[2] > 0.0 && eye[2] < EDGE_DEPTHS[EDGE_DEPTHS.len() - 1]);
            let next = camera(t + 1.0 / 30.0);
            for axis in 0..3 {
                assert!((next[axis] - eye[axis]).abs() < 0.015);
                close(eye[axis], camera(t + period)[axis]);
            }
        }
        let before = camera(-0.001);
        let at = camera(0.0);
        let after = camera(0.001);
        for axis in 0..3 {
            assert!(((at[axis] - before[axis]) - (after[axis] - at[axis])).abs() < 0.00001);
        }
        assert!(camera(6.0)[2] - camera(1.0)[2] > 0.8);
        for t in [1.0, 2.0, 6.0, 8.0, 24.0, 36.0, 48.0] {
            for aspect in [0.1, 16.0 / 9.0, 10.0] {
                for line in project(t, aspect, [0.0; 3], 1000) {
                    assert!(
                        line.start
                            .into_iter()
                            .chain(line.end)
                            .all(|v| v.is_finite() && v.abs() <= 1.00001)
                    );
                    assert!(line.width.is_finite() && line.width > 0.0);
                }
            }
        }

        // Both views project the same fixed floor mark, with a large enough
        // displacement to observe its attachment in a short capture.
        let world = transverse(Surface::Floor, 2, 0);
        let aspect = 16.0 / 9.0;
        let stroke = |t| project_line(world, camera(t), aspect).unwrap();
        let first = stroke(1.0);
        let later = stroke(6.0);
        assert!((first.start[0] - later.start[0]).abs() > 0.1);
        for (t, projected) in [(1.0, first), (6.0, later)] {
            let eye = camera(t);
            close(
                projected.start[0],
                FOCAL * (world[0][0] - eye[0]) / (aspect * (world[0][2] - eye[2])),
            );
            close(
                projected.start[1],
                FOCAL * (world[0][1] - eye[1]) / (world[0][2] - eye[2]),
            );
            let mut parts = Vec::new();
            append_stroke(&projected, 0.025 * 1.35, 0.35 + 0.12, 4, &mut parts);
            assert!(
                project(t, aspect, [0.0; 3], 1000)
                    .windows(parts.len())
                    .any(|window| window == parts)
            );
        }
    }

    #[test]
    fn fixed_marks_follow_continuously_moving_camera() {
        let world = edge_mark(Surface::Floor, 2, 3);
        let at = |t| project_line(world, camera(t), 1.0).unwrap();
        let a = at(2.0);
        let b = at(2.001);
        let c = at(4.0);
        assert_ne!(a.start, c.start);
        assert!((a.start[0] - b.start[0]).abs() < 0.001);
        assert!((a.start[1] - b.start[1]).abs() < 0.001);
        // The returned endpoints are projections of the same world stroke.
        let eye = camera(2.0);
        close(
            a.start[0],
            FOCAL * (world[0][0] - eye[0]) / (world[0][2] - eye[2]),
        );
        close(
            a.start[1],
            FOCAL * (world[0][1] - eye[1]) / (world[0][2] - eye[2]),
        );
        close(
            a.end[0],
            FOCAL * (world[1][0] - eye[0]) / (world[1][2] - eye[2]),
        );
        close(
            a.end[1],
            FOCAL * (world[1][1] - eye[1]) / (world[1][2] - eye[2]),
        );
        let parts = segments(world, eye, SEGMENTS_PER_STROKE);
        assert!(
            project(2.0, 1.0, [0.0; 3], 1000)
                .windows(parts.len())
                .any(|window| window == parts)
        );
    }

    #[test]
    fn budgets_audio_aspects_and_invalid_inputs_are_safe() {
        for aspect in [0.001, 0.5, 1.0, 1.777, 4.0, f32::NAN, f32::INFINITY, -1.0] {
            for budget in [0, 1, 3, 16, 1000] {
                for audio in [[0.0; 3], [1.0; 3], [f32::NAN, f32::INFINITY, -2.0]] {
                    let lines = project(3.0, aspect, audio, budget);
                    assert!(lines.len() <= budget);
                    assert_eq!(lines, project(3.0, aspect, audio, budget));
                    for line in lines {
                        assert!(
                            line.start
                                .into_iter()
                                .chain(line.end)
                                .all(|v| v.is_finite() && v.abs() <= 1.00001)
                        );
                        assert!(line.width.is_finite() && line.width > 0.0);
                        assert!((0.0..=1.0).contains(&line.intensity));
                    }
                }
            }
        }
        let quiet = project(0.0, 1.0, [0.0; 3], 1000);
        let loud = project(0.0, 1.0, [1.0; 3], 1000);
        assert!(!quiet.is_empty());
        for budget in [0, 1, 3, 16] {
            assert_eq!(project(0.0, 1.0, [0.0; 3], budget).len(), budget);
        }
        assert_eq!(quiet.len(), loud.len());
        for (q, l) in quiet.iter().zip(loud) {
            assert_eq!(q.start, l.start);
            assert_eq!(q.end, l.end);
            assert!(l.width > q.width && l.intensity > q.intensity);
        }
        assert_eq!(
            project(f32::NAN, 1.0, [0.0; 3], 10),
            project(0.0, 1.0, [0.0; 3], 10)
        );
    }
}
