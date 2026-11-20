use super::{Snapshot, ValidationError};
use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn python_command(value: Option<OsString>) -> Command {
    let value = value
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "python3".into());
    // An existing literal path takes precedence, including paths with spaces.
    // Otherwise split UTF-8 values into whitespace-separated arguments; this
    // deliberately does not implement shell quoting, expansion, or globbing.
    // A path containing spaces cannot be combined with arguments this way.
    // Non-UTF-8 values remain a single executable path, without arguments.
    if !Path::new(&value).is_file()
        && let Some(text) = value.to_str()
    {
        let mut words = text.split_whitespace();
        if let Some(program) = words.next() {
            let mut command = Command::new(program);
            command.args(words);
            return command;
        }
    }
    Command::new(value)
}

#[test]
fn python_command_selects_interpreter_without_shell_evaluation() {
    use std::ffi::OsStr;

    for value in [None, Some(OsString::new())] {
        let command = python_command(value);
        assert_eq!(command.get_program(), OsStr::new("python3"));
        assert_eq!(command.get_args().count(), 0);
    }
    let command = python_command(Some("/absolute/python -B -I".into()));
    assert_eq!(command.get_program(), OsStr::new("/absolute/python"));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec![OsStr::new("-B"), OsStr::new("-I")]
    );
    let command = python_command(Some("python3 $HOME/*.py".into()));
    assert_eq!(command.get_program(), OsStr::new("python3"));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec![OsStr::new("$HOME/*.py")]
    );

    let path = std::env::temp_dir().join(format!(
        "chipviz python {} {}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    let command = python_command(Some(path.as_os_str().to_owned()));
    assert_eq!(command.get_program(), path.as_os_str());
    assert_eq!(command.get_args().count(), 0);
    std::fs::remove_file(path).unwrap();
}

#[cfg(unix)]
#[test]
fn python_command_preserves_non_utf8_executable() {
    use std::os::unix::ffi::OsStringExt;

    let path = OsString::from_vec(b"/python-\xff".to_vec());
    let command = python_command(Some(path.clone()));
    assert_eq!(command.get_program(), path);
    assert_eq!(command.get_args().count(), 0);
}

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

fn header(count: u64) -> Vec<u8> {
    let mut bytes = b"CVXE\x01\x01\0\0".to_vec();
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&crc32(&bytes).to_le_bytes());
    bytes
}

fn fix_record(bytes: &mut [u8]) {
    bytes[56] = bytes[24..56].iter().fold(0, |xor, byte| xor ^ byte);
    let crc = crc32(&bytes[..141]);
    bytes[141..145].copy_from_slice(&crc.to_le_bytes());
}

fn record(sequence: u64, family: u8, variant: u8) -> Vec<u8> {
    let mut bytes = vec![0; 145];
    bytes[..6].copy_from_slice(b"CVX1\x01\x01");
    bytes[6] = variant;
    bytes[8..16].copy_from_slice(&sequence.to_le_bytes());
    bytes[16..24].copy_from_slice(&sequence.to_le_bytes());
    bytes[24] = 0xc7;
    bytes[26..28].copy_from_slice(&(sequence as u16).to_le_bytes());
    bytes[44] = family;
    fix_record(&mut bytes);
    bytes
}

fn file(records: &[Vec<u8>]) -> Vec<u8> {
    let mut result = header(records.len() as u64);
    for record in records {
        result.extend_from_slice(record);
    }
    result
}

fn star(sequence: u64, epoch: i64) -> Vec<u8> {
    let mut bytes = record(sequence, 6, 0);
    bytes[57..89].fill(1);
    bytes[89..97].copy_from_slice(&epoch.to_le_bytes());
    bytes[109..141].fill(2);
    fix_record(&mut bytes);
    bytes
}

fn result(bytes: &[u8]) -> &'static str {
    match Snapshot::validate(bytes) {
        Ok(_) => "valid",
        Err(ValidationError::Invalid) => "invalid",
        Err(ValidationError::StarfieldUnavailable) => "unavailable",
        Err(ValidationError::AllocationFailed) => panic!("test allocation failed"),
    }
}

#[test]
fn snapshot_preserves_identity_order_and_owned_bytes() {
    let records = vec![record(0, 2, 2), record(1, 11, 3), record(2, 7, 0)];
    let mut input = file(&records);
    let original_header = input[..20].to_vec();
    let snapshot = Snapshot::validate(&input).unwrap();
    input.fill(0);
    input = file(&[record(0, 0, 0)]);
    assert_eq!(snapshot.header(), original_header);
    assert_eq!(snapshot.count(), 3);
    for (index, expected) in records.iter().enumerate() {
        let item = snapshot.record(index).unwrap();
        assert_eq!(item.envelope(), expected);
        assert_eq!(item.sequence(), index as u64);
        assert_eq!(item.tick(), index as u64);
        assert_eq!(item.family(), expected[44]);
        assert_eq!(item.variant(), expected[6]);
        assert_eq!(item.metadata(), &expected[57..141]);
    }
    assert!(snapshot.record(3).is_none());
    assert_eq!(
        Snapshot::validate(&input)
            .unwrap()
            .record(0)
            .unwrap()
            .family(),
        0
    );
}

#[test]
fn snapshot_survives_source_file_replacement() {
    let path = std::env::temp_dir().join(format!(
        "chipviz-catalog-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&path).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let _cleanup = Cleanup(path.clone());
    let source = path.join("source");
    let replacement = path.join("replacement");
    let original = file(&[record(0, 2, 2)]);
    std::fs::write(&source, &original).unwrap();
    let snapshot = Snapshot::validate(&std::fs::read(&source).unwrap()).unwrap();
    std::fs::write(&replacement, file(&[record(0, 11, 3)])).unwrap();
    std::fs::rename(&replacement, &source).unwrap();
    assert_eq!(snapshot.record(0).unwrap().envelope(), &original[20..]);
    assert_eq!(snapshot.record(0).unwrap().variant(), 2);
    assert_eq!(
        Snapshot::validate(&std::fs::read(&source).unwrap())
            .unwrap()
            .record(0)
            .unwrap()
            .family(),
        11
    );
}

#[test]
fn maximum_record_count_produces_complete_owned_snapshot() {
    let count = 1_000_000;
    let mut input = Vec::with_capacity(20 + 145 * count);
    input.extend_from_slice(&header(count as u64));
    for sequence in 0..count {
        input.extend_from_slice(&record(sequence as u64, 11, 3));
    }
    let snapshot = Snapshot::validate(&input).unwrap();
    assert_eq!(snapshot.count(), count);
    for index in [0, 65_536, count - 1] {
        let item = snapshot.record(index).unwrap();
        assert_eq!(item.envelope(), record(index as u64, 11, 3));
        assert_eq!(item.tick(), index as u64);
    }
    assert!(snapshot.record(count).is_none());
    input.fill(0);
    assert_eq!(snapshot.record(count - 1).unwrap().sequence(), 999_999);
}

#[test]
fn differential_recordings_against_python_host_validator() {
    let mut vectors: Vec<(&str, Vec<u8>, &str)> = Vec::new();
    vectors.push((
        "mixed identities",
        file(&[record(0, 2, 2), record(1, 11, 3)]),
        "valid",
    ));
    for family in 0..14 {
        if family == 6 {
            continue;
        }
        let max = if family == 2 {
            2
        } else if family == 11 {
            3
        } else {
            0
        };
        for variant in 0..=max {
            vectors.push(("valid pair", file(&[record(0, family, variant)]), "valid"));
        }
        vectors.push((
            "invalid variant",
            file(&[record(0, family, max + 1)]),
            "invalid",
        ));
    }
    vectors.push(("unknown family", file(&[record(0, 14, 0)]), "invalid"));
    let mut active = record(0, 7, 0);
    active[47] = 4;
    for index in 0..4 {
        active[48 + index * 2] = 127;
    }
    fix_record(&mut active);
    vectors.push(("active notes", file(&[active.clone()]), "valid"));
    active[54] = 128;
    fix_record(&mut active);
    vectors.push(("invalid active note", file(&[active]), "invalid"));
    let mut inactive = record(0, 7, 0);
    inactive[54] = 255;
    fix_record(&mut inactive);
    vectors.push(("inactive note", file(&[inactive]), "valid"));
    for (offset, value) in [(47, 5), (46, 0x20), (24, 0), (25, 1), (26, 1)] {
        let mut bad = record(0, 7, 0);
        bad[offset] = value;
        fix_record(&mut bad);
        vectors.push(("bad frame", file(&[bad]), "invalid"));
    }
    for (offset, value) in [(0, 0), (4, 2), (5, 2), (7, 1), (57, 1)] {
        let mut bad = record(0, 7, 0);
        bad[offset] = value;
        fix_record(&mut bad);
        vectors.push(("bad envelope", file(&[bad]), "invalid"));
    }
    let mut bad_xor = record(0, 7, 0);
    bad_xor[56] ^= 1;
    let crc = crc32(&bad_xor[..141]);
    bad_xor[141..].copy_from_slice(&crc.to_le_bytes());
    vectors.push(("bad xor", file(&[bad_xor]), "invalid"));
    let mut corrupt = record(1, 7, 0);
    corrupt[144] ^= 1;
    vectors.push(("late crc", file(&[record(0, 7, 0), corrupt]), "invalid"));
    vectors.push((
        "late unsupported pair",
        file(&[record(0, 7, 0), record(1, 11, 4)]),
        "invalid",
    ));
    vectors.push((
        "sequence gap",
        file(&[record(0, 7, 0), record(2, 7, 0)]),
        "invalid",
    ));
    vectors.push((
        "overflowed sequence",
        file(&[record(0, 7, 0), record(u64::MAX, 7, 0)]),
        "invalid",
    ));
    let mut tick = record(1, 7, 0);
    tick[16..24].copy_from_slice(&0u64.to_le_bytes());
    fix_record(&mut tick);
    vectors.push(("tick mismatch", file(&[record(0, 7, 0), tick]), "invalid"));
    let good = file(&[record(0, 7, 0)]);
    vectors.push(("truncated", good[..good.len() - 1].to_vec(), "invalid"));
    let mut trailing = good.clone();
    trailing.push(0);
    vectors.push(("trailing", trailing, "invalid"));
    vectors.push(("legacy mode", vec![0xc7; 33], "invalid"));
    let mut bad_header = good.clone();
    bad_header[16] ^= 1;
    vectors.push(("header crc", bad_header, "invalid"));
    let mut wrong_mode = good.clone();
    wrong_mode[5] = 2;
    let crc = crc32(&wrong_mode[..16]);
    wrong_mode[16..20].copy_from_slice(&crc.to_le_bytes());
    vectors.push(("wrong catalog", wrong_mode, "invalid"));
    for offset in [0, 4, 6, 7] {
        let mut bad = good.clone();
        bad[offset] ^= 1;
        let crc = crc32(&bad[..16]);
        bad[16..20].copy_from_slice(&crc.to_le_bytes());
        vectors.push(("bad header", bad, "invalid"));
    }
    for count in [0, 2, 1_000_000, 1_000_001, u64::MAX] {
        vectors.push(("count bounds or length", header(count), "invalid"));
    }
    let valid_star = star(0, 946_684_800_000);
    vectors.push((
        "star unavailable",
        file(std::slice::from_ref(&valid_star)),
        "unavailable",
    ));
    vectors.push(("star missing digest", file(&[record(0, 6, 0)]), "invalid"));
    for (offset, min, max) in [
        (97, -900_000_000i32, 900_000_000i32),
        (101, -1_800_000_000, 1_800_000_000),
        (105, -12_000_000, 100_000_000),
    ] {
        for (value, expected) in [
            (min, "unavailable"),
            (max, "unavailable"),
            (min - 1, "invalid"),
            (max + 1, "invalid"),
            (i32::MIN, "invalid"),
            (i32::MAX, "invalid"),
        ] {
            let mut wire = star(0, 946_684_800_000);
            wire[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            fix_record(&mut wire);
            vectors.push(("star signed location boundary", file(&[wire]), expected));
        }
    }
    for (epoch, expected) in [
        (946_684_800_000, "unavailable"),
        (4_102_444_799_999, "unavailable"),
        (946_684_799_999, "invalid"),
        (4_102_444_800_000, "invalid"),
        (i64::MIN, "invalid"),
        (i64::MAX, "invalid"),
    ] {
        vectors.push((
            "star signed epoch boundary",
            file(&[star(0, epoch)]),
            expected,
        ));
    }
    for (tick, expected) in [(3, "unavailable"), (4, "invalid")] {
        let mut records: Vec<_> = (0..tick).map(|i| record(i, 7, 0)).collect();
        records.push(star(tick, 4_102_444_799_949));
        vectors.push(("star rational sky time boundary", file(&records), expected));
    }
    let mut changed_star = star(2, 946_684_800_000);
    changed_star[97] = 1;
    fix_record(&mut changed_star);
    vectors.push((
        "star metadata changed",
        file(&[valid_star.clone(), record(1, 7, 0), changed_star]),
        "invalid",
    ));
    vectors.push((
        "star sky time",
        file(&[record(0, 7, 0), star(1, 4_102_444_799_999)]),
        "invalid",
    ));
    let mut bad_after_star = record(1, 7, 0);
    bad_after_star[144] ^= 1;
    vectors.push((
        "late error after star",
        file(&[valid_star, bad_after_star]),
        "invalid",
    ));

    let mut wrap = header(65_537);
    for sequence in 0..=65_536 {
        wrap.extend_from_slice(&record(sequence, 2, 1));
    }
    let snapshot = Snapshot::validate(&wrap).unwrap();
    assert_eq!(snapshot.record(65_536).unwrap().tick(), 65_536);
    assert_eq!(
        &snapshot.record(65_536).unwrap().envelope()[26..28],
        &[0, 0]
    );
    vectors.push(("frame wrap", wrap, "valid"));

    let script = format!(
        "{}/tests/catalog_v1_reference.py",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut child = python_command(std::env::var_os("PYTHON"))
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("development differential test requires a Python interpreter");
    {
        let mut stdin = child.stdin.take().unwrap();
        for (_, bytes, _) in &vectors {
            stdin
                .write_all(&(bytes.len() as u64).to_le_bytes())
                .unwrap();
            stdin.write_all(bytes).unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "Python oracle failed: {:?}",
        output.status
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let oracle: Vec<_> = stdout.lines().collect();
    assert_eq!(oracle.len(), vectors.len());
    for ((name, bytes, expected), python) in vectors.iter().zip(oracle) {
        assert_eq!(result(bytes), *expected, "Rust: {name}");
        assert_eq!(python, *expected, "Python: {name}");
    }
}
