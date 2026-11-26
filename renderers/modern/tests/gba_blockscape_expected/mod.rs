use crate::gba_blockscape::{DemoState, Frame, PIXELS};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct NativeOracle {
    directory: PathBuf,
    executable: PathBuf,
}

impl NativeOracle {
    pub fn compile() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("chipviz-gba-oracle-{}-{stamp}", std::process::id()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&directory)
            .expect("create private oracle directory");
        let oracle = Self {
            executable: directory.join("native-oracle"),
            directory,
        };
        let output = Command::new("cc")
            .arg("-std=c11")
            .arg("-O0")
            .arg(root.join("tests/gba_blockscape_expected/native_oracle.c"))
            .arg(root.join("../../cores/gba/homebrew/source/colour_field.c"))
            .arg(root.join("../../cores/gba/homebrew/source/native_prefetch.c"))
            .arg("-o")
            .arg(&oracle.executable)
            .output()
            .expect("host C compiler is required for native-source oracle");
        assert!(
            output.status.success(),
            "oracle compilation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        oracle
    }

    pub fn compose(&self, frame: u16, state: DemoState, keys: u16) -> Frame {
        let mut child = Command::new(&self.executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("run native-source oracle");
        let mut stdin = child.stdin.take().unwrap();
        write!(
            stdin,
            "{} {} {} {} {} {} {}",
            frame,
            state.scene,
            state.palette,
            state.energy,
            u8::from(state.beat),
            state.note,
            keys
        )
        .unwrap();
        for bin in state.spectrum {
            write!(stdin, " {bin}").unwrap();
        }
        drop(stdin);
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "native-source oracle failed");
        assert_eq!(output.stdout.len(), PIXELS + 512);
        let mut palette = [0; 256];
        for (colour, bytes) in palette
            .iter_mut()
            .zip(output.stdout[PIXELS..].chunks_exact(2))
        {
            *colour = u16::from_le_bytes([bytes[0], bytes[1]]);
        }
        Frame {
            indices: output.stdout[..PIXELS].to_vec(),
            palette,
        }
    }
}

impl Drop for NativeOracle {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.executable);
        let _ = std::fs::remove_dir(&self.directory);
    }
}
