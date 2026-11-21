#![cfg(not(feature = "dmg-preview"))]

#[test]
fn release_feature_sets_reject_preview_before_input_or_window_setup() {
    for binary in [
        env!("CARGO_BIN_EXE_chipviz-m1"),
        env!("CARGO_BIN_EXE_chipviz-pi5"),
    ] {
        for args in [
            &["--dmg-preview", "--help"][..],
            &["--dmg-preview", "--list-monitors"][..],
            &["--mic", "--dmg-preview"][..],
        ] {
            let output = std::process::Command::new(binary)
                .args(args)
                .output()
                .expect("run renderer");
            assert!(!output.status.success(), "{binary} {args:?}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("--dmg-preview requires the dmg-preview Cargo feature"),
                "{binary} {args:?}: {stderr}"
            );
        }
    }
}
