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
                "{option} requires a value; --exclusive-output is a flag"
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
