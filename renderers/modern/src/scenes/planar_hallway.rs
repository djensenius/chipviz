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
const DEPTH_ROWS: usize = 22;

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

// Each row contains an oblique stroke ending at surface edges and a shorter
// longitudinal stroke. Staggering the surfaces prevents rectangular hoops.
fn world_line(surface: Surface, surface_index: usize, row: usize, across: bool) -> [[f32; 3]; 2] {
    let z = 1.5 + row as f32 * 2.6 + surface_index as f32 * 0.38;
    let extent = surface.extent();
    if across {
        [surface.point(-extent, z), surface.point(extent, z + 1.4)]
    } else {
        let u = extent * if row % 2 == 0 { -0.38 } else { 0.38 };
        [surface.point(u, z + 0.3), surface.point(u, z + 2.3)]
    }
}

// A bounded, continuous camera path avoids recycling geometry past the
// viewer. The painted marks remain fixed in world coordinates.
fn camera(elapsed: f32) -> [f32; 3] {
    [
        0.22 * (elapsed * 0.19).sin(),
        0.12 * (elapsed * 0.13).sin(),
        1.1 + 0.6 * (elapsed * 0.11).sin(),
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

fn project_line(
    world: [[f32; 3]; 2],
    eye: [f32; 3],
    aspect: f32,
    thickness: f32,
    intensity: f32,
) -> Option<ProjectedLine> {
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
    // Reciprocal depth is linear in screen space; use the visible midpoint
    // so distant marks stay narrow even when a segment crosses the viewport.
    let midpoint = (lo + hi) * 0.5;
    let inverse_depth = (1.0 - midpoint) / a[2] + midpoint / b[2];
    Some(ProjectedLine {
        start: point(lo),
        end: point(hi),
        width: FOCAL * thickness * inverse_depth,
        intensity,
    })
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
    for row in 0..DEPTH_ROWS {
        for (index, surface) in SURFACES.into_iter().enumerate() {
            for across in [true, false] {
                if lines.len() == max_lines {
                    return lines;
                }
                if let Some(line) = project_line(
                    world_line(surface, index, row, across),
                    eye,
                    aspect,
                    thickness,
                    intensity,
                ) {
                    lines.push(line);
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

    #[test]
    fn strokes_are_on_their_planes_and_terminate_at_edges() {
        for (index, surface) in SURFACES.into_iter().enumerate() {
            for row in 0..DEPTH_ROWS {
                for across in [true, false] {
                    let [a, b] = world_line(surface, index, row, across);
                    let (fixed, value, u) = match surface {
                        Surface::Left => (0, -HALF_WIDTH, 1),
                        Surface::Right => (0, HALF_WIDTH, 1),
                        Surface::Floor => (1, -HALF_HEIGHT, 0),
                        Surface::Ceiling => (1, HALF_HEIGHT, 0),
                    };
                    close(a[fixed], value);
                    close(b[fixed], value);
                    assert!(a[u].abs() <= surface.extent());
                    assert!(b[u].abs() <= surface.extent());
                    if across {
                        close(a[u], -surface.extent());
                        close(b[u], surface.extent());
                        assert_ne!(a[2], b[2]);
                    } else {
                        close(a[u], b[u]);
                    }
                }
            }
        }
        // Adjacent surface strokes do not close into depth-aligned hoops.
        assert_ne!(
            world_line(Surface::Left, 0, 5, true)[0][2],
            world_line(Surface::Floor, 2, 5, true)[0][2]
        );
    }

    #[test]
    fn projection_has_perspective_and_near_clipping() {
        let eye = [0.0; 3];
        let near = [[0.0, -1.0, 4.0], [1.0, -1.0, 4.0]];
        let far = [[0.0, -1.0, 8.0], [1.0, -1.0, 8.0]];
        let n = project_line(near, eye, 1.0, 0.025, 0.5).unwrap();
        let f = project_line(far, eye, 1.0, 0.025, 0.5).unwrap();
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
        let clipped =
            project_line([[-4.0, 0.0, 2.0], [4.0, 0.0, 2.0]], eye, 1.0, 0.025, 0.5).unwrap();
        close(clipped.start[0], -1.0);
        close(clipped.end[0], 1.0);
    }

    #[test]
    fn fixed_marks_follow_continuously_moving_camera() {
        let world = world_line(Surface::Floor, 2, 5, true);
        let at = |t| project_line(world, camera(t), 1.0, 0.025, 0.35).unwrap();
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
        assert!(project(2.0, 1.0, [0.0; 3], 1000).contains(&a));
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
            assert_eq!(project(0.0, 1.0, [0.0; 3], budget), quiet[..budget]);
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
