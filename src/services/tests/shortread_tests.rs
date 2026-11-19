//! Short flatpak reads and executor seams.

use super::*;

// Short Flatpak reads and streaming mutations share the framework process
// transport, but reads retain only the route's declared response budget.
static SHORT_READ_FIXTURE_SEQUENCE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

fn short_read_fixture(mode: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt as _;
    use std::sync::atomic::Ordering;

    let sequence = SHORT_READ_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "store-flatpak-read-{mode}-{}-{sequence}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create short-read fixture directory");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/flatpak-operation-fixture");
    let program = directory.join("flatpak-fixture");
    std::fs::copy(source, &program).expect("copy short-read fixture");
    let mut permissions = std::fs::metadata(&program)
        .expect("fixture metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&program, permissions).expect("make short-read fixture executable");
    std::fs::write(directory.join("mode"), mode).expect("write fixture mode");
    (directory, program)
}

fn short_read_limits(stdout: usize, stderr: usize, deadline: Duration) -> ShortReadLimits {
    ShortReadLimits {
        stdout,
        stderr,
        deadline,
    }
}

fn short_read_pid_identity(pid: u32) -> Option<(i32, u64)> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, fields) = raw.rsplit_once(") ")?;
    let fields = fields.split_ascii_whitespace().collect::<Vec<_>>();
    Some((fields.get(2)?.parse().ok()?, fields.get(19)?.parse().ok()?))
}

// Poll a PID marker until the fixture has both created the file AND
// written a parseable non-zero PID. The fixture opens the file with
// O_CREAT|O_TRUNC before writing, so a naive `exists()` check races the
// write and can observe an empty file; retrying keeps flaky spawns off
// the flatpak-cancellation gates.
fn wait_for_short_read_pid(path: &std::path::Path, ceiling: Duration) -> u32 {
    let deadline = Instant::now() + ceiling;
    loop {
        if path.exists()
            && let Ok(raw) = std::fs::read_to_string(path)
            && let Ok(pid) = raw.trim().parse::<u32>()
            && pid > 0
        {
            return pid;
        }
        if Instant::now() >= deadline {
            panic!(
                "fixture did not publish a valid PID at {} within ceiling",
                path.display()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_short_read_file(path: &std::path::Path, ceiling: Duration) {
    let _ = wait_for_short_read_pid(path, ceiling);
}

fn read_fixture_pid(path: &std::path::Path) -> u32 {
    wait_for_short_read_pid(path, Duration::from_secs(2))
}

fn wait_for_short_read_identity_gone(pid: u32, identity: (i32, u64), ceiling: Duration) {
    let deadline = Instant::now() + ceiling;
    while short_read_pid_identity(pid) == Some(identity) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_ne!(
        short_read_pid_identity(pid),
        Some(identity),
        "owned process {pid} survived teardown ceiling"
    );
}

#[test]
fn short_read_drains_both_pipes_concurrently_with_bounded_retention() {
    let (directory, program) = short_read_fixture("capture");
    let limit = 128 * 1024;
    std::fs::write(directory.join("stdout-bytes"), limit.to_string()).unwrap();
    std::fs::write(directory.join("stderr-bytes"), limit.to_string()).unwrap();
    let started = Instant::now();
    let output = run_bounded_flatpak(
        &program,
        &["info".into()],
        short_read_limits(limit, limit, Duration::from_secs(2)),
        &OperationCancellation::default(),
    )
    .expect("exact-limit dual flood must complete");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(output.stdout.len(), limit);
    assert_eq!(output.stderr.len(), limit);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn short_read_deadline_kills_term_ignoring_group_and_reaps_leader() {
    let (directory, program) = short_read_fixture("hold");
    let worker = std::thread::spawn(move || {
        run_bounded_flatpak(
            &program,
            &["info".into()],
            short_read_limits(1024 * 1024, 1024 * 1024, Duration::from_millis(250)),
            &OperationCancellation::default(),
        )
    });
    wait_for_short_read_file(&directory.join("leader"), Duration::from_secs(1));
    wait_for_short_read_file(&directory.join("descendant"), Duration::from_secs(1));
    let leader = read_fixture_pid(&directory.join("leader"));
    let descendant = read_fixture_pid(&directory.join("descendant"));
    let leader_identity = short_read_pid_identity(leader).expect("leader starts alive");
    let descendant_identity = short_read_pid_identity(descendant).expect("descendant starts alive");
    let started = Instant::now();
    let error = worker
        .join()
        .expect("deadline worker joins")
        .expect_err("deadline fails");
    assert!(error.contains("deadline"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(2));
    wait_for_short_read_identity_gone(leader, leader_identity, Duration::from_secs(2));
    wait_for_short_read_identity_gone(descendant, descendant_identity, Duration::from_secs(2));
    std::fs::remove_dir_all(directory).unwrap();
}
