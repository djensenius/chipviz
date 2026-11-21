//! CLI-only catalog-v1 acquisition, preflight and ordered replay.

use super::*;
use crate::catalog_v1::{Record, Snapshot};
use std::io::Read;

const MAX_BYTES: usize = 20 + 145 * 1_000_000;

// Inspect tokens before removing the flag: it must never be swallowed as a
// value, including when it follows the missing value.
pub(super) fn cli_tokens(tokens: Vec<String>) -> Result<(Vec<String>, bool), String> {
    if !tokens.iter().any(|token| token == "--catalog-v1") {
        return Ok((tokens, false));
    }
    let mut result = Vec::new();
    let mut input = false;
    let mut iter = tokens.into_iter();
    while let Some(token) = iter.next() {
        if token == "--catalog-v1" {
            continue;
        }
        if matches!(
            token.as_str(),
            "--scene"
                | "--house"
                | "--tunnel-variant"
                | "--starfield"
                | "--catalog"
                | "--star-catalog"
                | "--location"
                | "--time"
                | "--follow-time"
        ) {
            return Err(format!("{token} is incompatible with --catalog-v1"));
        }
        let takes_value = matches!(
            token.as_str(),
            "--profile" | "--frames" | "--input" | "--size" | "--out" | "--mic-layers"
        );
        if takes_value {
            let value = iter
                .next()
                .ok_or_else(|| format!("{token} requires a value"))?;
            if is_cli_option(&value) {
                return Err(format!(
                    "{token} requires a value; {value} is an option (use ./{value} for a literal path)"
                ));
            }
            input |= token == "--input";
            result.push(token);
            result.push(value);
        } else {
            result.push(token);
        }
    }
    if !input
        && !result
            .iter()
            .any(|token| token == "--help" || token == "-h")
    {
        return Err("--catalog-v1 requires --input FILE".into());
    }
    Ok((result, true))
}

fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    let same = a.is_file()
        && b.is_file()
        && a.len() == b.len()
        && matches!((a.modified(), b.modified()), (Ok(a), Ok(b)) if a == b);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        same && a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.mtime() == b.mtime()
            && a.mtime_nsec() == b.mtime_nsec()
            && a.ctime() == b.ctime()
            && a.ctime_nsec() == b.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        same
    }
}

#[cfg(unix)]
fn open_source(path: &Path) -> Result<std::fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(not(unix))]
fn open_source(_path: &Path) -> Result<std::fs::File, String> {
    Err("--catalog-v1 is supported only on Unix".into())
}

fn acquire(path: &Path) -> Result<(Vec<u8>, std::fs::Metadata), String> {
    acquire_checked(path, || {})
}

fn acquire_checked(
    path: &Path,
    after_read: impl FnOnce(),
) -> Result<(Vec<u8>, std::fs::Metadata), String> {
    let error = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let mut file = open_source(path)?;
    let before = file.metadata().map_err(|e| error(&e))?;
    let named = std::fs::metadata(path).map_err(|e| error(&e))?;
    if !same_file(&before, &named) {
        return Err(format!(
            "{}: source changed or is not a regular file",
            path.display()
        ));
    }
    if before.len() > MAX_BYTES as u64 {
        return Err(format!("{}: catalog-v1 input is too large", path.display()));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(before.len() as usize)
        .map_err(|e| error(&e))?;
    let mut buffer = [0u8; 8192];
    loop {
        let count = file.read(&mut buffer).map_err(|e| error(&e))?;
        if count == 0 {
            break;
        }
        if count > MAX_BYTES - bytes.len() {
            return Err(format!("{}: catalog-v1 input is too large", path.display()));
        }
        bytes.try_reserve_exact(count).map_err(|e| error(&e))?;
        bytes.extend_from_slice(&buffer[..count]);
    }
    after_read();
    let after = file.metadata().map_err(|e| error(&e))?;
    let named_after = std::fs::metadata(path).map_err(|e| error(&e))?;
    if !same_file(&before, &after)
        || !same_file(&before, &named_after)
        || bytes.len() as u64 != before.len()
    {
        return Err(format!(
            "{}: source changed during acquisition",
            path.display()
        ));
    }
    Ok((bytes, before))
}

// Output and input directories must not be concurrently modified. Compare
// file identity, not size or timestamps: hard links share the input inode.
fn reject_source_output(path: &Path, source: &std::fs::Metadata) -> Result<(), String> {
    match std::fs::metadata(path) {
        Ok(output) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if output.dev() == source.dev() && output.ino() == source.ino() {
                    return Err(format!(
                        "{}: output aliases catalog-v1 input",
                        path.display()
                    ));
                }
                Ok(())
            }
            #[cfg(not(unix))]
            {
                let _ = (output, source);
                Err("--catalog-v1 is supported only on Unix".into())
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn write_catalog_png(
    outputs: &mut OutputFiles,
    path: &Path,
    source: &std::fs::Metadata,
    width: u32,
    height: u32,
    rgb: &[u8],
) -> Result<(), String> {
    reject_source_output(path, source)?;
    write_png(outputs, path, width, height, rgb)
}

fn write_catalog_manifest(
    outputs: &mut OutputFiles,
    path: &Path,
    source: &std::fs::Metadata,
    manifest: &[u8],
) -> Result<(), String> {
    reject_source_output(path, source)?;
    write_manifest(outputs, path, manifest)
}

fn preflight(snapshot: &Snapshot, frames: &[u64]) -> Result<(), String> {
    if frames.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("--catalog-v1 frames must be strictly increasing".into());
    }
    if frames.iter().any(|&frame| frame >= snapshot.count() as u64) {
        return Err("--catalog-v1 frame is past the input".into());
    }
    // Both the byte and the actual registry entry must coincide. Do not rely
    // on modulo dispatch to turn an unavailable catalog family into another.
    let names = [
        "SignalField",
        "SpectrumVolume",
        "PulseTunnel",
        "Lines",
        "Static",
        "Object",
    ];
    for (family, name) in names.iter().enumerate() {
        if scenes::normalize_scene_byte(family as u8, false) != family as u8
            || scenes::REGISTRY.get(family).map(|entry| entry.name) != Some(*name)
            || scenes::dispatch_configured(family as u8, false).name() != *name
        {
            return Err("catalog-v1 registry identity mismatch".into());
        }
    }
    for index in 0..snapshot.count() {
        let record = snapshot.record(index).expect("validated index");
        if record.family() > 5 || record.variant() != 0 {
            return Err(format!(
                "record {index}: unavailable catalog family {} variant {}",
                record.family(),
                record.variant()
            ));
        }
        let frame = ControlFrame::parse_wire(&record.envelope()[24..57])
            .map_err(|e| format!("record {index}: {e}"))?;
        if frame.scene != record.family() {
            return Err(format!("record {index}: family identity mismatch"));
        }
    }
    Ok(())
}

fn load(path: &Path, frames: &[u64]) -> Result<(Snapshot, std::fs::Metadata), String> {
    let (bytes, source) = acquire(path)?;
    let snapshot = Snapshot::validate(&bytes)
        .map_err(|e| format!("{}: catalog-v1 validation: {e}", path.display()))?;
    drop(bytes);
    preflight(&snapshot, frames)?;
    Ok((snapshot, source))
}

// Called only after full snapshot and requested-frame preflight. The visitor
// is the sole replay path for both drawn and state-only frames.
fn replay_records<'a>(
    snapshot: &'a Snapshot,
    frames: &[u64],
    mut visit: impl FnMut(usize, Record<'a>, bool) -> Result<(), String>,
) -> Result<(), String> {
    let Some(&last) = frames.last() else {
        return Ok(());
    };
    let last = usize::try_from(last).map_err(|_| "--catalog-v1 frame is past the input")?;
    if last >= snapshot.count() {
        return Err("--catalog-v1 frame is past the input".into());
    }
    let mut requested = frames.iter().copied().peekable();
    for index in 0..=last {
        let is_requested = requested.peek() == Some(&(index as u64));
        if is_requested {
            requested.next();
        }
        visit(
            index,
            snapshot.record(index).expect("validated index"),
            is_requested,
        )?;
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut text, "{byte:02x}").expect("writing to a string");
    }
    text
}

pub(super) fn run(args: &CaptureArgs, exclusive: bool) -> Result<(), String> {
    let Input::File(path) = &args.input else {
        return Err("--catalog-v1 requires --input FILE, not procedural or synth-mic".into());
    };
    let (snapshot, source) = load(path, &args.frames)?;
    let manifest_path = args.out.join("manifest.json");
    reject_source_output(&manifest_path, &source)?;
    std::fs::create_dir_all(&args.out).map_err(|e| format!("{}: {e}", args.out.display()))?;
    let (width, height) = args.size;
    let mut renderer =
        pollster::block_on(OffscreenRenderer::new(args.profile.clone(), width, height))
            .map_err(|e| e.to_string())?;
    let layers = mic_layers::allowed(args.mic_layers);
    if layers != args.mic_layers {
        eprintln!("{}", mic_layers::NO_LAYERS_NOTICE);
    }
    let pipeline = renderer.pipeline_mut();
    pipeline.configure_colour_field(colour_field::Config {
        palette: colour_field::PaletteSource::Wire,
        ..colour_field::Config::default()
    });
    pipeline.set_mic_layers(layers);
    pipeline.configure_scenes(SceneConfig {
        lines_drop_cuts: args.lines_drop_cuts,
        tunnel_variant: TunnelVariant::Original,
    });
    pipeline.configure_starfield(None);
    pipeline.enable_starfield_dispatch(false);
    let mut captures = Vec::new();
    let mut outputs = OutputFiles::new(exclusive);
    replay_records(&snapshot, &args.frames, |index, record, is_requested| {
        let frame = ControlFrame::parse_wire(&record.envelope()[24..57])
            .map_err(|e| format!("record {index}: {e}"))?;
        let time = elapsed(index as u64);
        if is_requested {
            let rgb = renderer.render(frame, time).map_err(|e| e.to_string())?;
            let displayed = renderer.pipeline_mut().committed_displayed_scene();
            let entry = scenes::dispatch_configured(displayed, false);
            let file = format!("frame{index:06}-{}.png", entry.name());
            let output = args.out.join(&file);
            write_catalog_png(&mut outputs, &output, &source, width, height, &rgb)?;
            println!("wrote {}", args.out.join(&file).display());
            captures.push(format!(
                "    {{\"file\": {}, \"frame_index\": {index}, \"elapsed\": {time}, \"scene\": {}, \"displayed_scene\": {displayed}, \"scene_name\": {}, \"family\": {}, \"variant\": {}, \"sequence\": {}, \"tick\": {}, \"metadata_hex\": {}, \"envelope_hex\": {}}}",
                json_string(&file), frame.scene, json_string(entry.name()), record.family(),
                record.variant(), record.sequence(), record.tick(), json_string(&hex(record.metadata())),
                json_string(&hex(record.envelope()))
            ));
        } else {
            let size = renderer.size();
            renderer.pipeline_mut().advance_state(frame, time, size);
        }
        Ok(())
    })?;
    let manifest = format!(
        "{{\n  \"profile\": {},\n  \"input\": {},\n  \"input_mode\": \"catalog-v1\",\n  \"mic_layers\": {},\n  \"width\": {width},\n  \"height\": {height},\n  \"fps\": {CAPTURE_FPS},\n  \"captures\": [\n{}\n  ]\n}}\n",
        json_string(args.profile.name),
        json_string(&path.display().to_string()),
        json_string(layers.name()),
        captures.join(",\n")
    );
    write_catalog_manifest(&mut outputs, &manifest_path, &source, manifest.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        !crc
    }

    fn fixture(families: &[u8]) -> Vec<u8> {
        let mut bytes = b"CVXE\x01\x01\0\0".to_vec();
        bytes.extend_from_slice(&(families.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&crc32(&bytes).to_le_bytes());
        for (index, &family) in families.iter().enumerate() {
            let mut record = [0u8; 145];
            record[..6].copy_from_slice(b"CVX1\x01\x01");
            record[8..16].copy_from_slice(&(index as u64).to_le_bytes());
            record[16..24].copy_from_slice(&(index as u64).to_le_bytes());
            record[24] = 0xc7;
            record[26..28].copy_from_slice(&(index as u16).to_le_bytes());
            let frame = ControlFrame::procedural(index as u64);
            record[28] = frame.bpm;
            record[29] = frame.beat_phase;
            record[30..32].copy_from_slice(&frame.beat_count.to_le_bytes());
            record[32..36].copy_from_slice(&[frame.energy, frame.bass, frame.mid, frame.treble]);
            record[36..44].copy_from_slice(&frame.spectrum);
            record[44] = family;
            record[45] = frame.palette;
            record[46] = frame.flags;
            record[56] = record[24..56].iter().fold(0, |a, b| a ^ b);
            let crc = crc32(&record[..141]);
            record[141..].copy_from_slice(&crc.to_le_bytes());
            bytes.extend_from_slice(&record);
        }
        bytes
    }

    #[cfg(unix)]
    fn root() -> PathBuf {
        use std::os::unix::fs::DirBuilderExt;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "chipviz-catalog-capture-{}-{nonce}",
            std::process::id()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        path
    }

    #[test]
    #[cfg(unix)]
    fn pairs_transitions_and_identity_survive_owned_snapshot() {
        let path = root();
        let file = path.join("input");
        let families: Vec<u8> = (0..6)
            .flat_map(|from| (0..6).flat_map(move |to| [from, to]))
            .collect();
        let bytes = fixture(&families);
        std::fs::write(&file, &bytes).unwrap();
        let (snapshot, _) = load(&file, &[0, 4, 71]).unwrap();
        std::fs::write(&file, fixture(&[5])).unwrap();
        for index in 0..snapshot.count() {
            let record = snapshot.record(index).unwrap();
            assert_eq!(
                record.envelope(),
                &bytes[20 + 145 * index..20 + 145 * (index + 1)]
            );
            assert_eq!(record.metadata(), &[0; 84]);
            assert_eq!(record.sequence(), index as u64);
            assert_eq!(record.tick(), index as u64);
            assert_eq!(record.family(), bytes[20 + 145 * index + 44]);
            assert_eq!(record.variant(), 0);
        }
        assert!(preflight(&snapshot, &[1, 1]).is_err());
        assert!(preflight(&snapshot, &[2, 1]).is_err());
        assert!(preflight(&snapshot, &[72]).is_err());
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn replay_visitor_stops_at_last_requested_record() {
        let families = [0, 1, 2, 3, 4, 5];
        let snapshot = Snapshot::validate(&fixture(&families)).unwrap();
        for (frames, expected) in [
            (&[0][..], &[(0, true)][..]),
            (
                &[1, 3][..],
                &[(0, false), (1, true), (2, false), (3, true)][..],
            ),
            (
                &[5][..],
                &[
                    (0, false),
                    (1, false),
                    (2, false),
                    (3, false),
                    (4, false),
                    (5, true),
                ][..],
            ),
            (&[][..], &[][..]),
        ] {
            preflight(&snapshot, frames).unwrap();
            let mut observed = Vec::new();
            replay_records(&snapshot, frames, |index, record, requested| {
                assert_eq!(record.sequence(), index as u64);
                assert_eq!(record.family(), families[index]);
                observed.push((index, requested));
                Ok(())
            })
            .unwrap();
            assert_eq!(observed, expected);
        }
        let unavailable = Snapshot::validate(&fixture(&[0, 1, 2, 3, 4, 7])).unwrap();
        assert!(preflight(&unavailable, &[0]).is_err());
        let mut malformed = fixture(&families);
        malformed[20 + 145 * 5 + 50] ^= 1;
        assert!(Snapshot::validate(&malformed).is_err());
    }

    #[test]
    fn complete_preflight_rejects_late_unavailable_and_malformed_records() {
        for family in 6..=13 {
            let bytes = fixture(&[0, family]);
            match Snapshot::validate(&bytes) {
                Ok(snapshot) => assert!(preflight(&snapshot, &[0]).is_err(), "{family}"),
                Err(_) => assert_eq!(family, 6), // Starfield metadata is required.
            }
            let mut star = fixture(&[0, 6]);
            let start = 20 + 145;
            star[start + 57..start + 89].fill(1);
            star[start + 89..start + 97].copy_from_slice(&946_684_800_000i64.to_le_bytes());
            star[start + 109..start + 141].fill(2);
            let crc = crc32(&star[start..start + 141]);
            star[start + 141..start + 145].copy_from_slice(&crc.to_le_bytes());
            assert!(matches!(
                Snapshot::validate(&star),
                Err(crate::catalog_v1::ValidationError::StarfieldUnavailable)
            ));
        }
        let mut bytes = fixture(&[0, 2]);
        bytes[20 + 145 + 6] = 1;
        let start = 20 + 145;
        let crc = crc32(&bytes[start..start + 141]);
        bytes[start + 141..start + 145].copy_from_slice(&crc.to_le_bytes());
        assert!(preflight(&Snapshot::validate(&bytes).unwrap(), &[0]).is_err());
        bytes[start + 50] ^= 1;
        assert!(Snapshot::validate(&bytes).is_err());
        let valid = fixture(&[0, 1]);
        assert!(Snapshot::validate(&valid[..valid.len() - 1]).is_err());
        let mut trailing = valid.clone();
        trailing.push(0);
        assert!(Snapshot::validate(&trailing).is_err());
        let mut wrong_mode = valid;
        wrong_mode[5] = 0;
        assert!(Snapshot::validate(&wrong_mode).is_err());
        let legacy_frame = &wrong_mode[20 + 24..20 + 57];
        assert!(Snapshot::validate(legacy_frame).is_err());
        let mut out_of_order = fixture(&[0, 1]);
        let start = 20 + 145;
        out_of_order[start + 8..start + 16].copy_from_slice(&0u64.to_le_bytes());
        let crc = crc32(&out_of_order[start..start + 141]);
        out_of_order[start + 141..start + 145].copy_from_slice(&crc.to_le_bytes());
        assert!(Snapshot::validate(&out_of_order).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn acquisition_bounds_and_preflight_have_no_output_effects() {
        let root = root();
        let input = root.join("input");
        let out = root.join("out");
        let mut args = CaptureArgs {
            profile: RenderProfile::pi5(),
            scene: None,
            frames: vec![0],
            input: Input::File(input.clone()),
            size: (16, 16),
            out: out.clone(),
            mic_layers: mic_layers::Mode::Off,
            lines_drop_cuts: false,
        };
        let mut corrupt_late = fixture(&[0, 1]);
        corrupt_late[20 + 145 + 50] ^= 1;
        for bytes in [fixture(&[0, 7]), corrupt_late, vec![0; 20]] {
            std::fs::write(&input, bytes).unwrap();
            assert!(run(&args, false).is_err());
            assert!(!out.exists());
        }
        std::fs::File::create(&input)
            .unwrap()
            .set_len(MAX_BYTES as u64 + 1)
            .unwrap();
        assert!(run(&args, false).unwrap_err().contains("too large"));
        assert!(!out.exists());
        std::fs::write(&input, fixture(&[0])).unwrap();
        args.frames = vec![1];
        assert!(run(&args, false).is_err());
        assert!(!out.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn acquisition_detects_replacement_and_mutation() {
        let root = root();
        let input = root.join("input");
        let replacement = root.join("replacement");
        std::fs::write(&input, fixture(&[0])).unwrap();
        std::fs::write(&replacement, fixture(&[1])).unwrap();
        assert!(
            acquire_checked(&input, || {
                std::fs::rename(&replacement, &input).unwrap();
            })
            .is_err()
        );
        assert!(
            acquire_checked(&input, || {
                std::fs::write(&input, fixture(&[0, 1])).unwrap();
            })
            .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn output_leaves_reject_direct_hardlink_and_symlink_source_aliases() {
        let root = root();
        for name in ["manifest.json", "frame000000-SignalField.png"] {
            for alias in ["direct", "hardlink", "symlink"] {
                for exclusive in [false, true] {
                    let case = root.join(format!("{name}-{alias}-{exclusive}"));
                    std::fs::create_dir(&case).unwrap();
                    let leaf = case.join(name);
                    let input = if alias == "direct" {
                        leaf.clone()
                    } else {
                        case.join("input")
                    };
                    let bytes = fixture(&[0]);
                    std::fs::write(&input, &bytes).unwrap();
                    if alias == "hardlink" {
                        std::fs::hard_link(&input, &leaf).unwrap();
                    } else if alias == "symlink" {
                        std::os::unix::fs::symlink(&input, &leaf).unwrap();
                    }
                    let (_, source) = load(&input, &[0]).unwrap();
                    let mut outputs = OutputFiles::new(exclusive);
                    let error = if name == "manifest.json" {
                        write_catalog_manifest(&mut outputs, &leaf, &source, b"replacement")
                    } else {
                        write_catalog_png(&mut outputs, &leaf, &source, 1, 1, &[1, 2, 3])
                    };
                    assert!(error.unwrap_err().contains("aliases catalog-v1 input"));
                    assert_eq!(std::fs::read(&input).unwrap(), bytes);
                    if name == "manifest.json" {
                        let args = CaptureArgs {
                            profile: RenderProfile::pi5(),
                            scene: None,
                            frames: vec![0],
                            input: Input::File(input.clone()),
                            size: (16, 16),
                            out: case.clone(),
                            mic_layers: mic_layers::Mode::Off,
                            lines_drop_cuts: false,
                        };
                        assert!(
                            run(&args, exclusive)
                                .unwrap_err()
                                .contains("aliases catalog-v1 input")
                        );
                        assert_eq!(std::fs::read(&input).unwrap(), bytes);
                    }
                }
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn catalog_cli_conflicts_and_option_values_in_both_orders() {
        for conflict in [
            "--scene",
            "--house",
            "--tunnel-variant",
            "--starfield",
            "--catalog",
            "--star-catalog",
            "--location",
            "--time",
            "--follow-time",
        ] {
            for tokens in [
                vec!["--catalog-v1", "--input", "in", conflict],
                vec![conflict, "--catalog-v1", "--input", "in"],
            ] {
                assert!(cli_tokens(tokens.into_iter().map(str::to_string).collect()).is_err());
            }
        }
        for option in [
            "--profile",
            "--frames",
            "--input",
            "--size",
            "--out",
            "--mic-layers",
        ] {
            for tokens in [
                vec![option, "--catalog-v1"],
                vec!["--catalog-v1", "--input", "in", option, "--help"],
            ] {
                assert!(cli_tokens(tokens.into_iter().map(str::to_string).collect()).is_err());
            }
        }
        assert!(cli_tokens(vec!["--catalog-v1".into()]).is_err());
        assert!(parse_args(["--out", "shots", "--catalog-v1"].map(str::to_string)).is_err());
        assert!(
            parse_args_with_options(["--out", "shots", "--catalog-v1"].map(str::to_string))
                .is_err()
        );
        assert!(
            parse_args_with_starfield_options(
                ["--out", "shots", "--catalog-v1"].map(str::to_string)
            )
            .is_err()
        );
    }

    #[test]
    #[cfg(unix)]
    #[ignore = "requires an allocated GPU adapter"]
    fn sparse_catalog_stills_match_independent_sequential_renderer() {
        let root = root();
        let input = root.join("input");
        let families: Vec<_> = (0..84).map(|i| [0, 2, 3, 4, 5, 1, 0][i / 12]).collect();
        let bytes = fixture(&families);
        std::fs::write(&input, &bytes).unwrap();
        let snapshot = Snapshot::validate(&bytes).unwrap();
        let mut case_images = Vec::new();
        for (profile, case, mode) in [
            (RenderProfile::m1(), "off", mic_layers::Mode::Off),
            (RenderProfile::m1(), "both", mic_layers::Mode::Both),
            (RenderProfile::pi5(), "off", mic_layers::Mode::Off),
            (RenderProfile::pi5(), "both", mic_layers::Mode::Both),
        ] {
            let args = CaptureArgs {
                profile: profile.clone(),
                scene: None,
                frames: vec![0, 13, 29, 36, 48, 71],
                input: Input::File(input.clone()),
                size: (160, 90),
                out: root.join(format!("{}-{case}", profile.name)),
                mic_layers: mode,
                lines_drop_cuts: true,
            };
            run(&args, true).unwrap();
            let mut reference =
                pollster::block_on(OffscreenRenderer::new(args.profile.clone(), 160, 90)).unwrap();
            reference
                .pipeline_mut()
                .configure_colour_field(colour_field::Config {
                    palette: colour_field::PaletteSource::Wire,
                    ..colour_field::Config::default()
                });
            reference.pipeline_mut().configure_scenes(SceneConfig {
                lines_drop_cuts: true,
                tunnel_variant: TunnelVariant::Original,
            });
            reference
                .pipeline_mut()
                .set_mic_layers(mic_layers::allowed(mode));
            reference.pipeline_mut().enable_starfield_dispatch(false);
            let mut images = Vec::new();
            for index in 0..snapshot.count() {
                let record = snapshot.record(index).unwrap();
                let frame = ControlFrame::parse_wire(&record.envelope()[24..57]).unwrap();
                let rgb = reference.render(frame, elapsed(index as u64)).unwrap();
                if args.frames.contains(&(index as u64)) {
                    let scene = reference.pipeline_mut().committed_displayed_scene();
                    let file = args.out.join(format!(
                        "frame{index:06}-{}.png",
                        scenes::dispatch_configured(scene, false).name()
                    ));
                    let decoder = png::Decoder::new(std::fs::File::open(file).unwrap());
                    let mut reader = decoder.read_info().unwrap();
                    let mut actual = vec![0; reader.output_buffer_size()];
                    let info = reader.next_frame(&mut actual).unwrap();
                    assert_eq!(&actual[..info.buffer_size()], rgb, "frame {index}");
                    images.push(actual[..info.buffer_size()].to_vec());
                }
            }
            let manifest = std::fs::read(args.out.join("manifest.json")).unwrap();
            let manifest_text = std::str::from_utf8(&manifest).unwrap();
            assert!(manifest_text.contains(&format!("\"profile\": \"{}\"", profile.name)));
            let entries: Vec<_> = manifest_text
                .lines()
                .filter(|line| line.contains("\"frame_index\":"))
                .collect();
            assert_eq!(entries.len(), args.frames.len());
            assert_eq!(
                args.frames
                    .iter()
                    .map(|&index| families[index as usize])
                    .collect::<Vec<_>>(),
                [0, 2, 3, 4, 5, 1]
            );
            for &index in &args.frames {
                let record = snapshot.record(index as usize).unwrap();
                let expected_envelope =
                    &bytes[20 + 145 * index as usize..20 + 145 * (index as usize + 1)];
                assert_eq!(record.envelope(), expected_envelope);
                assert_eq!(record.metadata(), &[0; 84]);
                let matching: Vec<_> = entries
                    .iter()
                    .filter(|entry| entry.contains(&format!("\"frame_index\": {index},")))
                    .collect();
                assert_eq!(matching.len(), 1, "frame {index}");
                assert!(
                matching[0].contains(&format!(
                    "\"family\": {}, \"variant\": 0, \"sequence\": {index}, \"tick\": {index}, \"metadata_hex\": \"{}\", \"envelope_hex\": \"{}\"",
                    families[index as usize], hex(&[0; 84]), hex(expected_envelope)
                )),
                "frame {index}: {}",
                matching[0]
            );
            }
            assert!(run(&args, true).is_err());
            assert_eq!(
                std::fs::read(args.out.join("manifest.json")).unwrap(),
                manifest
            );
            let collision_dir = root.join(format!("manifest-collision-{}-{case}", profile.name));
            std::fs::create_dir(&collision_dir).unwrap();
            std::fs::write(collision_dir.join("manifest.json"), b"keep").unwrap();
            let collision_args = CaptureArgs {
                out: collision_dir.clone(),
                frames: vec![0],
                ..args
            };
            assert!(run(&collision_args, true).is_err());
            assert_eq!(
                std::fs::read(collision_dir.join("manifest.json")).unwrap(),
                b"keep"
            );
            assert_eq!(std::fs::read_dir(collision_dir).unwrap().count(), 2);
            case_images.push(images);
        }
        for (profile, images) in ["m1", "pi5"].into_iter().zip(case_images.chunks_exact(2)) {
            if mic_layers::LAYERS_BUILD {
                assert_ne!(
                    images[0], images[1],
                    "{profile}: enabled layers must affect output"
                );
            } else {
                assert_eq!(
                    images[0], images[1],
                    "{profile}: default build must gate layers off"
                );
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
