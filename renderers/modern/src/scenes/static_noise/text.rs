//! Understandable content the Static scene resolves into: embedded
//! multilingual phrases, mathematical constants and a traced 3D curve.
//!
//! Text reaches the scene through [`TextSource`], so a later live source
//! (for example climate headlines fetched on the Mac or bridge, issue #15)
//! can replace or extend [`Embedded`] without touching the scene. This phase
//! adds no networking.

use std::f64::consts::{E, PI, SQRT_2};

/// A list of phrases the scene can show. Phrases should use characters the
/// 5x8 font draws (see `font.rs`); anything else renders as glyph-like noise.
pub(crate) trait TextSource {
    fn len(&self) -> usize;
    fn phrase(&self, index: usize) -> &str;
}

/// The built-in phrase list.
pub(crate) struct Embedded;

/// Source and licence: short greetings and everyday words written for chipviz
/// by its contributors (no third-party text), covered by the repository
/// licence. Language tags are BCP 47. Uppercase, because the font is.
pub(crate) const PHRASES: &[(&str, &str)] = &[
    ("en", "SIGNAL AND NOISE"),
    ("es", "HOLA MUNDO"),
    ("el", "ΓΕΙΑ ΣΟΥ ΚΟΣΜΕ"),
    ("fr", "BRUIT BLANC"),
    ("ru", "ПРИВЕТ МИР"),
    ("de", "HALLO WELT"),
    ("pt", "OLÁ MUNDO"),
    ("tr", "MERHABA DÜNYA"),
    ("uk", "ДОБРИЙ ДЕНЬ"),
    ("sw", "HABARI DUNIA"),
    ("vi", "XIN CHÀO"),
    ("it", "CIAO MONDO"),
    ("pl", "CZEŚĆ"),
    ("mi", "KIA ORA"),
    ("is", "HALLÓ HEIMUR"),
    ("eo", "SALUTON MONDO"),
    ("fi", "HEI MAAILMA"),
    ("es", "LA TIERRA SE CALIENTA"),
];

impl TextSource for Embedded {
    fn len(&self) -> usize {
        PHRASES.len()
    }

    fn phrase(&self, index: usize) -> &str {
        PHRASES[index % PHRASES.len()].1
    }
}

/// Constant `index` (cycling) as display lines, e.g. `["π =", "3.14159..."]`.
/// Values come from `std` and are printed to 15 decimal places, the
/// precision an `f64` carries; nothing is fetched.
pub(crate) fn constant(index: usize) -> [String; 2] {
    let golden = (1.0 + 5.0_f64.sqrt()) / 2.0;
    let (name, value) = [("π", PI), ("e", E), ("φ", golden), ("√2", SQRT_2)][index % 4];
    [format!("{name} ="), format!("{value:.15}")]
}

/// Point `t` (0..1) along a (3, 2) torus knot, rotated by `angle` about
/// the vertical and tipped a little towards the viewer. Coordinates lie in
/// roughly -1..1.
pub(crate) fn knot_point(t: f32, angle: f32) -> [f32; 3] {
    let u = t * std::f32::consts::TAU;
    let radius = 2.0 + (3.0 * u).cos();
    let point = [
        radius * (2.0 * u).cos(),
        (3.0 * u).sin(),
        radius * (2.0 * u).sin(),
    ];
    let (sin, cos) = angle.sin_cos();
    let x = point[0] * cos + point[2] * sin;
    let z = -point[0] * sin + point[2] * cos;
    let (tip_sin, tip_cos) = 0.45f32.sin_cos();
    let y = point[1] * tip_cos - z * tip_sin;
    let z = point[1] * tip_sin + z * tip_cos;
    [x / 3.2, y / 3.2, z / 3.2]
}

/// The coordinate label shown under the traced knot.
pub(crate) fn coordinates(point: [f32; 3]) -> String {
    format!("X{:+.3} Y{:+.3} Z{:+.3}", point[0], point[1], point[2])
}

#[cfg(test)]
mod tests {
    use super::super::font::glyph;
    use super::*;

    #[test]
    fn every_embedded_phrase_is_drawable() {
        for (language, phrase) in PHRASES {
            for character in phrase.chars() {
                assert!(glyph(character).is_some(), "{language}: {character}");
            }
        }
        for index in 0..4 {
            for line in constant(index) {
                assert!(line.chars().all(|character| glyph(character).is_some()));
            }
        }
        let label = coordinates(knot_point(0.3, 1.2));
        assert!(label.chars().all(|character| glyph(character).is_some()));
    }

    #[test]
    fn constants_are_generated_locally_to_fifteen_places() {
        assert_eq!(constant(0), ["π =", "3.141592653589793"]);
        assert_eq!(constant(1)[1], "2.718281828459045");
        assert_eq!(constant(2)[1], "1.618033988749895");
        assert_eq!(constant(3), ["√2 =", "1.414213562373095"]);
        assert_eq!(constant(4), constant(0));
    }

    #[test]
    fn knot_stays_in_range() {
        for step in 0..=200 {
            let point = knot_point(step as f32 / 200.0, step as f32 * 0.37);
            assert!(
                point
                    .iter()
                    .all(|value| value.is_finite() && value.abs() <= 1.01)
            );
        }
    }
}
