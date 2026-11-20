//! Procedural models for the Object scene.
//!
//! Source and licence: every model below is an original construction written
//! in code for chipviz (boxes, cylinders and spheres painted by functions; no
//! downloaded meshes or textures, and lettering uses chipviz's own
//! hand-drawn 5x8 font from the Static scene), so it is covered by the
//! repository licence.
//!
//! A model is a cloud of surface splats: each primitive samples its surface
//! on a regular grid whose spacing (`step`) is chosen per profile from the
//! instance budget, so the Pi 5 draws the same shapes with fewer, larger
//! splats. Models sit within about one unit of the origin, front facing +z.

use super::super::static_noise::font::glyph;
use super::super::{cos, hash, sin_cos};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

pub(super) type Vec3 = [f32; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Surface {
    pub(super) color: Vec3,
    /// Specular highlight strength, 0..1.
    pub(super) shine: f32,
    /// Emissive share, 0 (fully lit) to 1 (ignores lighting).
    pub(super) glow: f32,
}

const fn matte(color: Vec3) -> Surface {
    Surface {
        color,
        shine: 0.0,
        glow: 0.0,
    }
}

const fn gloss(color: Vec3, shine: f32) -> Surface {
    Surface {
        color,
        shine,
        glow: 0.0,
    }
}

const fn lit(color: Vec3, glow: f32) -> Surface {
    Surface {
        color,
        shine: 0.5,
        glow,
    }
}

/// One surface sample: position, outward normal and the two grid edges
/// (`du`, `dv`) it covers, all in object space.
#[derive(Clone, Copy, Debug)]
pub(super) struct Splat {
    pub(super) pos: Vec3,
    pub(super) normal: Vec3,
    pub(super) du: Vec3,
    pub(super) dv: Vec3,
    pub(super) surface: Surface,
}

/// What a paint function sees: the sample in primitive-local coordinates.
pub(super) struct Hit {
    pub(super) local: Vec3,
    pub(super) normal: Vec3,
    /// 0..1 across the face (boxes: x then y on z faces, z then y on x
    /// faces, x then z on y faces; cylinders: angle then height or radius).
    pub(super) uv: [f32; 2],
}

/// A primitive's origin and axes (columns) in object space.
#[derive(Clone, Copy, Debug)]
struct Place {
    origin: Vec3,
    axes: [Vec3; 3],
}

fn at(origin: Vec3) -> Place {
    Place {
        origin,
        axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    }
}

impl Place {
    /// Rotates the axes by `angle` about local axis `axis` (right-handed).
    fn turned(mut self, axis: usize, angle: f32) -> Self {
        let (i, j) = ((axis + 1) % 3, (axis + 2) % 3);
        let (s, c) = sin_cos(angle);
        let (a, b) = (self.axes[i], self.axes[j]);
        self.axes[i] = std::array::from_fn(|k| a[k] * c + b[k] * s);
        self.axes[j] = std::array::from_fn(|k| b[k] * c - a[k] * s);
        self
    }

    fn dir(&self, v: Vec3) -> Vec3 {
        std::array::from_fn(|k| {
            self.axes[0][k] * v[0] + self.axes[1][k] * v[1] + self.axes[2][k] * v[2]
        })
    }

    fn point(&self, v: Vec3) -> Vec3 {
        let d = self.dir(v);
        std::array::from_fn(|k| self.origin[k] + d[k])
    }
}

pub(super) struct Mesh {
    step: f32,
    pub(super) splats: Vec<Splat>,
}

fn steps(length: f32, step: f32) -> usize {
    (length / step).ceil().max(1.0) as usize
}

impl Mesh {
    pub(super) fn new(step: f32) -> Self {
        Self {
            step,
            splats: Vec::new(),
        }
    }

    fn push(&mut self, place: Place, hit: &Hit, edges: [Vec3; 2], surface: Surface) {
        self.splats.push(Splat {
            pos: place.point(hit.local),
            normal: place.dir(hit.normal),
            du: place.dir(edges[0]),
            dv: place.dir(edges[1]),
            surface,
        });
    }

    fn cuboid(&mut self, place: Place, half: Vec3, paint: impl Fn(&Hit) -> Surface) {
        for (axis, (a, b)) in [(0, (2, 1)), (1, (0, 2)), (2, (0, 1))] {
            let (nu, nv) = (
                steps(2.0 * half[a], self.step),
                steps(2.0 * half[b], self.step),
            );
            let (su, sv) = (2.0 * half[a] / nu as f32, 2.0 * half[b] / nv as f32);
            let (mut du, mut dv) = ([0.0; 3], [0.0; 3]);
            du[a] = su;
            dv[b] = sv;
            for sign in [-1.0, 1.0] {
                let mut normal = [0.0; 3];
                normal[axis] = sign;
                for i in 0..nu {
                    for j in 0..nv {
                        let mut local = [0.0; 3];
                        local[axis] = sign * half[axis];
                        local[a] = -half[a] + (i as f32 + 0.5) * su;
                        local[b] = -half[b] + (j as f32 + 0.5) * sv;
                        let uv = [(i as f32 + 0.5) / nu as f32, (j as f32 + 0.5) / nv as f32];
                        let hit = Hit { local, normal, uv };
                        self.push(place, &hit, [du, dv], paint(&hit));
                    }
                }
            }
        }
    }

    /// A cylinder along local y; the caps are rings from `inner` to `radius`.
    fn cylinder(
        &mut self,
        place: Place,
        [radius, inner, half]: [f32; 3],
        paint: impl Fn(&Hit) -> Surface,
    ) {
        let around = steps(TAU * radius, self.step).max(8);
        let tall = steps(2.0 * half, self.step);
        let height = 2.0 * half / tall as f32;
        for i in 0..around {
            let angle = TAU * (i as f32 + 0.5) / around as f32;
            let (s, c) = sin_cos(angle);
            let arc = TAU * radius / around as f32;
            for j in 0..tall {
                let y = -half + (j as f32 + 0.5) * height;
                let hit = Hit {
                    local: [c * radius, y, s * radius],
                    normal: [c, 0.0, s],
                    uv: [angle / TAU, (j as f32 + 0.5) / tall as f32],
                };
                let edges = [[-s * arc, 0.0, c * arc], [0.0, height, 0.0]];
                self.push(place, &hit, edges, paint(&hit));
            }
        }
        let rings = steps(radius - inner, self.step);
        let dr = (radius - inner) / rings as f32;
        for ring in 0..rings {
            let r = inner + (ring as f32 + 0.5) * dr;
            let count = steps(TAU * r, self.step).max(6);
            let arc = TAU * r / count as f32;
            for i in 0..count {
                let angle = TAU * (i as f32 + 0.5) / count as f32;
                let (s, c) = sin_cos(angle);
                for sign in [-1.0, 1.0] {
                    let hit = Hit {
                        local: [c * r, sign * half, s * r],
                        normal: [0.0, sign, 0.0],
                        uv: [angle / TAU, r / radius],
                    };
                    let edges = [[-s * arc, 0.0, c * arc], [c * dr, 0.0, s * dr]];
                    self.push(place, &hit, edges, paint(&hit));
                }
            }
        }
    }

    fn sphere(&mut self, place: Place, radius: f32, paint: impl Fn(&Hit) -> Surface) {
        let rows = steps(PI * radius, self.step).max(4);
        let dv = PI * radius / rows as f32;
        for row in 0..rows {
            let polar = PI * (row as f32 + 0.5) / rows as f32;
            let (sp, cp) = sin_cos(polar);
            let count = steps(TAU * radius * sp, self.step).max(3);
            let arc = TAU * radius * sp / count as f32;
            for i in 0..count {
                let angle = TAU * (i as f32 + 0.5) / count as f32;
                let (s, c) = sin_cos(angle);
                let normal = [sp * c, cp, sp * s];
                let hit = Hit {
                    local: normal.map(|axis| axis * radius),
                    normal,
                    uv: [angle / TAU, 1.0 - row as f32 / rows as f32],
                };
                let edges = [
                    [-s * arc, 0.0, c * arc],
                    [cp * c * dv, -sp * dv, cp * s * dv],
                ];
                self.push(place, &hit, edges, paint(&hit));
            }
        }
    }
}

/// Audio and time inputs the paint functions animate with.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Look {
    /// Frames since the section started.
    pub(super) t: f32,
    pub(super) spectrum: [f32; 8],
    pub(super) mid: f32,
    pub(super) treble: f32,
    pub(super) energy: f32,
    /// Beat punch, 1 at the onset decaying to 0.
    pub(super) beat: f32,
    /// The turntable angle, for view-dependent sheen.
    pub(super) yaw: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Television,
    VideoTape,
    CompactDisc,
    Sun,
    Tree,
    Computer,
}

impl Kind {
    pub(super) const ALL: [Self; 6] = [
        Self::Television,
        Self::VideoTape,
        Self::CompactDisc,
        Self::Sun,
        Self::Tree,
        Self::Computer,
    ];

    /// Banner caption in the infomercial overlay.
    pub(super) fn caption(self) -> &'static str {
        match self {
            Self::Television => "COLOR TELEVISION",
            Self::VideoTape => "VHS TAPE",
            Self::CompactDisc => "COMPACT DISC",
            Self::Sun => "THE SUN",
            Self::Tree => "A TREE",
            Self::Computer => "HOME COMPUTER",
        }
    }

    /// Presentation scale, so every model fills a similar share of the frame.
    pub(super) fn size(self) -> f32 {
        match self {
            Self::Television => 0.82,
            Self::Computer => 0.92,
            _ => 1.0,
        }
    }

    /// Flat objects (and the sun's face) rock instead of turning all the way.
    pub(super) fn rocks(self) -> bool {
        matches!(self, Self::CompactDisc | Self::Sun)
    }

    pub(super) fn build(self, mesh: &mut Mesh, look: &Look) {
        match self {
            Self::Television => television(mesh, look),
            Self::VideoTape => video_tape(mesh, look),
            Self::CompactDisc => compact_disc(mesh, look),
            Self::Sun => sun(mesh, look),
            Self::Tree => tree(mesh, look),
            Self::Computer => computer(mesh, look),
        }
    }
}

/// Fully saturated colour for hue `h` (wraps every 1.0).
pub(super) fn hue(h: f32) -> Vec3 {
    [0.0, 2.0 / 3.0, 1.0 / 3.0]
        .map(|offset: f32| (((h + offset).rem_euclid(1.0) * 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0))
}

fn mix3(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

fn scale3(a: Vec3, s: f32) -> Vec3 {
    a.map(|v| v * s)
}

/// Stable 0..1 noise for a position quantised to `cell`.
fn speckle(p: Vec3, cell: f32, salt: u32) -> f32 {
    let q = p.map(|v| (v / cell).floor() as i32 as u32);
    hash(
        q[0].wrapping_mul(73_856_093)
            ^ q[1].wrapping_mul(19_349_663)
            ^ q[2].wrapping_mul(83_492_791)
            ^ salt,
    )
}

/// Whether text `line` covers (`u`, `v`) of its box (0..1, v up), drawn
/// with the Static scene's 5x8 font and a one-pixel gap after each letter.
fn text_lit(line: &str, u: f32, v: f32) -> bool {
    let count = line.chars().count();
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) || count == 0 {
        return false;
    }
    let x = u * count as f32 * 6.0;
    let (column, row) = (x as usize % 6, ((1.0 - v) * 8.0) as usize);
    let character = line.chars().nth(x as usize / 6).unwrap_or(' ');
    column < 5 && glyph(character).is_some_and(|bits| bits >> (row.min(7) * 5 + column) & 1 == 1)
}

const CHROME: Surface = gloss([0.75, 0.76, 0.8], 1.0);
const BLACK_PLASTIC: Surface = gloss([0.05, 0.05, 0.06], 0.45);
const BEIGE: Vec3 = [0.8, 0.74, 0.6];

/// Test-card bars, plus orange for the eighth band.
const BARS: [Vec3; 8] = [
    [0.9, 0.9, 0.9],
    [0.9, 0.9, 0.1],
    [0.1, 0.9, 0.9],
    [0.1, 0.9, 0.1],
    [0.9, 0.1, 0.9],
    [0.9, 0.1, 0.1],
    [0.1, 0.1, 0.9],
    [0.9, 0.5, 0.1],
];

/// A wood-cabinet CRT set with rabbit ears; the screen shows colour bars
/// whose heights follow the eight spectrum bands.
fn television(mesh: &mut Mesh, look: &Look) {
    let wood = |p: Vec3| {
        let grain = 0.82 + 0.18 * (p[1] * 38.0 + (p[0] * 4.0 + p[2] * 3.0).sin() * 2.5).sin();
        gloss(scale3([0.45, 0.25, 0.1], grain), 0.3)
    };
    mesh.cuboid(at([0.0; 3]), [0.95, 0.68, 0.6], |hit| {
        let [x, y, _] = hit.local;
        if hit.normal[2] < 0.5 {
            return wood(hit.local);
        }
        if x > 0.5 {
            // Silver control panel with speaker slots below the knobs.
            let slot = y < -0.12 && (y * 18.0).rem_euclid(1.0) < 0.45 && x > 0.56 && x < 0.89;
            return gloss(if slot { [0.1; 3] } else { [0.6, 0.6, 0.63] }, 0.7);
        }
        gloss([0.09, 0.09, 0.1], 0.4)
    });
    mesh.cuboid(at([-0.2, 0.0, 0.61]), [0.62, 0.52, 0.02], |hit| {
        if hit.normal[2] < 0.5 {
            return gloss([0.09, 0.09, 0.1], 0.4);
        }
        let [u, v] = hit.uv;
        let band = ((u * 8.0) as usize).min(7);
        let level = 0.12 + 0.82 * look.spectrum[band];
        let edge = (u - 0.5).abs().max((v - 0.5).abs());
        let tube = 1.0 - (edge - 0.38).max(0.0) * 4.0;
        let line = [0.7, 1.0][usize::from((v * 48.0).rem_euclid(1.0) < 0.7)];
        let color = scale3(BARS[band], if v < level { 1.0 } else { 0.12 });
        Surface {
            color: scale3(color, tube * line),
            shine: 0.9,
            glow: 0.85,
        }
    });
    for y in [0.36, 0.08] {
        let turn = look.t * 0.02 + y * 9.0;
        let knob = at([0.72, y, 0.64]).turned(0, FRAC_PI_2);
        mesh.cylinder(knob, [0.09, 0.0, 0.04], move |hit| {
            let angle = hit.local[2].atan2(hit.local[0]);
            if hit.normal[1] > 0.5 && (angle - turn).rem_euclid(TAU) < 0.35 {
                matte([0.9, 0.1, 0.05])
            } else {
                gloss([0.15; 3], 0.8)
            }
        });
    }
    mesh.cylinder(at([0.0, 0.72, -0.2]), [0.14, 0.0, 0.05], |_| BLACK_PLASTIC);
    for side in [-1.0f32, 1.0] {
        let angle = side * (0.55 + 0.05 * (look.t * 0.03).sin());
        let (sin, cos) = sin_cos(angle);
        let up = [-sin, cos, 0.0];
        let along = |d: f32| [up[0] * d, 0.76 + up[1] * d, -0.2];
        let rod = at(along(0.5)).turned(2, angle);
        mesh.cylinder(rod, [0.018, 0.0, 0.5], |_| CHROME);
        mesh.sphere(at(along(1.0)), 0.04, |_| CHROME);
    }
    for x in [-0.7, 0.7] {
        for z in [-0.42, 0.42] {
            mesh.cylinder(at([x, -0.76, z]), [0.05, 0.0, 0.08], |_| BLACK_PLASTIC);
        }
    }
}

/// A black VHS cassette: a striped "T-120" label and two windows whose
/// reels spin.
fn video_tape(mesh: &mut Mesh, look: &Look) {
    let spin = look.t * 0.08 * (1.0 + look.energy);
    mesh.cuboid(at([0.0; 3]), [1.0, 0.56, 0.13], |hit| {
        let [x, y, z] = hit.local;
        if hit.normal[2] < 0.5 {
            let ribbed = hit.normal[1].abs() < 0.5 && (z * 40.0).rem_euclid(1.0) < 0.3;
            return [BLACK_PLASTIC, gloss([0.02; 3], 0.3)][usize::from(ribbed)];
        }
        if x.abs() < 0.86 && (0.1..0.46).contains(&y) {
            let (u, v) = ((x + 0.86) / 1.72, (y - 0.1) / 0.36);
            if text_lit("T-120", (u - 0.06) / 0.42, (v - 0.25) / 0.5) {
                return matte([0.05; 3]);
            }
            if u > 0.55 && u < 0.97 && v > 0.2 && v < 0.8 {
                return matte(hue((v - 0.2) * 0.25 + 0.95));
            }
            return matte([0.92, 0.9, 0.84]);
        }
        if x.abs() < 0.66 && (-0.38..0.0).contains(&y) {
            for centre in [-0.36f32, 0.36] {
                let (dx, dy) = (x - centre, y + 0.19);
                let r = dx.hypot(dy);
                let wound = if centre < 0.0 { 0.17 } else { 0.13 };
                if r < 0.1 {
                    let spoke = ((dy.atan2(dx) - spin) * 3.0 / PI).rem_euclid(2.0) < 0.6;
                    let dark = spoke || r < 0.035;
                    return matte(if dark { [0.15; 3] } else { [0.92; 3] });
                }
                if r < wound {
                    return gloss([0.22, 0.12, 0.06], 0.5);
                }
            }
            return gloss([0.12, 0.1, 0.12], 1.0);
        }
        BLACK_PLASTIC
    });
}

/// A compact disc facing the viewer; its data side shows a rainbow sheen
/// that slides as the disc rocks.
fn compact_disc(mesh: &mut Mesh, look: &Look) {
    let light = 0.8 + look.yaw * 1.6 + look.t * 0.01;
    let place = at([0.0; 3]).turned(0, FRAC_PI_2);
    mesh.cylinder(place, [1.0, 0.16, 0.012], |hit| {
        let [x, _, z] = hit.local;
        let r = x.hypot(z);
        if hit.normal[1].abs() < 0.5 {
            return CHROME;
        }
        if hit.normal[1] > 0.0 {
            if r < 0.34 {
                let ring = (0.24..0.27).contains(&r);
                return gloss(if ring { [0.45; 3] } else { [0.7, 0.74, 0.78] }, 1.0);
            }
            let angle = z.atan2(x);
            let streak = cos(angle - light).powi(2);
            let rainbow = hue(r * 1.3 + 0.35 * cos(2.0 * (angle - light)) + look.treble * 0.3);
            return Surface {
                color: mix3([0.72, 0.74, 0.78], rainbow, 0.25 + 0.7 * streak),
                shine: 1.0,
                glow: 0.3,
            };
        }
        // The printed label side.
        matte(match (r * 9.0) as usize % 3 {
            0 => [0.95, 0.95, 0.92],
            1 => [0.85, 0.1, 0.15],
            _ => [0.1, 0.25, 0.8],
        })
    });
}

/// The sun, smiling in sunglasses, its surface boiling.
fn sun(mesh: &mut Mesh, look: &Look) {
    let boil = (look.t * 0.25) as u32;
    mesh.sphere(at([0.0; 3]), 0.78, |hit| {
        let [x, y, z] = hit.local;
        if z > 0.2 && y < -0.12 && (0.29..0.36).contains(&x.hypot(y + 0.02)) {
            return lit([0.6, 0.12, 0.02], 0.8);
        }
        if z > 0.3 && y < 0.0 && (x.abs() - 0.42).hypot(y + 0.08) < 0.1 {
            return lit([1.0, 0.45, 0.35], 0.9);
        }
        let granule = speckle(hit.local, 0.09, boil);
        let warm = 0.5 + 0.5 * hit.normal[2];
        let heat = warm * 0.7 + granule * 0.3;
        lit(mix3([1.0, 0.42, 0.04], [1.0, 0.88, 0.3], heat), 0.92)
    });
    let shades = |hit: &Hit| {
        let [x, y, _] = hit.local;
        if hit.normal[2] > 0.5 && (x + y * 0.7 + 0.05).abs() < 0.03 {
            lit([0.9; 3], 1.0)
        } else {
            gloss([0.03, 0.03, 0.05], 1.0)
        }
    };
    for side in [-1.0f32, 1.0] {
        mesh.cuboid(at([side * 0.27, 0.16, 0.72]), [0.2, 0.12, 0.05], shades);
        let arm = at([side * 0.5, 0.2, 0.38]);
        mesh.cuboid(arm, [0.025, 0.025, 0.36], |_| BLACK_PLASTIC);
    }
    let bridge = at([0.0, 0.21, 0.74]);
    mesh.cuboid(bridge, [0.08, 0.025, 0.03], |_| BLACK_PLASTIC);
}

/// A round tree with a few apples on a patch of grass; the crown sways
/// with the mids.
fn tree(mesh: &mut Mesh, look: &Look) {
    let sway = (look.t * 0.06).sin() * (0.02 + 0.06 * look.mid);
    let bark = |hit: &Hit| {
        let stripe = (hit.uv[0] * 40.0 + hit.local[1] * 6.0).sin() * 0.12;
        matte(scale3([0.38, 0.23, 0.11], 0.85 + stripe))
    };
    mesh.cylinder(at([0.0, -0.5, 0.0]), [0.17, 0.0, 0.45], bark);
    for side in [-1.0f32, 1.0] {
        let branch = at([side * 0.2, -0.05, 0.0]).turned(2, -side * 0.8);
        mesh.cylinder(branch, [0.05, 0.0, 0.25], bark);
    }
    let crowns: [(Vec3, f32); 5] = [
        ([0.0, 0.33, 0.0], 0.55),
        ([-0.46, 0.12, 0.08], 0.38),
        ([0.46, 0.18, -0.06], 0.4),
        ([0.05, 0.72, 0.02], 0.36),
        ([0.12, 0.22, 0.42], 0.32),
    ];
    for (index, (centre, radius)) in crowns.into_iter().enumerate() {
        let lean = sway * (centre[1] + 0.4);
        let centre = [centre[0] + lean, centre[1], centre[2] + lean * 0.5];
        mesh.sphere(at(centre), radius, |hit| {
            let world = std::array::from_fn(|k| hit.local[k] + centre[k]);
            let leaf = speckle(world, 0.08, index as u32);
            if leaf < 0.03 && hit.normal[1] > -0.3 {
                return gloss([0.85, 0.06, 0.04], 0.9);
            }
            matte(scale3([0.14, 0.52, 0.1], 0.65 + 0.5 * leaf))
        });
    }
    mesh.cylinder(at([0.0, -0.97, 0.0]), [0.7, 0.0, 0.03], |hit| {
        if hit.normal[1] > 0.5 {
            let shade = 0.8 + 0.3 * speckle(hit.local, 0.05, 9);
            matte(scale3([0.2, 0.6, 0.12], shade))
        } else {
            matte([0.35, 0.22, 0.12])
        }
    });
}

/// A beige 1980s desktop: a system unit with two floppy drives, a monitor
/// of green phosphor text (bars following the spectrum) and a keyboard.
fn computer(mesh: &mut Mesh, look: &Look) {
    mesh.cuboid(at([0.0, -0.45, -0.1]), [0.9, 0.17, 0.62], |hit| {
        let [x, y, _] = hit.local;
        if hit.normal[2] < 0.5 {
            return matte(BEIGE);
        }
        for slot in [0.065, -0.065] {
            if (0.12..0.82).contains(&x) && (y - slot).abs() < 0.05 {
                let gap = (y - slot).abs() < 0.008 && (0.2..0.66).contains(&x);
                let latch = (0.7..0.76).contains(&x) && (y - slot).abs() < 0.03;
                return matte([[0.3, 0.29, 0.27], [0.05; 3]][usize::from(gap || latch)]);
            }
        }
        if (x + 0.75).hypot(y + 0.08) < 0.022 {
            return lit([1.0, 0.1 + 0.4 * look.beat, 0.05], 1.0);
        }
        if (-0.82..-0.62).contains(&x) && (0.03..0.11).contains(&y) {
            return matte(hue((0.11 - y) / 0.08 * 0.8));
        }
        let vent = (-0.5..0.0).contains(&x) && (y * 30.0).rem_euclid(1.0) < 0.3;
        matte(if vent { scale3(BEIGE, 0.6) } else { BEIGE })
    });
    mesh.cuboid(at([0.0, 0.2, -0.15]), [0.62, 0.46, 0.42], |_| matte(BEIGE));
    let back = scale3(BEIGE, 0.85);
    mesh.cuboid(at([0.0, 0.18, -0.68]), [0.45, 0.34, 0.12], |_| matte(back));
    let cursor = ((look.t / 20.0) as u32).is_multiple_of(2);
    mesh.cuboid(at([0.0, 0.23, 0.28]), [0.49, 0.36, 0.015], |hit| {
        if hit.normal[2] < 0.5 {
            return matte(BEIGE);
        }
        // A big "READY" across the top half (large enough to survive the
        // splat grid), then four spectrum bars and a blinking cursor block.
        let [u, v] = hit.uv;
        let on = if v > 0.5 {
            text_lit("READY", (u - 0.06) / 0.88, (v - 0.53) / 0.42)
        } else {
            let row = ((0.5 - v) / 0.1) as usize;
            let row_v = ((0.5 - v) / 0.1).fract();
            match row {
                0..4 => {
                    let level = look.spectrum[row * 2];
                    u > 0.06 && u < 0.1 + 0.84 * level && row_v > 0.2 && row_v < 0.8
                }
                _ => cursor && u > 0.06 && u < 0.16 && row_v < 0.8,
            }
        };
        Surface {
            color: [[0.02, 0.07, 0.03], [0.25, 1.0, 0.35]][usize::from(on)],
            shine: 0.8,
            glow: 0.9,
        }
    });
    let keyboard = at([0.0, -0.68, 0.78]).turned(0, 0.12);
    mesh.cuboid(keyboard, [0.82, 0.045, 0.2], |hit| {
        if hit.normal[1] < 0.5 {
            return matte(BEIGE);
        }
        let [u, v] = hit.uv;
        let (kx, ky) = ((u * 17.0).fract(), (v * 5.0).fract());
        if kx < 0.12 || ky < 0.15 {
            return matte([0.25, 0.23, 0.2]);
        }
        let dark = (v * 5.0) as usize == 0 || (u * 17.0) as usize >= 15;
        matte([[0.88, 0.84, 0.74], [0.55, 0.5, 0.42]][usize::from(dark)])
    });
}
