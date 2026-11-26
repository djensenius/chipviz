use std::path::PathBuf;

use chipviz_modern::{
    AppError, RenderProfile,
    capture::{
        self, CaptureArgs, CaptureOptions, Command, ConfiguredCommand, Input,
        StarfieldCaptureOptions, StarfieldConfiguredCommand, TunnelVariant,
    },
};

fn legacy_args(out: PathBuf) -> CaptureArgs {
    CaptureArgs {
        profile: RenderProfile::pi5(),
        scene: Some(2),
        frames: vec![120],
        input: Input::SynthMic,
        size: (160, 90),
        out,
        mic_layers: Default::default(),
        lines_drop_cuts: false,
    }
}

#[test]
fn legacy_literal_and_parser_remain_usable_from_an_external_crate() {
    let args = legacy_args("shots".into());
    let run: fn(&CaptureArgs) -> Result<(), String> = capture::run;
    let _ = (args, run);
    let parsed = capture::parse_args(["--out".into(), "shots".into()]).unwrap();
    assert!(matches!(parsed, Command::Capture(_)));
    assert!(
        capture::parse_args([
            "--out".into(),
            "shots".into(),
            "--tunnel-variant".into(),
            "hallway".into()
        ])
        .is_err()
    );
}

#[test]
fn configured_parser_and_programmatic_options_expose_variants() {
    let run: fn(&CaptureArgs, CaptureOptions) -> Result<(), String> = capture::run_with_options;
    let _ = run;
    assert_eq!(
        CaptureOptions::default().tunnel_variant,
        TunnelVariant::Original
    );
    let configured = capture::parse_args_with_options(
        ["--out", "shots", "--tunnel-variant", "hallway"].map(str::to_string),
    )
    .unwrap();
    match configured {
        ConfiguredCommand::Capture(args, options) => {
            assert_eq!(args.out, PathBuf::from("shots"));
            assert_eq!(options.tunnel_variant, TunnelVariant::Hallway);
        }
        ConfiguredCommand::Help => panic!("unexpected help"),
    }
    let options = CaptureOptions {
        tunnel_variant: TunnelVariant::Abstract,
    };
    assert_eq!(options.tunnel_variant, TunnelVariant::Abstract);

    let run_starfield: fn(&CaptureArgs, &StarfieldCaptureOptions) -> Result<(), String> =
        capture::run_with_starfield_options;
    let _ = run_starfield;
    let starfield = capture::parse_args_with_starfield_options(
        [
            "--out",
            "shots",
            "--scene",
            "6",
            "--star-catalog",
            "synthetic.csv",
            "--location",
            "0,0",
            "--tunnel-variant",
            "hallway",
        ]
        .map(str::to_string),
    )
    .unwrap();
    assert!(matches!(
        starfield,
        StarfieldConfiguredCommand::Capture(_, _)
    ));
    assert!(matches!(
        capture::parse_args(["--out", "shots", "--scene", "6"].map(str::to_string)).unwrap(),
        Command::Capture(_)
    ));
}

#[test]
fn public_app_error_still_supports_an_external_exhaustive_match() {
    fn downstream_match(error: AppError) -> &'static str {
        match error {
            AppError::Window(_) => "window",
            AppError::EventLoop(_) => "event loop",
            AppError::Io(_) => "io",
            AppError::Renderer(_) => "renderer",
            AppError::Monitor(_) => "monitor",
            AppError::Audio(_) => "audio",
        }
    }

    let error = AppError::Io(std::io::Error::other("expected test error"));
    assert_eq!(downstream_match(error), "io");
}

#[test]
fn capture_cli_legacy_local_mode_literal_paths_allow_help_without_outputs() {
    let root = std::env::temp_dir().join(format!(
        "chipviz-capture-house-literal-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    for args in [
        &["--out", "--house", "--help"][..],
        &["--out", "shots", "--input", "--house", "--help"][..],
        &[
            "--catalog-v1",
            "--input",
            "recording",
            "--out",
            "--house",
            "--help",
        ][..],
        &[
            "--catalog-v1",
            "--out",
            "shots",
            "--input",
            "--house",
            "--help",
        ][..],
        &[
            "--catalog-v1",
            "--exclusive-output",
            "--out",
            "--house",
            "--input",
            "--house",
            "--help",
        ][..],
        &["--exclusive-output", "--out", "--dmg-preview", "--help"][..],
        &[
            "--exclusive-output",
            "--out",
            "shots",
            "--input",
            "--dmg-preview",
            "--help",
        ][..],
        &[
            "--catalog-v1",
            "--input",
            "--dmg-preview",
            "--out",
            "shots",
            "--help",
        ][..],
        &[
            "--catalog-v1",
            "--input",
            "recording",
            "--out",
            "--dmg-preview",
            "--help",
        ][..],
        &[
            "--catalog-v1",
            "--exclusive-output",
            "--out",
            "--dmg-preview",
            "--input",
            "--dmg-preview",
            "--help",
        ][..],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("usage: chipviz-capture"),
            "{args:?}: {output:?}"
        );
        assert!(!root.join("--house").exists());
        assert!(!root.join("--dmg-preview").exists());
        assert!(!root.join("shots").exists());
    }
    std::fs::remove_dir(&root).unwrap();
}

#[test]
fn capture_cli_catalog_rejects_standalone_house_before_outputs() {
    for args in [
        &["--catalog-v1", "--input", "recording", "--house", "--help"][..],
        &["--house", "--catalog-v1", "--input", "recording", "--help"][..],
        &["--catalog-v1", "--input", "--house", "--house", "--help"][..],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("--house is incompatible with --catalog-v1"),
            "{args:?}: {stderr}"
        );
    }
}

#[test]
fn capture_cli_standalone_house_rejects_preview_and_option_values_before_outputs() {
    for args in [
        &["--out", "shots", "--house", "--dmg-preview", "--help"][..],
        &["--out", "shots", "--dmg-preview", "--house", "--help"][..],
        &["--house", "--out", "--dmg-preview", "--help"][..],
        &["--out", "--dmg-preview", "--house", "--help"][..],
        &["--house", "--out", "--help", "--help"][..],
        &["--out", "--help", "--house", "--help"][..],
        &["--out", "--house", "--house", "--help"][..],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("incompatible with --dmg-preview")
                || stderr.contains("--out requires a value"),
            "{args:?}: {stderr}"
        );
    }
}

#[test]
fn capture_cli_rejects_explicit_starfield_without_catalog_or_location() {
    let output_dir = std::env::temp_dir().join(format!(
        "chipviz-starfield-missing-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
        .args(["--starfield", "--out"])
        .arg(&output_dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--starfield requires --star-catalog PATH and --location LAT,LON"),
        "{stderr}"
    );
    assert!(
        !output_dir.exists(),
        "invalid opt-in must fail before output setup"
    );
}

#[test]
fn capture_cli_rejects_exclusive_duplicate_frames_before_gpu_setup() {
    let output_dir = std::env::temp_dir().join(format!(
        "chipviz-capture-duplicates-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
        .args(["--frames", "0,60,0", "--exclusive-output", "--out"])
        .arg(&output_dir)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("duplicate output frame: 0"), "{stderr}");
    assert!(!output_dir.exists());
}

#[test]
#[cfg(unix)]
fn catalog_cli_preflights_late_unavailable_records_and_conflicts() {
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
    let root = std::env::temp_dir().join(format!(
        "chipviz-catalog-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let input = root.join("in.cvxe");
    let out = root.join("out");
    let mut bytes = b"CVXE\x01\x01\0\0".to_vec();
    bytes.extend_from_slice(&2u64.to_le_bytes());
    bytes.extend_from_slice(&crc32(&bytes).to_le_bytes());
    for (index, family) in [0u8, 7].into_iter().enumerate() {
        let mut record = [0u8; 145];
        record[..6].copy_from_slice(b"CVX1\x01\x01");
        record[8..16].copy_from_slice(&(index as u64).to_le_bytes());
        record[16..24].copy_from_slice(&(index as u64).to_le_bytes());
        record[24] = 0xc7;
        record[26..28].copy_from_slice(&(index as u16).to_le_bytes());
        record[44] = family;
        record[56] = record[24..56].iter().fold(0, |a, b| a ^ b);
        let crc = crc32(&record[..141]);
        record[141..].copy_from_slice(&crc.to_le_bytes());
        bytes.extend_from_slice(&record);
    }
    std::fs::write(&input, &bytes).unwrap();
    for first in [true, false] {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"));
        if first {
            command.arg("--catalog-v1");
        }
        command
            .arg("--input")
            .arg(&input)
            .arg("--frames")
            .arg("0")
            .arg("--out")
            .arg(&out);
        if !first {
            command.arg("--catalog-v1");
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("record 1: unavailable"));
        assert!(!out.exists());
    }
    for option in ["--input", "--out"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
            .arg(option)
            .arg("--catalog-v1")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("requires a value"));
        assert!(!out.exists());
    }
    for source in ["procedural", "synth-mic"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
            .args(["--catalog-v1", "--input", source, "--out"])
            .arg(&out)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("requires --input FILE"));
        assert!(!out.exists());
    }
    for conflict in ["--scene", "--tunnel-variant", "--starfield"] {
        for first in [true, false] {
            let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"));
            if first {
                command.arg("--catalog-v1");
            }
            command
                .arg("--input")
                .arg(&input)
                .arg("--out")
                .arg(&out)
                .arg(conflict);
            if !first {
                command.arg("--catalog-v1");
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(2));
            assert!(!out.exists());
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn catalog_cli_rejects_fifo_without_blocking() {
    let root = std::env::temp_dir().join(format!(
        "chipviz-catalog-fifo-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let fifo = root.join("input");
    let out = root.join("out");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
        .arg("--catalog-v1")
        .arg("--input")
        .arg(&fifo)
        .arg("--out")
        .arg(&out)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("capture blocked opening FIFO");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not a regular file"));
    assert!(!out.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn capture_cli_rejects_exclusive_flag_as_a_missing_path_value() {
    let root = std::env::temp_dir().join(format!(
        "chipviz-capture-missing-path-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let output_dir = root.join("existing");
    std::fs::create_dir(&output_dir).unwrap();
    let manifest = output_dir.join("manifest.json");
    std::fs::write(&manifest, b"preserve me").unwrap();

    for option in ["--out", "--input", "--catalog", "--star-catalog"] {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"));
        command.current_dir(&root);
        if option != "--out" {
            command.arg("--out").arg(&output_dir);
        }
        let output = command
            .args([option, "--exclusive-output"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{option}: {stderr}");
        assert!(
            stderr.contains(&format!(
                "{option} requires a value; --exclusive-output is an option"
            )),
            "{option}: {stderr}"
        );
        assert_eq!(std::fs::read(&manifest).unwrap(), b"preserve me");
        assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 1);
        assert!(!root.join("--exclusive-output").exists());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn capture_cli_rejects_known_flags_as_path_values_in_any_order() {
    let root = std::env::temp_dir().join(format!(
        "chipviz-capture-option-value-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let output_dir = root.join("existing");
    std::fs::create_dir(&output_dir).unwrap();
    let manifest = output_dir.join("manifest.json");
    std::fs::write(&manifest, b"preserve me").unwrap();

    for option in ["--out", "--input", "--catalog"] {
        for token in ["--lines-drop-cuts", "--follow-time", "--scene", "--help"] {
            for position in 0..2 {
                let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"));
                command.current_dir(&root);
                if option != "--out" {
                    command.arg("--out").arg(&output_dir);
                }
                if position == 0 {
                    command.arg("--exclusive-output");
                }
                command.args([option, token]);
                if position == 1 {
                    command.arg("--exclusive-output");
                }
                let output = command.output().unwrap();
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert_eq!(
                    output.status.code(),
                    Some(2),
                    "{option} {token}, position {position}: {stderr}"
                );
                assert!(
                    stderr.contains(&format!("{option} requires a value; {token} is an option")),
                    "{option} {token}, position {position}: {stderr}"
                );
                assert_eq!(std::fs::read(&manifest).unwrap(), b"preserve me");
                assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 1);
                assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
            }
        }
    }
    // Without the opt-in, recognized option names remain valid path values.
    // An invalid frame list stops parsing before any output or GPU setup.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
        .current_dir(&root)
        .args([
            "--out",
            "--lines-drop-cuts",
            "--input",
            "--scene",
            "--frames",
            "not-a-number",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("--frames expects indices"), "{stderr}");
    assert_eq!(std::fs::read(&manifest).unwrap(), b"preserve me");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
#[ignore = "requires a GPU adapter"]
fn catalog_cli_preserves_input_when_later_png_aliases_it() {
    use std::os::unix::fs::DirBuilderExt;

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

    let mut recording = b"CVXE\x01\x01\0\0".to_vec();
    recording.extend_from_slice(&2u64.to_le_bytes());
    recording.extend_from_slice(&crc32(&recording).to_le_bytes());
    for index in 0..2u64 {
        let mut record = [0u8; 145];
        record[..6].copy_from_slice(b"CVX1\x01\x01");
        record[8..16].copy_from_slice(&index.to_le_bytes());
        record[16..24].copy_from_slice(&index.to_le_bytes());
        record[24] = 0xc7;
        record[26..28].copy_from_slice(&(index as u16).to_le_bytes());
        record[56] = record[24..56].iter().fold(0, |a, b| a ^ b);
        let crc = crc32(&record[..141]);
        record[141..].copy_from_slice(&crc.to_le_bytes());
        recording.extend_from_slice(&record);
    }
    let root = std::env::temp_dir().join(format!(
        "chipviz-catalog-png-alias-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    for alias in ["direct", "hardlink"] {
        for exclusive in [false, true] {
            let case = root.join(format!("{alias}-{exclusive}"));
            let out = case.join("out");
            std::fs::create_dir_all(&out).unwrap();
            let collision = out.join("frame000001-SignalField.png");
            let input = if alias == "direct" {
                collision.clone()
            } else {
                case.join("recording.cvxe")
            };
            std::fs::write(&input, &recording).unwrap();
            if alias == "hardlink" {
                std::fs::hard_link(&input, &collision).unwrap();
            }
            let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"));
            command
                .args(["--catalog-v1", "--profile", "pi5", "--input"])
                .arg(&input)
                .args(["--frames", "0,1", "--size", "160x90", "--out"])
                .arg(&out);
            if exclusive {
                command.arg("--exclusive-output");
            }
            let output = command.output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(
                output.status.code(),
                Some(1),
                "{alias} {exclusive}: {stderr}"
            );
            assert!(
                stderr.contains("output aliases catalog-v1 input"),
                "{stderr}"
            );
            assert!(
                stderr.contains(&collision.display().to_string()),
                "{stderr}"
            );
            assert_eq!(std::fs::read(&input).unwrap(), recording);
            assert_eq!(std::fs::read(&collision).unwrap(), recording);

            // The first PNG was already written; a later collision does not
            // roll it back, and the manifest has not yet been created.
            let first = std::fs::read(out.join("frame000000-SignalField.png")).unwrap();
            assert!(first.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert!(!out.join("manifest.json").exists());
            assert_eq!(std::fs::read_dir(&out).unwrap().count(), 2);
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires a GPU adapter"]
fn exclusive_cli_preserves_existing_png_and_manifest() {
    let root = std::env::temp_dir().join(format!(
        "chipviz-capture-collisions-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    for existing in ["frame000000-PulseTunnel.png", "manifest.json"] {
        let output_dir = root.join(existing);
        std::fs::create_dir(&output_dir).unwrap();
        let collision = output_dir.join(existing);
        let original = b"existing output must remain intact";
        std::fs::write(&collision, original).unwrap();
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
            .args([
                "--profile",
                "pi5",
                "--scene",
                "2",
                "--input",
                "synth-mic",
                "--frames",
                "0",
                "--size",
                "160x90",
                "--exclusive-output",
                "--out",
            ])
            .arg(&output_dir)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{existing}: {stderr}");
        assert!(
            stderr.contains(&collision.display().to_string()),
            "{existing}: {stderr}"
        );
        assert_eq!(std::fs::read(&collision).unwrap(), original);
        if existing == "manifest.json" {
            // A PNG created before the manifest collision is allowed to remain.
            let png = std::fs::read(output_dir.join("frame000000-PulseTunnel.png")).unwrap();
            assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 2);
        } else {
            assert_eq!(std::fs::read_dir(&output_dir).unwrap().count(), 1);
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires a GPU adapter"]
fn exclusive_cli_matches_legacy_synthetic_capture() {
    let root = std::env::temp_dir().join(format!(
        "chipviz-capture-exclusive-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    for (directory, exclusive) in [("legacy", false), ("exclusive", true)] {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"));
        command.args([
            "--profile",
            "pi5",
            "--scene",
            "2",
            "--input",
            "synth-mic",
            "--frames",
            "0,60",
            "--size",
            "160x90",
            "--out",
        ]);
        command.arg(root.join(directory));
        if exclusive {
            command.arg("--exclusive-output");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for file in [
        "frame000000-PulseTunnel.png",
        "frame000060-PulseTunnel.png",
        "manifest.json",
    ] {
        assert_eq!(
            std::fs::read(root.join("legacy").join(file)).unwrap(),
            std::fs::read(root.join("exclusive").join(file)).unwrap(),
            "{file}"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires a GPU adapter"]
fn capture_api_and_cli_select_variants_without_changing_the_default() {
    let root = std::env::temp_dir().join(format!(
        "chipviz-capture-api-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let mut args = legacy_args(root.join("legacy"));
    capture::run(&args).unwrap();
    args.out = root.join("default");
    capture::run_with_options(&args, CaptureOptions::default()).unwrap();
    let legacy = std::fs::read(root.join("legacy/frame000120-PulseTunnel.png")).unwrap();
    let default = std::fs::read(root.join("default/frame000120-PulseTunnel.png")).unwrap();
    assert_eq!(legacy, default);
    assert_eq!(
        std::fs::read(root.join("legacy/manifest.json")).unwrap(),
        std::fs::read(root.join("default/manifest.json")).unwrap()
    );
    args.out = root.join("hallway");
    capture::run_with_options(
        &args,
        CaptureOptions {
            tunnel_variant: TunnelVariant::Hallway,
        },
    )
    .unwrap();
    let hallway = std::fs::read(root.join("hallway/frame000120-PulseTunnel-Hallway.png")).unwrap();
    assert_ne!(hallway, legacy);
    let manifest = std::fs::read_to_string(root.join("hallway/manifest.json")).unwrap();
    assert!(manifest.contains("\"scene_name\": \"PulseTunnel-Hallway\""));
    assert!(manifest.contains("\"camera\": \"screen_space\""));
    let cli = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
        .args([
            "--profile",
            "pi5",
            "--scene",
            "2",
            "--input",
            "synth-mic",
            "--frames",
            "120",
            "--size",
            "160x90",
            "--tunnel-variant",
            "abstract",
            "--out",
        ])
        .arg(root.join("cli"))
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    let abstract_space =
        std::fs::read(root.join("cli/frame000120-PulseTunnel-Abstract.png")).unwrap();
    assert_ne!(abstract_space, legacy);
    assert_ne!(abstract_space, hallway);
    let manifest = std::fs::read_to_string(root.join("cli/manifest.json")).unwrap();
    assert!(manifest.contains("\"scene_name\": \"PulseTunnel-Abstract\""));
    assert!(manifest.contains("\"camera\": \"screen_space\""));

    // Raw .cvz scene 6 and bare --scene 6 keep legacy SignalField dispatch.
    let mut scene_six_fixture =
        include_bytes!("../../../host/fixtures/musical/groove.cvz").to_vec();
    scene_six_fixture[20] = 6;
    let checksum = scene_six_fixture[..32]
        .iter()
        .fold(0, |checksum, byte| checksum ^ byte);
    scene_six_fixture[32] = checksum;
    let fixture_path = root.join("scene-six.cvz");
    std::fs::write(&fixture_path, scene_six_fixture).unwrap();
    let legacy_cli = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
        .args(["--profile", "pi5", "--input"])
        .arg(&fixture_path)
        .args(["--frames", "0", "--size", "160x90", "--out"])
        .arg(root.join("legacy-cli-scene-six"))
        .output()
        .unwrap();
    assert!(
        legacy_cli.status.success(),
        "{}",
        String::from_utf8_lossy(&legacy_cli.stderr)
    );
    assert!(
        root.join("legacy-cli-scene-six/frame000000-SignalField.png")
            .exists()
    );
    let manifest =
        std::fs::read_to_string(root.join("legacy-cli-scene-six/manifest.json")).unwrap();
    assert!(manifest.contains("\"scene\": 6, \"displayed_scene\": 0"));

    let bare_scene_six = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
        .args([
            "--profile",
            "pi5",
            "--scene",
            "6",
            "--input",
            "procedural",
            "--frames",
            "0",
            "--size",
            "160x90",
            "--out",
        ])
        .arg(root.join("bare-scene-six"))
        .output()
        .unwrap();
    assert!(
        bare_scene_six.status.success(),
        "{}",
        String::from_utf8_lossy(&bare_scene_six.stderr)
    );
    assert!(
        root.join("bare-scene-six/frame000000-SignalField.png")
            .exists()
    );
    let manifest = std::fs::read_to_string(root.join("bare-scene-six/manifest.json")).unwrap();
    assert!(manifest.contains("\"scene\": 6, \"displayed_scene\": 0"));

    let mut wrapped_fixture = std::fs::read(&fixture_path).unwrap();
    wrapped_fixture[20] = 13;
    let checksum = wrapped_fixture[..32]
        .iter()
        .fold(0, |checksum, byte| checksum ^ byte);
    wrapped_fixture[32] = checksum;
    let wrapped_fixture_path = root.join("scene-thirteen.cvz");
    std::fs::write(&wrapped_fixture_path, wrapped_fixture).unwrap();
    let unrelated_scene = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
        .args(["--profile", "pi5", "--input"])
        .arg(&wrapped_fixture_path)
        .args(["--frames", "0", "--size", "160x90", "--out"])
        .arg(root.join("legacy-scene-thirteen"))
        .output()
        .unwrap();
    assert!(
        unrelated_scene.status.success(),
        "{}",
        String::from_utf8_lossy(&unrelated_scene.stderr)
    );
    assert!(
        root.join("legacy-scene-thirteen/frame000000-SpectrumVolume.png")
            .exists()
    );
    let manifest =
        std::fs::read_to_string(root.join("legacy-scene-thirteen/manifest.json")).unwrap();
    assert!(manifest.contains("\"scene\": 13, \"displayed_scene\": 1"));

    let synthetic_catalog = root.join("synthetic-stars.csv");
    std::fs::write(&synthetic_catalog, "ra,dec,mag\n0,90,1\n").unwrap();
    let starfield_cli = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
        .args([
            "--profile",
            "pi5",
            "--scene",
            "6",
            "--input",
            "procedural",
            "--frames",
            "0",
            "--size",
            "160x90",
            "--star-catalog",
        ])
        .arg(&synthetic_catalog)
        .args(["--location", "90,0", "--time", "946728000", "--out"])
        .arg(root.join("opt-in-starfield"))
        .output()
        .unwrap();
    assert!(
        starfield_cli.status.success(),
        "{}",
        String::from_utf8_lossy(&starfield_cli.stderr)
    );
    assert!(
        root.join("opt-in-starfield/frame000000-Starfield.png")
            .exists()
    );
    let manifest = std::fs::read_to_string(root.join("opt-in-starfield/manifest.json")).unwrap();
    assert!(manifest.contains("\"scene\": 6, \"displayed_scene\": 6"));

    // --starfield opts into recorded scene-6 frames without forcing every
    // frame to 6; the raw fixture byte selects Starfield only in this mode.
    let recorded_cli = std::process::Command::new(env!("CARGO_BIN_EXE_chipviz-capture"))
        .args(["--profile", "pi5", "--input"])
        .arg(&fixture_path)
        .args([
            "--frames",
            "0",
            "--size",
            "160x90",
            "--starfield",
            "--star-catalog",
        ])
        .arg(&synthetic_catalog)
        .args(["--location", "90,0", "--time", "946728000", "--out"])
        .arg(root.join("recorded-starfield"))
        .output()
        .unwrap();
    assert!(
        recorded_cli.status.success(),
        "{}",
        String::from_utf8_lossy(&recorded_cli.stderr)
    );
    assert!(
        root.join("recorded-starfield/frame000000-Starfield.png")
            .exists()
    );
    let manifest = std::fs::read_to_string(root.join("recorded-starfield/manifest.json")).unwrap();
    assert!(manifest.contains("\"scene\": 6, \"displayed_scene\": 6"));

    // The pre-Starfield API must retain scene-byte-6's legacy SignalField
    // mapping; only the separate Starfield API opts in to Starfield.
    args.scene = Some(6);
    args.input = Input::Procedural;
    args.frames = vec![0];
    args.out = root.join("legacy-scene-six");
    capture::run(&args).unwrap();
    assert!(
        root.join("legacy-scene-six/frame000000-SignalField.png")
            .exists()
    );
    let manifest = std::fs::read_to_string(root.join("legacy-scene-six/manifest.json")).unwrap();
    assert!(manifest.contains("\"scene\": 6, \"displayed_scene\": 0"));
    std::fs::remove_dir_all(root).unwrap();
}
