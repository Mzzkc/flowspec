use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn public_stdin_limit_is_inclusive_and_refuses_without_output() {
    let limit = 128 * 1024 * 1024;
    for (size, expected) in [
        (2, 0),
        (64 * 1024 * 1024 + 39, 0),
        (limit, 0),
        (limit + 1, 1),
    ] {
        let mut file = tempfile::tempfile().unwrap();
        let mut input = vec![b' '; size];
        input[..2].copy_from_slice(b"{}");
        file.write_all(&input).unwrap();
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(0)).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_flowspec"))
            .args(["expand-report", "--quiet"])
            .stdin(Stdio::from(file))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(expected), "input bytes {size}");
        if expected == 0 {
            assert_eq!(output.stdout, b"{}");
        } else {
            assert!(output.stdout.is_empty());
            assert!(String::from_utf8_lossy(&output.stderr).contains("128 MiB"));
        }
    }
}
