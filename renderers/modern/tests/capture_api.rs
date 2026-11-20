use std::path::PathBuf;

use chipviz_modern::{
    RenderProfile,
    capture::{
        self, CaptureArgs, CaptureOptions, Command, ConfiguredCommand, Input, TunnelVariant,
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
    std::fs::remove_dir_all(root).unwrap();
}
