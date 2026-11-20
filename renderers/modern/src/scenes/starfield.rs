//! Catalog-driven local-sky star field.

use std::io::Read;
use std::sync::Arc;

use super::{CameraMode, SceneBuild, SceneEntry};
use crate::control_frame::ControlFrame;
use crate::profile::RenderProfile;
use crate::scene::Instance;
use crate::scenes::SceneState;

pub const ENTRY: SceneEntry = SceneEntry {
    name: "Starfield",
    camera: CameraMode::ScreenSpace,
    build: SceneBuild::Stateful(build),
};
pub const SCENE: u8 = 6;
pub const MAX_RENDERED_MAGNITUDE: f64 = 6.5;

#[derive(Clone, Debug)]
pub struct StarfieldConfig {
    pub catalog: Arc<StarCatalog>,
    pub latitude_degrees: f64,
    pub longitude_degrees: f64,
    pub unix_seconds: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Star {
    pub right_ascension_hours: f64,
    pub declination_degrees: f64,
    pub magnitude: f64,
    pub colour_index: Option<f64>,
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StarCatalog {
    /// Brightest-first, so the profile's instance cap retains the most visible stars.
    stars: Vec<Star>,
}

impl StarCatalog {
    pub fn from_reader(reader: impl Read) -> Result<Self, CatalogError> {
        let mut reader = csv::ReaderBuilder::new()
            .flexible(false)
            .from_reader(reader);
        let headers = reader
            .headers()
            .map_err(|error| CatalogError(format!("could not read CSV header row: {error}")))?;
        let mut columns = std::collections::HashMap::new();
        for (index, name) in headers.iter().enumerate() {
            if columns.insert(name.to_string(), index).is_some() {
                return Err(CatalogError(format!(
                    "duplicate CSV header {name:?}; each column name must be unique"
                )));
            }
        }
        for required in ["ra", "dec", "mag"] {
            if !columns.contains_key(required) {
                return Err(CatalogError(format!(
                    "catalog is missing required CSV column {required:?} (required: ra, dec, mag)"
                )));
            }
        }
        let ra_column = columns["ra"];
        let dec_column = columns["dec"];
        let magnitude_column = columns["mag"];
        let colour_column = columns.get("ci").copied();
        let name_column = columns.get("proper").copied();

        let mut stars = Vec::new();
        for (record_index, result) in reader.records().enumerate() {
            let line = record_index + 2;
            let record = result.map_err(|error| {
                CatalogError(format!("malformed CSV record on line {line}: {error}"))
            })?;
            let ra = parse_field(&record, ra_column, "ra", line)?;
            if !(0.0..24.0).contains(&ra) {
                return Err(CatalogError(format!(
                    "line {line}: ra must be finite and in [0, 24) hours, got {ra}"
                )));
            }
            let dec = parse_field(&record, dec_column, "dec", line)?;
            if !(-90.0..=90.0).contains(&dec) {
                return Err(CatalogError(format!(
                    "line {line}: dec must be finite and in [-90, 90] degrees, got {dec}"
                )));
            }
            let magnitude = parse_field(&record, magnitude_column, "mag", line)?;
            let colour_index = colour_column
                .filter(|&column| !record.get(column).unwrap_or_default().trim().is_empty())
                .map(|column| parse_field(&record, column, "ci", line))
                .transpose()?;
            let name = name_column
                .and_then(|column| record.get(column))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            stars.push(Star {
                right_ascension_hours: ra,
                declination_degrees: dec,
                magnitude,
                colour_index,
                name,
            });
        }
        if stars.is_empty() {
            return Err(CatalogError(
                "catalog contains no star records; provide at least one data row".to_string(),
            ));
        }
        stars.sort_by(|left, right| left.magnitude.total_cmp(&right.magnitude));
        stars.retain(|star| star.magnitude <= MAX_RENDERED_MAGNITUDE);
        if stars.is_empty() {
            return Err(CatalogError(format!(
                "catalog has no stars with magnitude <= {MAX_RENDERED_MAGNITUDE}; the renderer's visibility limit is magnitude {MAX_RENDERED_MAGNITUDE}"
            )));
        }
        Ok(Self { stars })
    }

    pub fn load(path: &std::path::Path) -> Result<Self, CatalogError> {
        let file = std::fs::File::open(path).map_err(|error| {
            CatalogError(format!("cannot open catalog {}: {error}", path.display()))
        })?;
        Self::from_reader(file)
            .map_err(|error| CatalogError(format!("invalid catalog {}: {error}", path.display())))
    }

    pub fn stars(&self) -> &[Star] {
        &self.stars
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogError(pub String);

impl std::fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CatalogError {}

fn parse_field(
    record: &csv::StringRecord,
    column: usize,
    name: &str,
    line: usize,
) -> Result<f64, CatalogError> {
    let value = record.get(column).unwrap_or_default().trim();
    let parsed = value.parse::<f64>().map_err(|_| {
        CatalogError(format!(
            "line {line}: {name} must be a finite number, got {value:?}"
        ))
    })?;
    if !parsed.is_finite() {
        return Err(CatalogError(format!(
            "line {line}: {name} must be finite, got {value:?}"
        )));
    }
    Ok(parsed)
}

/// Returns (azimuth, altitude) in radians. Azimuth is clockwise from north.
pub fn horizontal_coordinates(
    star: &Star,
    latitude_degrees: f64,
    longitude_degrees: f64,
    unix_seconds: f64,
) -> (f64, f64) {
    let julian_date = unix_seconds / 86_400.0 + 2_440_587.5;
    let centuries = (julian_date - 2_451_545.0) / 36_525.0;
    let gmst_degrees = 280.460_618_37
        + 360.985_647_366_29 * (julian_date - 2_451_545.0)
        + 0.000_387_933 * centuries * centuries
        - centuries * centuries * centuries / 38_710_000.0;
    let local_sidereal = (gmst_degrees + longitude_degrees)
        .rem_euclid(360.0)
        .to_radians();
    let right_ascension = (star.right_ascension_hours * 15.0).to_radians();
    let declination = star.declination_degrees.to_radians();
    let latitude = latitude_degrees.to_radians();
    let hour_angle = local_sidereal - right_ascension;

    let sin_altitude =
        declination.sin() * latitude.sin() + declination.cos() * latitude.cos() * hour_angle.cos();
    let altitude = sin_altitude.clamp(-1.0, 1.0).asin();
    let azimuth = (-hour_angle.sin() * declination.cos())
        .atan2(
            declination.sin() * latitude.cos()
                - declination.cos() * latitude.sin() * hour_angle.cos(),
        )
        .rem_euclid(std::f64::consts::TAU);
    (azimuth, altitude)
}

/// Projects the visible local sky to an azimuth/altitude panorama. North wraps
/// across the left and right edges; east, south and west lie at x=-0.5, 0, 0.5.
pub fn project_star(
    star: &Star,
    latitude_degrees: f64,
    longitude_degrees: f64,
    unix_seconds: f64,
) -> Option<[f32; 2]> {
    let (azimuth, altitude) =
        horizontal_coordinates(star, latitude_degrees, longitude_degrees, unix_seconds);
    if altitude <= 0.0 {
        return None;
    }
    Some([
        (azimuth / std::f64::consts::PI - 1.0) as f32,
        (altitude / std::f64::consts::FRAC_PI_2 * 2.0 - 1.0) as f32,
    ])
}

fn build(
    profile: &RenderProfile,
    frame: ControlFrame,
    _elapsed: f32,
    aspect: f32,
    state: &SceneState,
    instances: &mut Vec<Instance>,
) {
    let Some(config) = state.starfield() else {
        return;
    };
    let audio_level = frame.energy as f32 / 255.0;
    for star in config
        .catalog
        .stars()
        .iter()
        .take_while(|star| star.magnitude <= MAX_RENDERED_MAGNITUDE)
    {
        let Some(center) = project_star(
            star,
            config.latitude_degrees,
            config.longitude_degrees,
            config.unix_seconds,
        ) else {
            continue;
        };
        // Lift the catalog's large naked-eye dynamic range for an emissive
        // display while keeping magnitude ordering monotonic.
        let brightness = (10.0_f64.powf(-0.2 * (star.magnitude + 1.5)) as f32).clamp(0.0, 1.0)
            * (0.85 + audio_level * 0.15);
        let tint = star
            .colour_index
            .map(color_index_rgb)
            .unwrap_or([1.0, 1.0, 1.0]);
        let point_size = 0.0025 + brightness * 0.009;
        // Keep the catalog-derived hue while allowing the audio envelope only
        // to make a bounded, non-flashing brightness adjustment.
        instances.push(Instance {
            center: [center[0], center[1], 0.0],
            size: [point_size / aspect.max(f32::MIN_POSITIVE), point_size],
            color: [
                tint[0] * brightness,
                tint[1] * brightness,
                tint[2] * brightness,
                1.0,
            ],
            rotation: 0.0,
            scene_mix: 0.0,
            pattern: [0.0; 2],
        });
        if instances.len() >= profile.instance_count {
            break;
        }
    }
}

fn color_index_rgb(colour_index: f64) -> [f32; 3] {
    // A compact black-body approximation for the familiar B-V colour index.
    let temperature = (4600.0
        * (1.0 / (0.92 * colour_index + 1.7) + 1.0 / (0.92 * colour_index + 0.62)))
        .clamp(1_000.0, 40_000.0);
    let temperature = temperature / 100.0;
    let red = if temperature <= 66.0 {
        255.0
    } else {
        (329.698_73 * (temperature - 60.0).powf(-0.133_204_76)).clamp(0.0, 255.0)
    };
    let green = if temperature <= 66.0 {
        (99.470_8 * temperature.ln() - 161.119_57).clamp(0.0, 255.0)
    } else {
        (288.122_16 * (temperature - 60.0).powf(-0.075_514_85)).clamp(0.0, 255.0)
    };
    let blue = if temperature >= 66.0 {
        255.0
    } else if temperature <= 19.0 {
        0.0
    } else {
        (138.517_73 * (temperature - 10.0).ln() - 305.044_8).clamp(0.0, 255.0)
    };
    [
        (red / 255.0) as f32,
        (green / 255.0) as f32,
        (blue / 255.0) as f32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(csv: &str) -> Result<StarCatalog, CatalogError> {
        StarCatalog::from_reader(csv.as_bytes())
    }

    #[test]
    fn parses_quoted_optional_names_and_sorts_by_brightness() {
        let parsed =
            catalog("proper,ra,dec,mag,ci\n\"Alpha, the bright\",6,23,1.0,0.2\nBeta,12,-10,2.0,\n")
                .expect("valid catalog");
        assert_eq!(parsed.stars()[0].name.as_deref(), Some("Alpha, the bright"));
        assert_eq!(parsed.stars()[0].colour_index, Some(0.2));
        assert_eq!(parsed.stars()[1].name.as_deref(), Some("Beta"));
        assert_eq!(parsed.stars()[1].colour_index, None);
    }

    #[test]
    fn rejects_missing_headers_empty_catalog_and_bad_values() {
        for (contents, expected) in [
            ("ra,dec\n1,2\n", "missing required CSV column"),
            ("ra,dec,mag\n", "no star records"),
            ("ra,dec,mag\n1,2,7\n", "no stars with magnitude <= 6.5"),
            ("ra,dec,mag\n,2,1\n", "ra must be"),
            ("ra,dec,mag\n1,,1\n", "dec must be"),
            ("ra,dec,mag\n1,2,nan\n", "mag must be finite"),
            ("ra,dec,mag\n24,2,1\n", "ra must be finite and in [0, 24)"),
            (
                "ra,dec,mag\n1,-91,1\n",
                "dec must be finite and in [-90, 90]",
            ),
            ("ra,dec,mag,ci\n1,2,1,nope\n", "ci must be"),
            ("ra,dec,mag\n\"unterminated,2,1\n", "malformed CSV record"),
        ] {
            let error = catalog(contents).expect_err("invalid catalog");
            assert!(error.to_string().contains(expected), "{contents}: {error}");
        }
    }

    #[test]
    fn skips_blank_optional_fields_and_filters_fainter_than_limit() {
        let catalog = catalog(
            "proper,ra,dec,mag,ci\nBright,0,90,1.0,0.2\n\"\",1,90,6.5,\nToo faint,2,90,6.51,\n",
        )
        .expect("blank optional fields are allowed");
        assert_eq!(catalog.stars()[1].name, None);
        assert_eq!(catalog.stars()[1].colour_index, None);

        let mut state = SceneState::default();
        state.configure_starfield(Some(StarfieldConfig {
            catalog: Arc::new(catalog),
            latitude_degrees: 45.0,
            longitude_degrees: 0.0,
            unix_seconds: 946_728_000.0,
        }));
        let mut instances = Vec::new();
        build(
            &RenderProfile::pi5(),
            ControlFrame::procedural(0),
            0.0,
            16.0 / 9.0,
            &state,
            &mut instances,
        );
        assert_eq!(instances.len(), 2);
        assert!(instances[0].size[0] > instances[1].size[0]);
    }

    #[test]
    fn projection_is_deterministic_for_known_time_and_location() {
        let star = Star {
            right_ascension_hours: 6.0,
            declination_degrees: 0.0,
            magnitude: 1.0,
            colour_index: None,
            name: None,
        };
        let timestamp = 946_693_184.772_528;
        let actual = project_star(&star, 0.0, 0.0, timestamp).expect("star is above the horizon");
        let expected = [0.5, 0.0];
        assert!((actual[0] - expected[0]).abs() < 1.0e-5, "{actual:?}");
        assert!((actual[1] - expected[1]).abs() < 1.0e-5, "{actual:?}");
        assert_eq!(project_star(&star, 0.0, 0.0, timestamp), Some(actual));
        assert!(project_star(&star, 0.0, 90.0, timestamp).is_none());
        let different_latitude = project_star(&star, 45.0, 0.0, timestamp).unwrap();
        assert!((different_latitude[1] - actual[1]).abs() > 0.1);
        assert!(project_star(&star, 0.0, 0.0, timestamp + 21_600.0).is_none());
    }
}
