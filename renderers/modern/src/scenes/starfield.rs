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
    build: SceneBuild::ViewportStateful(build),
};
pub const SCENE: u8 = 6;
pub const MAX_RENDERED_MAGNITUDE: f64 = 6.5;
pub const STARFIELD_SNAPSHOT_PERIOD_SECONDS: f64 = 10.0;
const MAX_STAR_LINEAR_LUMINANCE: f32 = 0.05;
const MAX_RENDERED_STARS: usize = 200;
const RIGHT_ASCENSION_BINS: usize = 48;
const DECLINATION_BINS: usize = 18;
const FINE_RA_BINS_PER_SKY_BIN: usize = 4;
const FINE_DEC_BINS_PER_SKY_BIN: usize = 4;
const FINE_SKY_SUBCELLS: usize = FINE_RA_BINS_PER_SKY_BIN * FINE_DEC_BINS_PER_SKY_BIN;
const STARS_PER_FINE_SUBCELL: usize = 16;
#[cfg(test)]
const EXTRA_BOUNDARY_CANDIDATES_PER_SUBCELL: usize = 4;
#[cfg(test)]
const MAX_CANDIDATES_PER_FINE_SUBCELL: usize =
    STARS_PER_FINE_SUBCELL + EXTRA_BOUNDARY_CANDIDATES_PER_SUBCELL + 1;
#[cfg(test)]
const SKY_CANDIDATE_BOUND: usize =
    RIGHT_ASCENSION_BINS * DECLINATION_BINS * FINE_SKY_SUBCELLS * MAX_CANDIDATES_PER_FINE_SUBCELL;
const MIN_STAR_MAGNITUDE: f64 = -2.0;
const BRIGHTNESS_DECADES_PER_MAGNITUDE: f64 = 0.23;

#[derive(Clone, Debug)]
pub struct StarfieldConfig {
    pub catalog: Arc<StarCatalog>,
    pub latitude_degrees: f64,
    pub longitude_degrees: f64,
    pub unix_seconds: f64,
    /// Advance from the base UTC epoch with the renderer's monotonic clock.
    pub follow_elapsed: bool,
}

fn star_brightness(magnitude: f64) -> f32 {
    (f64::from(MAX_STAR_LINEAR_LUMINANCE)
        * 10.0_f64.powf(-BRIGHTNESS_DECADES_PER_MAGNITUDE * (magnitude - MIN_STAR_MAGNITUDE)))
    .clamp(0.000_55, f64::from(MAX_STAR_LINEAR_LUMINANCE)) as f32
}

impl StarfieldConfig {
    pub fn epoch_at(&self, elapsed: f32) -> f64 {
        self.unix_seconds
            + if self.follow_elapsed {
                (f64::from(elapsed.max(0.0)) / STARFIELD_SNAPSHOT_PERIOD_SECONDS).floor()
                    * STARFIELD_SNAPSHOT_PERIOD_SECONDS
            } else {
                0.0
            }
    }
}

/// The latest valid wall-clock UTC reading, sampled on the monotonic render
/// clock. The app publishes it only at the Starfield snapshot cadence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UtcClock {
    unix_seconds: f64,
    monotonic_seconds: f64,
}

impl UtcClock {
    pub fn new(unix_seconds: f64) -> Self {
        Self {
            unix_seconds,
            monotonic_seconds: 0.0,
        }
    }

    pub fn update(&mut self, wall_unix_seconds: f64, monotonic_seconds: f64) -> f64 {
        if !wall_unix_seconds.is_finite() || !monotonic_seconds.is_finite() {
            return self.unix_seconds;
        }
        if monotonic_seconds < self.monotonic_seconds {
            return self.unix_seconds;
        }
        self.unix_seconds = wall_unix_seconds;
        self.monotonic_seconds = monotonic_seconds;
        self.unix_seconds
    }
}

pub fn snapshot_due(elapsed: f64, last_snapshot: f64) -> bool {
    elapsed.is_finite()
        && last_snapshot.is_finite()
        && elapsed - last_snapshot >= STARFIELD_SNAPSHOT_PERIOD_SECONDS
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
    /// Bounded bright and coordinate-boundary candidates per J2000 sky subcell.
    candidate_bins: Vec<Vec<usize>>,
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
        let distance_column = columns.get("dist").copied();

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
            let observer_distance = distance_column
                .filter(|&column| !record.get(column).unwrap_or_default().trim().is_empty())
                .map(|column| parse_field(&record, column, "dist", line))
                .transpose()?;
            if observer_distance == Some(0.0) {
                continue;
            }
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
        let candidate_bins = Self::build_candidate_bins(&stars);
        Ok(Self {
            stars,
            candidate_bins,
        })
    }

    fn build_candidate_bins(stars: &[Star]) -> Vec<Vec<usize>> {
        let mut bins = vec![Vec::new(); RIGHT_ASCENSION_BINS * DECLINATION_BINS];
        let mut candidates: Vec<Vec<Vec<usize>>> =
            vec![vec![Vec::new(); FINE_SKY_SUBCELLS]; bins.len()];
        let mut coincident = vec![vec![None; FINE_SKY_SUBCELLS]; bins.len()];
        let mut extremes: Vec<Vec<[Option<usize>; 4]>> =
            vec![vec![[None; 4]; FINE_SKY_SUBCELLS]; bins.len()];
        for (index, star) in stars.iter().enumerate() {
            let ra_bin = ((star.right_ascension_hours / 24.0 * RIGHT_ASCENSION_BINS as f64)
                as usize)
                .min(RIGHT_ASCENSION_BINS - 1);
            let dec_bin = (((star.declination_degrees + 90.0) / 180.0 * DECLINATION_BINS as f64)
                as usize)
                .min(DECLINATION_BINS - 1);
            let bin_index = dec_bin * RIGHT_ASCENSION_BINS + ra_bin;
            let ra_in_bin =
                star.right_ascension_hours / 24.0 * RIGHT_ASCENSION_BINS as f64 - ra_bin as f64;
            let dec_in_bin = (star.declination_degrees + 90.0) / 180.0 * DECLINATION_BINS as f64
                - dec_bin as f64;
            let fine_ra = (ra_in_bin * FINE_RA_BINS_PER_SKY_BIN as f64) as usize;
            let fine_dec = (dec_in_bin * FINE_DEC_BINS_PER_SKY_BIN as f64) as usize;
            let subcell = fine_dec.min(FINE_DEC_BINS_PER_SKY_BIN - 1) * FINE_RA_BINS_PER_SKY_BIN
                + fine_ra.min(FINE_RA_BINS_PER_SKY_BIN - 1);
            // The magnitude cap alone can hide a horizon-visible boundary
            // star behind brighter records just below the horizon. Keep
            // coordinate extremes independently of that capped list.
            let subcell_extremes = &mut extremes[bin_index][subcell];
            if let Some(first) = subcell_extremes[0] {
                if star.right_ascension_hours < stars[first].right_ascension_hours {
                    subcell_extremes[0] = Some(index);
                }
                if star.right_ascension_hours
                    > stars[subcell_extremes[1].unwrap()].right_ascension_hours
                {
                    subcell_extremes[1] = Some(index);
                }
                if star.declination_degrees
                    < stars[subcell_extremes[2].unwrap()].declination_degrees
                {
                    subcell_extremes[2] = Some(index);
                }
                if star.declination_degrees
                    > stars[subcell_extremes[3].unwrap()].declination_degrees
                {
                    subcell_extremes[3] = Some(index);
                }
            } else {
                *subcell_extremes = [Some(index); 4];
            }
            let subcell_candidates = &mut candidates[bin_index][subcell];
            if subcell_candidates.len() < STARS_PER_FINE_SUBCELL {
                let spatially_distinct = subcell_candidates.iter().all(|&candidate| {
                    let candidate = &stars[candidate];
                    star.right_ascension_hours != candidate.right_ascension_hours
                        || star.declination_degrees != candidate.declination_degrees
                });
                if spatially_distinct {
                    subcell_candidates.push(index);
                } else if coincident[bin_index][subcell].is_none() {
                    coincident[bin_index][subcell] = Some(index);
                }
            }
        }
        for bin_index in 0..bins.len() {
            for subcell in 0..FINE_SKY_SUBCELLS {
                bins[bin_index].extend(candidates[bin_index][subcell].iter().copied());
                for index in extremes[bin_index][subcell].iter().flatten() {
                    if !bins[bin_index].contains(index) {
                        bins[bin_index].push(*index);
                    }
                }
                if candidates[bin_index][subcell].len() < STARS_PER_FINE_SUBCELL
                    && let Some(index) = coincident[bin_index][subcell]
                    && !bins[bin_index].contains(&index)
                {
                    bins[bin_index].push(index);
                }
            }
        }
        bins
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
    let (right_ascension, declination) = precess_j2000(
        (star.right_ascension_hours * 15.0).to_radians(),
        star.declination_degrees.to_radians(),
        julian_date,
    );
    let gmst_degrees = greenwich_mean_sidereal_degrees(julian_date);
    let local_sidereal = (gmst_degrees + longitude_degrees)
        .rem_euclid(360.0)
        .to_radians();
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

fn greenwich_mean_sidereal_degrees(julian_date: f64) -> f64 {
    let centuries = (julian_date - 2_451_545.0) / 36_525.0;
    280.460_618_37
        + 360.985_647_366_29 * (julian_date - 2_451_545.0)
        + 0.000_387_933 * centuries * centuries
        - centuries * centuries * centuries / 38_710_000.0
}

/// IAU 1976 precession from mean equatorial J2000 coordinates to the date.
fn precess_j2000(right_ascension: f64, declination: f64, julian_date: f64) -> (f64, f64) {
    let centuries = (julian_date - 2_451_545.0) / 36_525.0;
    let t2 = centuries * centuries;
    let t3 = t2 * centuries;
    let arcseconds_to_radians = std::f64::consts::PI / (180.0 * 3_600.0);
    let zeta = (2_306.218_1 * centuries + 0.301_88 * t2 + 0.017_998 * t3) * arcseconds_to_radians;
    let z = (2_306.218_1 * centuries + 1.094_68 * t2 + 0.018_203 * t3) * arcseconds_to_radians;
    let theta = (2_004.310_9 * centuries - 0.426_65 * t2 - 0.041_833 * t3) * arcseconds_to_radians;
    let a = right_ascension + zeta;
    let sin_declination = declination.sin();
    let cos_declination = declination.cos();
    let x = cos_declination * a.sin();
    let y = theta.cos() * cos_declination * a.cos() - theta.sin() * sin_declination;
    let zed = theta.sin() * cos_declination * a.cos() + theta.cos() * sin_declination;
    (
        (x.atan2(y) + z).rem_euclid(std::f64::consts::TAU),
        zed.clamp(-1.0, 1.0).asin(),
    )
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
    _frame: ControlFrame,
    elapsed: f32,
    aspect: f32,
    viewport: (u32, u32),
    state: &SceneState,
    instances: &mut Vec<Instance>,
) {
    let Some(config) = state.starfield() else {
        return;
    };
    let unix_seconds = config.epoch_at(elapsed);
    let mut visible = Vec::with_capacity(MAX_RENDERED_STARS.min(profile.instance_count) * 4);
    for (bin_index, bin) in config.catalog.candidate_bins.iter().enumerate() {
        if bin.is_empty() {
            continue;
        }
        let dec_bin = bin_index / RIGHT_ASCENSION_BINS;
        let ra_bin = bin_index % RIGHT_ASCENSION_BINS;
        let cell_center = Star {
            right_ascension_hours: (ra_bin as f64 + 0.5) * 24.0 / RIGHT_ASCENSION_BINS as f64,
            declination_degrees: (dec_bin as f64 + 0.5) * 180.0 / DECLINATION_BINS as f64 - 90.0,
            magnitude: 0.0,
            colour_index: None,
            name: None,
        };
        // A 13-degree margin covers each cell's angular radius plus J2000
        // precession across the supported date range.
        if horizontal_coordinates(
            &cell_center,
            config.latitude_degrees,
            config.longitude_degrees,
            unix_seconds,
        )
        .1 < -13.0_f64.to_radians()
        {
            continue;
        }
        for (rank, &star_index) in bin.iter().enumerate() {
            let star = &config.catalog.stars[star_index];
            let Some(center) = project_star(
                star,
                config.latitude_degrees,
                config.longitude_degrees,
                unix_seconds,
            ) else {
                continue;
            };
            visible.push((star, center, bin_index, rank, star_index));
        }
    }

    // The catalog's magnitude-sorted spatial bins bound the work and preserve
    // candidates across the whole sky. Preserve one candidate per visible
    // screen-space quarter before filling the remaining budget globally, so
    // a dense bright region cannot erase the rest of the panorama.
    visible.sort_by(
        |(left, _, left_bin, left_rank, _), (right, _, right_bin, right_rank, _)| {
            left.magnitude
                .total_cmp(&right.magnitude)
                .then_with(|| left_rank.cmp(right_rank))
                .then_with(|| spatial_order_key(*left_bin).cmp(&spatial_order_key(*right_bin)))
        },
    );
    let candidate_limit = MAX_RENDERED_STARS;
    let mut candidate_order = Vec::with_capacity(candidate_limit);
    let mut reserved = vec![false; visible.len()];
    for quarter in 0..4 {
        if candidate_order.len() == candidate_limit {
            break;
        }
        if let Some((index, _)) = visible.iter().enumerate().find(|(index, candidate)| {
            !reserved[*index] && screen_quarter(candidate.1[0]) == quarter
        }) {
            reserved[index] = true;
            candidate_order.push(index);
        }
    }
    for index in 0..visible.len() {
        if candidate_order.len() == candidate_limit {
            break;
        }
        if !reserved[index] {
            candidate_order.push(index);
        }
    }
    let mut selected = Vec::with_capacity(MAX_RENDERED_STARS.min(profile.instance_count));
    let mut selected_instances = 0;
    for index in candidate_order {
        let (star, center, _, _, _) = visible[index];
        if selected.len() >= MAX_RENDERED_STARS {
            break;
        }
        let brightness = star_brightness(star.magnitude);
        let tint = star
            .colour_index
            .map(color_index_rgb)
            .unwrap_or([1.0, 1.0, 1.0]);
        // The size curve preserves magnitude ordering while retaining a
        // two-pixel minimum footprint at small output sizes.
        let size_pixels = 2.0 + 1.25 * (brightness / MAX_STAR_LINEAR_LUMINANCE).sqrt();
        let point_size = 2.0 * size_pixels / viewport.1.max(1) as f32;
        let size = [point_size / aspect.max(f32::MIN_POSITIVE), point_size];
        let instance = Instance {
            center: [center[0], center[1], 0.0],
            size,
            color: [
                tint[0] * brightness,
                tint[1] * brightness,
                tint[2] * brightness,
                1.0,
            ],
            rotation: 0.0,
            scene_mix: 0.0,
            pattern: [0.0; 2],
        };
        let wrapped = crosses_horizontal_seam(instance);
        if let Some(draw_wrap) = seam_wrap_within_budget(
            wrapped,
            profile.instance_count.saturating_sub(selected_instances),
        ) {
            selected.push((star.magnitude, instance, draw_wrap));
            selected_instances += if draw_wrap { 2 } else { 1 };
        }
    }
    selected.sort_by(|(left, _, _), (right, _, _)| left.total_cmp(right));
    for (_, instance, wrapped) in selected.into_iter().rev() {
        append_wrapped(instance, wrapped, profile.instance_count, instances);
    }
}

fn spatial_order_key(bin_index: usize) -> usize {
    (bin_index * 383) % (RIGHT_ASCENSION_BINS * DECLINATION_BINS)
}

fn screen_quarter(x: f32) -> usize {
    (((x + 1.0) * 2.0).floor() as usize).min(3)
}

fn crosses_horizontal_seam(instance: Instance) -> bool {
    let half_width = instance.size[0] / 2.0;
    instance.center[0] - half_width < -1.0 || instance.center[0] + half_width > 1.0
}

fn seam_wrap_within_budget(wraps: bool, available: usize) -> Option<bool> {
    if wraps {
        (available >= 2).then_some(true)
    } else if available > 0 {
        Some(false)
    } else {
        None
    }
}

fn append_wrapped(instance: Instance, wraps: bool, budget: usize, instances: &mut Vec<Instance>) {
    if instances.len() + usize::from(wraps) + 1 > budget {
        return;
    }
    let offset = if instance.center[0] < 0.0 { 2.0 } else { -2.0 };
    instances.push(instance);
    if wraps {
        let mut wrapped = instance;
        wrapped.center[0] += offset;
        instances.push(wrapped);
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
            ("ra,dec,mag,dist\n,2,1,0\n", "ra must be"),
            ("ra,dec,mag,dist\n1,,1,0\n", "dec must be"),
            ("ra,dec,mag,dist\n1,2,nan,0\n", "mag must be finite"),
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
        let parsed_catalog = catalog(
            "proper,ra,dec,mag,ci\nBright,0,90,1.0,0.2\n\"\",1,90,6.5,\nToo faint,2,90,6.51,\n",
        )
        .expect("blank optional fields are allowed");
        assert_eq!(parsed_catalog.stars()[1].name, None);
        assert_eq!(parsed_catalog.stars()[1].colour_index, None);
        let with_distance = catalog("ra,dec,mag,dist\n0,0,1,0\n6,0,2,\n")
            .expect("distance is optional and a blank value is allowed");
        assert_eq!(with_distance.stars().len(), 1);
        assert_eq!(with_distance.stars()[0].right_ascension_hours, 6.0);

        let mut state = SceneState::default();
        state.configure_starfield(Some(StarfieldConfig {
            catalog: Arc::new(parsed_catalog),
            latitude_degrees: 45.0,
            longitude_degrees: 0.0,
            unix_seconds: 946_728_000.0,
            follow_elapsed: false,
        }));
        let mut instances = Vec::new();
        build(
            &RenderProfile::pi5(),
            ControlFrame::procedural(0),
            0.0,
            16.0 / 9.0,
            (160, 90),
            &state,
            &mut instances,
        );
        assert!(instances.len() >= 2);
        assert!(instances.len() <= 4);
        let brightest = instances
            .iter()
            .map(|instance| instance.color[0])
            .fold(0.0_f32, f32::max);
        let faintest = instances
            .iter()
            .map(|instance| instance.color[0])
            .fold(f32::INFINITY, f32::min);
        assert!(brightest > faintest);
        assert!(
            instances
                .iter()
                .all(|instance| instance.size[1] * 90.0 / 2.0 >= 2.0)
        );
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

    #[test]
    fn precession_matches_iau_1976_reference_and_changes_2100_horizon_classification() {
        let julian_date = 2_451_545.0 + 36_525.0;
        let (ra, dec) = precess_j2000(0.0, 0.0, julian_date);
        assert!((ra.to_degrees() / 15.0 - 0.085_444).abs() < 1.0e-5);
        assert!((dec.to_degrees() - 0.556_588).abs() < 1.0e-5);

        let star = Star {
            right_ascension_hours: 0.0,
            declination_degrees: 0.0,
            magnitude: 1.0,
            colour_index: None,
            name: None,
        };
        // 2100-01-01 at the equator: date precession moves this star just
        // above the horizon where its unprecessed J2000 position is below.
        assert!(
            project_star(&star, 0.0, -9.5, 4_102_444_800.0).is_some(),
            "supported-century precession must affect horizon visibility"
        );
    }

    #[test]
    fn live_epoch_advances_but_explicit_epoch_stays_fixed() {
        let catalog = Arc::new(catalog("ra,dec,mag\n6,0,1\n").unwrap());
        let live = StarfieldConfig {
            catalog: Arc::clone(&catalog),
            latitude_degrees: 0.0,
            longitude_degrees: 0.0,
            unix_seconds: 1_000.0,
            follow_elapsed: true,
        };
        let fixed = StarfieldConfig {
            follow_elapsed: false,
            ..live.clone()
        };
        assert_eq!(live.epoch_at(9.9), 1_000.0);
        assert_eq!(live.epoch_at(12.5), 1_010.0);
        assert_eq!(fixed.epoch_at(12.5), 1_000.0);
    }

    #[test]
    fn utc_clock_resynchronizes_to_wall_clock_at_snapshot_cadence() {
        let mut clock = UtcClock::new(1_000.0);
        assert_eq!(clock.update(1_000.1, 0.1), 1_000.1);
        assert!(!snapshot_due(9.9, 0.0));
        assert!(snapshot_due(10.0, 0.0));
        assert!(!snapshot_due(10.1, 10.0));

        assert_eq!(clock.update(3_600.0, 10.0), 3_600.0);
        assert_eq!(clock.update(3_600.2, 10.2), 3_600.2);
        assert_eq!(clock.update(0.0, 10.1), 3_600.2);
        assert!(!snapshot_due(19.9, 10.0));
        assert!(snapshot_due(20.0, 10.0));
        assert_eq!(clock.update(0.0, 20.0), 0.0);
        assert_eq!(clock.update(10_000.0, 30.0), 10_000.0);
        assert_eq!(clock.update(f64::NAN, 40.0), 10_000.0);
        assert!(!snapshot_due(f64::NAN, 40.0));
    }

    #[test]
    fn north_seam_wraps_quads_without_exceeding_the_instance_budget() {
        let edge = Instance {
            center: [-0.999, 0.0, 0.0],
            size: [0.03, 0.044],
            color: [1.0; 4],
            rotation: 0.0,
            scene_mix: 0.0,
            pattern: [0.0; 2],
        };
        let mut instances = Vec::new();
        append_wrapped(edge, true, 2, &mut instances);
        assert_eq!(instances.len(), 2);
        assert!(instances[1].center[0] > 1.0);
        let pixels = raster(&instances, (160, 90));
        assert!(luminance(pixels[45 * 160]) > 0.0);
        assert!(luminance(pixels[45 * 160 + 159]) > 0.0);

        let mut limited = Vec::new();
        assert_eq!(seam_wrap_within_budget(true, 1), None);
        if let Some(wrap) = seam_wrap_within_budget(true, 1) {
            append_wrapped(edge, wrap, 1, &mut limited);
        }
        assert!(limited.is_empty(), "a seam star is atomic under the cap");
        let pixels = raster(&limited, (160, 90));
        assert_eq!(luminance(pixels[45 * 160]), 0.0);
        assert_eq!(luminance(pixels[45 * 160 + 159]), 0.0);
        assert_eq!(seam_wrap_within_budget(true, 2), Some(true));
        assert_eq!(seam_wrap_within_budget(false, 1), Some(false));
        assert_eq!(seam_wrap_within_budget(true, 0), None);
    }

    #[test]
    fn coincident_faint_stars_are_drawn_before_the_brightest_star() {
        let catalog = Arc::new(catalog("ra,dec,mag\n0,0,-1\n0,0,6\n").unwrap());
        let mut state = SceneState::default();
        state.configure_starfield(Some(StarfieldConfig {
            catalog,
            latitude_degrees: 45.0,
            longitude_degrees: 80.0,
            unix_seconds: 946_728_000.0,
            follow_elapsed: false,
        }));
        let mut instances = Vec::new();
        build(
            &RenderProfile::pi5(),
            ControlFrame::procedural(6),
            0.0,
            16.0 / 9.0,
            (160, 90),
            &state,
            &mut instances,
        );
        assert_eq!(instances.len(), 2);
        assert!(instances[0].color[0] < instances[1].color[0]);
    }

    fn raster(instances: &[Instance], size: (u32, u32)) -> Vec<[f32; 3]> {
        let (width, height) = size;
        let mut pixels = vec![[0.0; 3]; (width * height) as usize];
        for instance in instances {
            let half_width = instance.size[0] / 2.0;
            let half_height = instance.size[1] / 2.0;
            let x_start = (((instance.center[0] - half_width + 1.0) * 0.5 * width as f32)
                .floor()
                .max(0.0)) as u32;
            let x_end = (((instance.center[0] + half_width + 1.0) * 0.5 * width as f32)
                .ceil()
                .min(width as f32)) as u32;
            let y_start = (((1.0 - instance.center[1] - half_height) * 0.5 * height as f32)
                .floor()
                .max(0.0)) as u32;
            let y_end = (((1.0 - instance.center[1] + half_height) * 0.5 * height as f32)
                .ceil()
                .min(height as f32)) as u32;
            for y in y_start..y_end {
                let ndc_y = 1.0 - 2.0 * (y as f32 + 0.5) / height as f32;
                for x in x_start..x_end {
                    let ndc_x = 2.0 * (x as f32 + 0.5) / width as f32 - 1.0;
                    if (ndc_x - instance.center[0]).abs() <= half_width
                        && (ndc_y - instance.center[1]).abs() <= half_height
                    {
                        pixels[(y * width + x) as usize] =
                            [instance.color[0], instance.color[1], instance.color[2]];
                    }
                }
            }
        }
        pixels
    }

    fn luminance(pixel: [f32; 3]) -> f32 {
        0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2]
    }

    #[test]
    fn dense_sky_publishes_held_snapshots_and_bounds_rendered_pixel_luminance() {
        fn inverse_precess_to_j2000(ra: f64, dec: f64, julian_date: f64) -> (f64, f64) {
            let mut j2000_ra = ra;
            let mut j2000_dec = dec;
            for _ in 0..5 {
                let (date_ra, date_dec) = precess_j2000(j2000_ra, j2000_dec, julian_date);
                let ra_delta = (ra - date_ra + std::f64::consts::PI)
                    .rem_euclid(std::f64::consts::TAU)
                    - std::f64::consts::PI;
                j2000_ra = (j2000_ra + ra_delta).rem_euclid(std::f64::consts::TAU);
                j2000_dec += dec - date_dec;
            }
            (j2000_ra, j2000_dec)
        }

        let base_utc = 1_795_187_247.0;
        let mut csv = String::from("ra,dec,mag\n");
        // Four rise/set pairs, each with a dense line of equal-magnitude
        // stars crossing the horizon between 0.1 and 0.8 seconds.
        for group in 0..8 {
            let crossing_utc = base_utc + (group + 1) as f64 * 0.1;
            let julian_date = crossing_utc / 86_400.0 + 2_440_587.5;
            let hour_angle = if group % 2 == 0 {
                -std::f64::consts::FRAC_PI_2
            } else {
                std::f64::consts::FRAC_PI_2
            };
            let apparent_ra = (greenwich_mean_sidereal_degrees(julian_date).to_radians()
                - hour_angle)
                .rem_euclid(std::f64::consts::TAU);
            for index in 0..1024 {
                let apparent_dec = (-80.0 + index as f64 * 160.0 / 1023.0).to_radians();
                let (ra, dec) = inverse_precess_to_j2000(apparent_ra, apparent_dec, julian_date);
                csv.push_str(&format!(
                    "{},{},-2\n",
                    ra.to_degrees() / 15.0,
                    dec.to_degrees()
                ));
            }
        }
        // A two-pixel-spaced synthetic background grid at a fainter magnitude.
        for dec_index in 0..32 {
            for ra_index in 0..32 {
                let ra = (ra_index as f64 + 0.5) * 24.0 / 32.0;
                let dec = -80.0 + (dec_index as f64 + 0.5) * 160.0 / 32.0;
                csv.push_str(&format!("{ra},{dec},-1.5\n"));
            }
        }
        let catalog = Arc::new(catalog(&csv).unwrap());
        let mut state = SceneState::default();
        state.configure_starfield(Some(StarfieldConfig {
            catalog,
            latitude_degrees: 0.0,
            longitude_degrees: 0.0,
            unix_seconds: base_utc,
            follow_elapsed: true,
        }));
        let profile = RenderProfile::pi5();
        let render = |elapsed| {
            let mut instances = Vec::new();
            build(
                &profile,
                ControlFrame::procedural(6),
                elapsed,
                16.0 / 9.0,
                (160, 90),
                &state,
                &mut instances,
            );
            assert!(instances.len() <= profile.instance_count);
            raster(&instances, (160, 90))
        };
        let initial = render(0.0);
        for frame in 0..=51 {
            assert_eq!(
                initial,
                render(frame as f32 / 60.0),
                "snapshot changed at frame {frame}"
            );
        }
        let updated = render(10.0);
        let max_luminance = initial
            .iter()
            .chain(&updated)
            .map(|&pixel| luminance(pixel))
            .fold(0.0_f32, f32::max);
        let max_change = initial
            .iter()
            .zip(&updated)
            .map(|(&left, &right)| (luminance(left) - luminance(right)).abs())
            .fold(0.0_f32, f32::max);
        let coverage = initial
            .iter()
            .filter(|&&pixel| luminance(pixel) > 0.0)
            .count();
        assert!(coverage > 0, "synthetic catalog must render stars");
        assert!(coverage <= MAX_RENDERED_STARS * 18);
        assert!(coverage < 160 * 90 * 28 / 100);
        for quarter in 0..4 {
            let has_star = initial.iter().enumerate().any(|(index, &pixel)| {
                let x = index % 160;
                x / 40 == quarter && luminance(pixel) > 0.0
            });
            assert!(has_star, "candidate selection lost sky quarter {quarter}");
        }
        assert!(max_luminance <= MAX_STAR_LINEAR_LUMINANCE + 1.0e-6);
        assert!(max_change <= MAX_STAR_LINEAR_LUMINANCE + 1.0e-6);
    }

    #[test]
    fn dense_below_horizon_catalog_has_a_bounded_visible_candidate_set() {
        let mut csv = String::from("ra,dec,mag\n");
        for _ in 0..120_000 {
            csv.push_str("0,-85,1\n");
        }
        let catalog = Arc::new(catalog(&csv).unwrap());
        let candidate_count: usize = catalog.candidate_bins.iter().map(Vec::len).sum();
        assert_eq!(candidate_count, 2);
        assert!(candidate_count <= SKY_CANDIDATE_BOUND);

        let mut state = SceneState::default();
        state.configure_starfield(Some(StarfieldConfig {
            catalog,
            latitude_degrees: 45.0,
            longitude_degrees: 0.0,
            unix_seconds: 946_728_000.0,
            follow_elapsed: false,
        }));
        let mut instances = Vec::new();
        build(
            &RenderProfile::pi5(),
            ControlFrame::procedural(6),
            0.0,
            16.0 / 9.0,
            (160, 90),
            &state,
            &mut instances,
        );
        assert!(instances.is_empty());
    }

    #[test]
    fn visible_faint_star_survives_sixteen_brighter_below_horizon_candidates_in_its_subcell() {
        let unix_seconds = 946_728_000.0;
        let ra_hours = 18.697_374_558;
        let mut csv = String::from("ra,dec,mag\n");
        for index in 0..16 {
            csv.push_str(&format!("{ra_hours},{},5\n", -46.6 + 0.06 * index as f64));
        }
        csv.push_str(&format!("{ra_hours},-45.2,6\n"));
        let catalog = Arc::new(catalog(&csv).unwrap());
        let target_index = catalog
            .stars
            .iter()
            .position(|star| star.declination_degrees == -45.2)
            .unwrap();
        assert!(
            catalog
                .candidate_bins
                .iter()
                .any(|bin| bin.contains(&target_index)),
            "a visible boundary candidate must survive the bounded index"
        );
        let target = &catalog.stars[target_index];
        let projected = project_star(target, 44.6, 0.0, unix_seconds)
            .expect("review fixture target is above the horizon");
        for star in catalog.stars.iter().filter(|star| star.magnitude == 5.0) {
            assert!(
                project_star(star, 44.6, 0.0, unix_seconds).is_none(),
                "fixture's brighter candidates should be below the horizon"
            );
        }
        let mut state = SceneState::default();
        state.configure_starfield(Some(StarfieldConfig {
            catalog,
            latitude_degrees: 44.6,
            longitude_degrees: 0.0,
            unix_seconds,
            follow_elapsed: false,
        }));
        let mut instances = Vec::new();
        build(
            &RenderProfile::pi5(),
            ControlFrame::procedural(6),
            0.0,
            16.0 / 9.0,
            (160, 90),
            &state,
            &mut instances,
        );
        assert!(
            instances.iter().any(|instance| {
                (instance.center[0] - projected[0]).abs() < 1.0e-5
                    && (instance.center[1] - projected[1]).abs() < 1.0e-5
            }),
            "the only visible target must survive actual scene construction"
        );
    }

    #[test]
    fn magnitude_brightness_and_size_keep_order_without_exceeding_output_caps() {
        let magnitudes = [-2.0, 0.0, 1.0, 5.0, 6.0, 6.5];
        let brightness: Vec<_> = magnitudes
            .iter()
            .map(|&magnitude| star_brightness(magnitude))
            .collect();
        let sizes: Vec<_> = brightness
            .iter()
            .map(|&value| 2.0 + 1.25 * (value / MAX_STAR_LINEAR_LUMINANCE).sqrt())
            .collect();
        assert!(brightness[0] <= MAX_STAR_LINEAR_LUMINANCE);
        assert!(brightness[0] > brightness[1]);
        assert!(brightness.windows(2).all(|pair| pair[0] > pair[1]));
        assert!(sizes.windows(2).all(|pair| pair[0] > pair[1]));
        assert!(*brightness.last().unwrap() >= 0.000_55);
        assert!(*sizes.last().unwrap() >= 2.0);

        let csv = format!(
            "ra,dec,mag\n{}",
            magnitudes
                .iter()
                .enumerate()
                .map(|(index, magnitude)| format!("{},60,{magnitude}\n", index as f64 + 0.1))
                .collect::<String>()
        );
        let mut state = SceneState::default();
        state.configure_starfield(Some(StarfieldConfig {
            catalog: Arc::new(catalog(&csv).unwrap()),
            latitude_degrees: 90.0,
            longitude_degrees: 0.0,
            unix_seconds: 946_728_000.0,
            follow_elapsed: false,
        }));
        let mut instances = Vec::new();
        build(
            &RenderProfile::pi5(),
            ControlFrame::procedural(6),
            0.0,
            16.0 / 9.0,
            (160, 90),
            &state,
            &mut instances,
        );
        assert_eq!(instances.len(), magnitudes.len());
        assert!(
            instances
                .windows(2)
                .all(|pair| pair[0].color[0] < pair[1].color[0])
        );
        assert!(
            instances
                .windows(2)
                .all(|pair| pair[0].size[1] < pair[1].size[1])
        );
    }

    #[test]
    fn brightness_is_invariant_to_audio_even_for_dense_overlap_at_multiple_rates() {
        let mut csv = String::from("ra,dec,mag\n");
        for _ in 0..16_500 {
            csv.push_str("0,0,1\n");
        }
        let catalog = Arc::new(catalog(&csv).unwrap());
        let mut state = SceneState::default();
        state.configure_starfield(Some(StarfieldConfig {
            catalog,
            latitude_degrees: 0.0,
            longitude_degrees: 0.0,
            unix_seconds: 946_728_000.0,
            follow_elapsed: false,
        }));
        for profile in [RenderProfile::m1(), RenderProfile::pi5()] {
            let mut reference = None;
            for fps in [60.0, 120.0, 240.0] {
                for energy in [0, 255, 0, 255] {
                    let mut frame = ControlFrame::procedural(6);
                    frame.energy = energy;
                    let mut instances = Vec::new();
                    build(
                        &profile,
                        frame,
                        1.0 / fps,
                        16.0 / 9.0,
                        (160, 90),
                        &state,
                        &mut instances,
                    );
                    assert!(instances.len() <= profile.instance_count);
                    let colors: Vec<_> = instances.iter().map(|instance| instance.color).collect();
                    if let Some(reference) = &reference {
                        assert_eq!(&colors, reference, "energy={energy}, fps={fps}");
                    } else {
                        reference = Some(colors);
                    }
                }
            }
        }
    }
}
