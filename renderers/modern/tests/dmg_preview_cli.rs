#[cfg(not(feature = "dmg-preview"))]
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

#[test]
fn conflicting_preview_and_house_exit_before_display_or_capture_setup() {
    for binary in [
        env!("CARGO_BIN_EXE_chipviz-m1"),
        env!("CARGO_BIN_EXE_chipviz-pi5"),
        env!("CARGO_BIN_EXE_chipviz-capture"),
    ] {
        for args in [
            &["--house", "--dmg-preview"][..],
            &["--dmg-preview", "--house"][..],
        ] {
            let output = std::process::Command::new(binary)
                .args(args)
                .output()
                .expect("run selector");
            assert!(!output.status.success(), "{binary} {args:?}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("incompatible")
                    || stderr.contains("cannot be combined")
                    || stderr.contains("requires the dmg-preview Cargo feature"),
                "{binary} {args:?}: {stderr}"
            );
        }
    }
}

#[test]
fn live_literal_house_catalog_and_selector_allow_help() {
    for binary in [
        env!("CARGO_BIN_EXE_chipviz-m1"),
        env!("CARGO_BIN_EXE_chipviz-pi5"),
    ] {
        for args in [
            &[
                "--star-catalog",
                "--house",
                "--location",
                "0,0",
                "--scene",
                "6",
                "--help",
            ][..],
            &[
                "--catalog",
                "--house",
                "--location",
                "0,0",
                "--scene",
                "6",
                "--help",
            ][..],
            &["--monitor", "--house", "--help"][..],
        ] {
            let output = std::process::Command::new(binary)
                .args(args)
                .output()
                .expect("run renderer help");
            assert!(output.status.success(), "{binary} {args:?}: {output:?}");
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("usage:"),
                "{binary} {args:?}: {output:?}"
            );
        }
    }
}

#[cfg(all(feature = "dmg-preview", feature = "unmeasured-flash"))]
#[test]
fn combined_features_allow_preview_help() {
    for binary in [
        env!("CARGO_BIN_EXE_chipviz-m1"),
        env!("CARGO_BIN_EXE_chipviz-pi5"),
    ] {
        let output = std::process::Command::new(binary)
            .args(["--dmg-preview", "--help"])
            .output()
            .expect("run preview help");
        assert!(output.status.success(), "{binary}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("usage:"),
            "{binary}: {output:?}"
        );
    }
}
