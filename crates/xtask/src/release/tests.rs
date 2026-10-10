use super::*;
use anyhow::bail;
use std::{cell::RefCell, process::Command};

fn arguments(command: &Command) -> Vec<String> {
    std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|value| value.to_string_lossy().into_owned())
        .collect()
}

#[derive(Default)]
struct GitRunner {
    commands: RefCell<Vec<Vec<String>>>,
    fail_dispatch: bool,
}
impl Runner for GitRunner {
    fn run(&self, command: &mut Command) -> Result<()> {
        let args = arguments(command);
        self.commands.borrow_mut().push(args.clone());
        if args.first().is_some_and(|program| program == "gh") {
            ensure!(
                !self.fail_dispatch || !args.iter().any(|arg| arg == "workflow"),
                "fixture dispatch failure"
            );
            return Ok(());
        }
        System.run(command)
    }
    fn output(&self, command: &mut Command) -> Result<String> {
        System.output(command)
    }
    fn logged(&self, _: &mut Command, _: &Path) -> Result<()> {
        bail!("Unexpected external validation")
    }
}

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    remote: PathBuf,
    options: Options,
    runner: GitRunner,
}
impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("repo [with spaces]");
        fs::create_dir_all(root.join("crates/sample/src")).unwrap();
        git(&root, &["init", "--quiet", "--initial-branch=master"]).unwrap();
        for (key, value) in [
            ("user.name", "Fixture"),
            ("user.email", "fixture@localhost"),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", "/dev/null"),
        ] {
            git(&root, &["config", key, value]).unwrap();
        }
        fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n[workspace.package]\nversion = \"0.1.0\" # retain comment\n").unwrap();
        fs::write(
            root.join("crates/sample/Cargo.toml"),
            "[package]\nname = \"sample\"\nversion.workspace = true\n",
        )
        .unwrap();
        fs::write(root.join("crates/sample/src/lib.rs"), "").unwrap();
        fs::write(root.join("Cargo.lock"), "# retain lock comment\nversion = 4\n[[package]]\nname = \"sample\"\nversion = \"0.1.0\"\n").unwrap();
        fs::write(root.join("tracked"), "before\n").unwrap();
        fs::write(root.join(".gitignore"), ".artifacts/\nignored\n").unwrap();
        git(&root, &["add", "--all"]).unwrap();
        git(&root, &["commit", "--quiet", "-m", "Initial"]).unwrap();
        let remote = temporary.path().join("remote.git");
        git(
            &root,
            &["init", "--quiet", "--bare", remote.to_str().unwrap()],
        )
        .unwrap();
        git(
            &root,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        )
        .unwrap();
        git(&root, &["push", "--quiet", "origin", "master"]).unwrap();
        Self {
            _temporary: temporary,
            root,
            remote,
            options: Options {
                check_only: true,
                skip_linux: true,
                skip_windows: true,
                ..Options::default()
            },
            runner: GitRunner::default(),
        }
    }
    fn state(&self) -> (String, String, Vec<u8>, Vec<u8>) {
        (
            git(&self.root, &["rev-parse", "HEAD"]).unwrap(),
            git(&self.root, &["write-tree"]).unwrap(),
            fs::read(self.root.join("Cargo.toml")).unwrap(),
            fs::read(self.root.join("Cargo.lock")).unwrap(),
        )
    }
    fn execute(&self, validate: impl FnOnce(&Path, &Path) -> Result<()>) -> Result<()> {
        execute(&self.root, &self.options, &self.runner, validate)
    }
    fn remote_head(&self) -> String {
        git(&self.remote, &["rev-parse", "refs/heads/master"]).unwrap()
    }
    #[cfg(unix)]
    fn hook(&self, path: &Path, source: &str) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, source).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn check_only_preserves_staged_unstaged_deleted_and_untracked_work() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("tracked"), "staged\n").unwrap();
    git(&fixture.root, &["add", "tracked"]).unwrap();
    fs::write(fixture.root.join("tracked"), "unstaged\n").unwrap();
    fs::write(fixture.root.join("new"), "new\n").unwrap();
    fs::write(fixture.root.join("ignored"), "private\n").unwrap();
    fs::remove_file(fixture.root.join("crates/sample/src/lib.rs")).unwrap();
    let before = fixture.state();
    fixture
        .execute(|source, _| {
            assert!(
                fs::read_to_string(source.join("Cargo.toml"))?
                    .contains("version = \"0.2.0\" # retain comment")
            );
            // `git checkout-index` converts LF to CRLF on Windows
            // (core.autocrlf), so compare line-ending-insensitively.
            assert_eq!(
                fs::read_to_string(source.join("tracked"))?.replace("\r\n", "\n"),
                "unstaged\n"
            );
            assert!(source.join("new").exists());
            assert!(!source.join("ignored").exists());
            assert!(!source.join("crates/sample/src/lib.rs").exists());
            assert_eq!(fixture.state(), before);
            Ok(())
        })
        .unwrap();
    assert_eq!(fixture.state(), before);
    assert!(fixture.runner.commands.borrow().is_empty());
}

#[test]
fn failed_validation_preserves_version_index_and_commit() {
    let fixture = Fixture::new();
    let before = fixture.state();
    let error = fixture
        .execute(|_, _| bail!("fixture failure"))
        .unwrap_err();
    assert!(error.to_string().contains("fixture failure"));
    assert_eq!(fixture.state(), before);
}

#[test]
fn concurrent_source_edit_stops_release_and_preserves_edit() {
    let fixture = Fixture::new();
    let before = fixture.state();
    let error = fixture
        .execute(|_, _| {
            fs::write(fixture.root.join("tracked"), "concurrent\n")?;
            Ok(())
        })
        .unwrap_err();
    assert!(error.to_string().contains("Source changed"));
    assert_eq!(fixture.state(), before);
    assert_eq!(
        fs::read_to_string(fixture.root.join("tracked")).unwrap(),
        "concurrent\n"
    );
}

#[test]
fn changed_snapshot_stops_release() {
    let fixture = Fixture::new();
    let before = fixture.state();
    let error = fixture
        .execute(|source, _| {
            fs::write(source.join("tracked"), "changed\n")?;
            Ok(())
        })
        .unwrap_err();
    assert!(error.to_string().contains("Checks changed source"));
    assert_eq!(fixture.state(), before);
}

#[test]
fn successful_release_pushes_tested_tree_and_dispatches_tag() {
    let mut fixture = Fixture::new();
    fixture.options.check_only = false;
    fixture.execute(|_, _| Ok(())).unwrap();
    let revision = git(&fixture.root, &["rev-parse", "HEAD"]).unwrap();
    assert_eq!(fixture.remote_head(), revision);
    assert_eq!(
        git(&fixture.remote, &["rev-parse", "refs/tags/v0.2.0"]).unwrap(),
        revision
    );
    assert_eq!(
        fixture.runner.commands.borrow().last().unwrap(),
        &["gh", "workflow", "run", "release.yaml", "--ref", "v0.2.0"]
    );
}

#[test]
fn wrong_branch_cannot_publish() {
    let mut fixture = Fixture::new();
    fixture.options.check_only = false;
    git(&fixture.root, &["checkout", "--quiet", "-b", "feature"]).unwrap();
    let before = fixture.state();
    let error = fixture.execute(|_, _| Ok(())).unwrap_err();
    assert!(error.to_string().contains("must run from master"));
    assert_eq!(fixture.state(), before);
}

#[test]
fn existing_tag_stops_before_version_change() {
    let mut fixture = Fixture::new();
    fixture.options.check_only = false;
    git(&fixture.root, &["tag", "v0.2.0"]).unwrap();
    let before = fixture.state();
    assert!(
        fixture
            .execute(|_, _| Ok(()))
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );
    assert_eq!(fixture.state(), before);
}

#[test]
fn dispatch_failure_reports_exact_retry_after_successful_push() {
    let mut fixture = Fixture::new();
    fixture.options.check_only = false;
    fixture.runner.fail_dispatch = true;
    let error = fixture.execute(|_, _| Ok(())).unwrap_err();
    assert!(format!("{error:#}").contains("Retry only: gh workflow run release.yaml --ref v0.2.0"));
    assert_eq!(
        fixture.remote_head(),
        git(&fixture.remote, &["rev-parse", "refs/tags/v0.2.0"]).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn rejected_atomic_push_changes_neither_remote_branch_nor_tag() {
    let mut fixture = Fixture::new();
    fixture.options.check_only = false;
    let before = fixture.remote_head();
    fixture.hook(
        &fixture.remote.join("hooks/pre-receive"),
        "#!/bin/sh\nexit 1\n",
    );
    assert!(fixture.execute(|_, _| Ok(())).is_err());
    assert_eq!(fixture.remote_head(), before);
    assert!(git(&fixture.remote, &["tag", "--list"]).unwrap().is_empty());
    assert!(
        !fixture
            .runner
            .commands
            .borrow()
            .iter()
            .any(|args| args.iter().any(|arg| arg == "workflow"))
    );
}

#[cfg(unix)]
#[test]
fn commit_hook_change_prevents_push() {
    let mut fixture = Fixture::new();
    fixture.options.check_only = false;
    let hooks = fixture.root.join(".git/hooks");
    git(
        &fixture.root,
        &["config", "core.hooksPath", hooks.to_str().unwrap()],
    )
    .unwrap();
    fixture.hook(
        &hooks.join("pre-commit"),
        "#!/bin/sh\necho changed > tracked\ngit add tracked\n",
    );
    let before = fixture.remote_head();
    assert!(
        fixture
            .execute(|_, _| Ok(()))
            .unwrap_err()
            .to_string()
            .contains("Commit hooks changed")
    );
    assert_eq!(fixture.remote_head(), before);
}

#[test]
fn release_lock_is_exclusive_and_released_on_drop() {
    let fixture = Fixture::new();
    let lock = Lock::acquire(&fixture.root).unwrap();
    let path = fixture.root.join(".artifacts/release.lock");
    assert!(Lock::acquire(&fixture.root).is_err());
    drop(lock);
    assert!(path.is_file(), "The lock inode must remain in place");
    // A second handle cannot read the byte range while the owner's exclusive
    // lock is held on Windows (ERROR_LOCK_VIOLATION), so verify the owner's
    // content only after releasing.
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!("pid={}\n", std::process::id()),
        "A contender changed the owner's lock file"
    );
    assert!(Lock::acquire(&fixture.root).is_ok());
}

#[test]
fn release_lock_child() {
    let Some(root) = std::env::var_os("TERMINATOR_RELEASE_LOCK_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let _lock = Lock::acquire(&root).unwrap();
    fs::write(root.join(".artifacts/lock-ready"), "ready").unwrap();
    loop {
        std::thread::park();
    }
}

#[test]
fn forced_process_exit_releases_lock_without_deleting_file() {
    let fixture = Fixture::new();
    let mut child = crate::harness::Process(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "release::tests::release_lock_child",
                "--nocapture",
            ])
            .env("TERMINATOR_RELEASE_LOCK_TEST_ROOT", &fixture.root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now()
        .checked_add(std::time::Duration::from_secs(5))
        .unwrap();
    while !fixture.root.join(".artifacts/lock-ready").exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "Lock child exited before acquiring the lock"
        );
        assert!(Instant::now() < deadline, "Lock child did not become ready");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(Lock::acquire(&fixture.root).is_err());
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    assert!(fixture.root.join(".artifacts/release.lock").is_file());
    assert!(Lock::acquire(&fixture.root).is_ok());
}

#[test]
fn legacy_directory_is_not_silently_removed_or_bypassed() {
    let fixture = Fixture::new();
    let path = fixture.root.join(".artifacts/release.lock");
    fs::create_dir_all(&path).unwrap();
    let error = Lock::acquire(&fixture.root)
        .err()
        .expect("Legacy lock must block");
    assert!(error.to_string().contains("Legacy release lock directory"));
    assert!(path.is_dir());
}

#[test]
fn version_bump_preserves_comments_and_external_package_versions() {
    let fixture = Fixture::new();
    let path = fixture.root.join("Cargo.lock");
    let mut lock = fs::read_to_string(&path).unwrap();
    lock.push_str("\n[[package]]\nname = \"sample\"\nversion = \"0.1.0\"\nsource = \"registry+https://example.com\"\n");
    fs::write(&path, lock).unwrap();
    assert_eq!(version::bump(&fixture.root).unwrap(), "0.2.0");
    let lock = fs::read_to_string(path).unwrap();
    assert!(lock.starts_with("# retain lock comment"));
    assert!(lock.contains("version = \"0.1.0\"\nsource"));
}

#[test]
fn missing_lockfile_member_does_not_change_either_file() {
    let fixture = Fixture::new();
    fs::write(
        fixture.root.join("Cargo.lock"),
        "version = 4\n[[package]]\nname = \"other\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let before = fixture.state();
    assert!(version::bump(&fixture.root).is_err());
    assert_eq!(fixture.state(), before);
}

#[derive(Default)]
struct FakeRunner {
    commands: RefCell<Vec<Vec<String>>>,
    fail: Option<&'static str>,
    architecture: Option<&'static str>,
    image_architecture: Option<&'static str>,
}
impl Runner for FakeRunner {
    fn run(&self, command: &mut Command) -> Result<()> {
        let args = arguments(command);
        self.commands.borrow_mut().push(args.clone());
        ensure!(
            !self
                .fail
                .is_some_and(|fail| args.iter().any(|arg| arg == fail)),
            "fixture check failure"
        );
        Ok(())
    }
    fn output(&self, command: &mut Command) -> Result<String> {
        let args = arguments(command);
        self.commands.borrow_mut().push(args.clone());
        ensure!(
            !self
                .fail
                .is_some_and(|fail| args.iter().any(|arg| arg == fail)),
            "fixture output failure"
        );
        if args.iter().any(|arg| arg == "{{.Architecture}}") {
            return Ok(if args.iter().any(|arg| arg == "info") {
                self.architecture.unwrap_or("x86_64")
            } else {
                self.image_architecture.unwrap_or("amd64")
            }
            .into());
        }
        Ok("C:/Users/Fixture/run".into())
    }
    fn logged(&self, command: &mut Command, path: &Path) -> Result<()> {
        fs::write(path, "fixture log\n")?;
        self.run(command)
    }
}

#[test]
fn failed_host_retains_logs_and_does_not_start_other_platforms() {
    let fixture = Fixture::new();
    let artifacts = fixture.root.join(".artifacts/releases/run-test");
    fs::create_dir_all(&artifacts).unwrap();
    let runner = FakeRunner {
        fail: Some("bash"),
        ..FakeRunner::default()
    };
    let error = validate(&fixture.root, &artifacts, &Options::default(), &runner).unwrap_err();
    assert!(error.to_string().contains("host failed"));
    assert_eq!(runner.commands.borrow().len(), 3);
    assert_eq!(
        fs::read_to_string(artifacts.join("host/run.log")).unwrap(),
        "fixture log\n"
    );
    let results: Value =
        serde_json::from_slice(&fs::read(artifacts.join("results.json")).unwrap()).unwrap();
    assert_eq!(results["host"]["status"], "failed");
}

#[test]
fn unit_failure_stops_before_runtime_and_packaging() {
    let fixture = Fixture::new();
    let runner = FakeRunner {
        fail: Some("test"),
        ..FakeRunner::default()
    };
    assert!(
        checks::run_with(
            &fixture.root,
            &fixture.root.join(".artifacts/check"),
            checks::Platform::Mac,
            &runner
        )
        .is_err()
    );
    let commands = runner.commands.borrow();
    assert_eq!(
        commands.last().unwrap(),
        &["sh", "scripts/check.sh", "test"]
    );
    assert!(
        !commands
            .iter()
            .any(|args| args.iter().any(|arg| arg == "package"))
    );
}

#[test]
fn mac_checks_include_cross_check_runtime_gui_and_package() {
    let fixture = Fixture::new();
    let runner = FakeRunner::default();
    checks::run_with(
        &fixture.root,
        &fixture.root.join(".artifacts/check"),
        checks::Platform::Mac,
        &runner,
    )
    .unwrap();
    let commands = runner.commands.borrow();
    for expected in [
        "windows-clippy",
        "test",
        "integration",
        "idle-close",
        "gui",
        "package",
    ] {
        assert!(
            commands
                .iter()
                .any(|args| args.iter().any(|arg| arg == expected)),
            "Missing {expected}"
        );
    }
}

#[test]
fn linux_checks_use_xvfb_and_the_ci_coverage_threshold() {
    let fixture = Fixture::new();
    let runner = FakeRunner::default();
    checks::run_with(
        &fixture.root,
        &fixture.root.join(".artifacts/check"),
        checks::Platform::Linux,
        &runner,
    )
    .unwrap();
    let commands = runner.commands.borrow();
    assert!(
        commands
            .iter()
            .any(|args| args.first().is_some_and(|arg| arg == "xvfb-run"))
    );
    assert!(commands.iter().any(|args| {
        args.windows(2)
            .any(|pair| pair == ["--fail-under-lines", "50"])
    }));
}

#[test]
fn windows_checks_do_not_claim_unix_runtime_coverage() {
    let fixture = Fixture::new();
    let runner = FakeRunner::default();
    checks::run_with(
        &fixture.root,
        &fixture.root.join(".artifacts/check"),
        checks::Platform::Windows,
        &runner,
    )
    .unwrap();
    let commands = runner.commands.borrow();
    assert!(
        commands
            .iter()
            .any(|args| args.iter().any(|arg| arg == "package"))
    );
    assert!(
        !commands
            .iter()
            .any(|args| args.iter().any(|arg| arg == "integration" || arg == "gui"))
    );
}

#[test]
fn windows_failure_still_collects_artifacts() {
    struct WindowsRunner(RefCell<Vec<Vec<String>>>);
    impl Runner for WindowsRunner {
        fn run(&self, command: &mut Command) -> Result<()> {
            self.0.borrow_mut().push(arguments(command));
            if command.get_program() == "ssh"
                && windows::decoded_script(command).contains("scripts/release-check.sh")
            {
                bail!("fixture Windows failure");
            }
            Ok(())
        }
        fn output(&self, _: &mut Command) -> Result<String> {
            Ok("C:/Users/Fixture/run".into())
        }
        fn logged(&self, _: &mut Command, _: &Path) -> Result<()> {
            unreachable!()
        }
    }
    let fixture = Fixture::new();
    let runner = WindowsRunner(RefCell::default());
    let error = windows::run_with(
        &fixture.root,
        &fixture.root.join(".artifacts/windows"),
        "windows-fixture",
        "C:/Program Files/Git/bin/bash.exe",
        &runner,
    )
    .unwrap_err();
    assert!(error.to_string().contains("fixture Windows failure"));
    let commands = runner.0.borrow();
    assert_eq!(commands.len(), 4);
    assert!(
        commands
            .last()
            .unwrap()
            .iter()
            .any(|arg| arg.ends_with("artifacts.tar.gz"))
    );
}

#[test]
fn windows_host_cannot_be_an_ssh_option() {
    assert!(windows::validate_host("-oProxyCommand=bad").is_err());
    assert!(windows::validate_host("").is_err());
    assert!(windows::validate_host("user@windows").is_ok());
}

#[cfg(unix)]
#[test]
fn logged_command_retains_stdout_stderr_and_failure() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("run.log");
    assert!(
        System
            .logged(
                &mut cmd(
                    temporary.path(),
                    "sh",
                    &["-c", "echo stdout; echo stderr >&2; exit 7"]
                ),
                &path
            )
            .is_err()
    );
    let log = fs::read_to_string(path).unwrap();
    assert!(log.contains("stdout"));
    assert!(log.contains("stderr"));
}

#[test]
fn release_options_preserve_explicit_windows_skip() {
    use clap::Parser;
    let args = crate::Args::try_parse_from([
        "xtask",
        "release",
        "--skip-windows",
        "--skip-windows",
        "--check-only",
    ])
    .unwrap();
    let crate::Task::Release(options) = args.task else {
        panic!("Expected release")
    };
    assert!(options.skip_windows && options.check_only);
}

#[test]
fn missing_windows_runner_requires_an_explicit_skip() {
    let mut options = Options::default();
    assert!(validate_windows_configuration(&options).is_err());
    options.skip_windows = true;
    assert!(validate_windows_configuration(&options).is_ok());
    options.skip_windows = false;
    options.windows_host = Some("windows-fixture".into());
    assert!(validate_windows_configuration(&options).is_ok());
}

#[test]
fn ssh_passes_unicode_powershell_as_encoded_arguments() {
    let script = "Write-Output 'spaces, $variables, and שלום'";
    let command = windows::powershell(Path::new("."), "windows-fixture", script);
    assert_eq!(command.get_program(), "ssh");
    assert_eq!(windows::decoded_script(&command), script);
    assert!(arguments(&command).iter().any(|arg| arg == "BatchMode=yes"));
}

#[cfg(unix)]
#[test]
fn repository_hook_skips_only_the_validated_staged_tree() {
    let fixture = Fixture::new();
    let hook = fixture.root.join("fixture-hook");
    fixture.hook(
        &hook,
        include_str!("../../../../.cargo-husky/hooks/pre-commit"),
    );
    fs::create_dir_all(fixture.root.join("scripts")).unwrap();
    fs::write(
        fixture.root.join("scripts/check.sh"),
        "#!/bin/sh\ntouch check-ran\nexit 3\n",
    )
    .unwrap();
    let tree = git(&fixture.root, &["write-tree"]).unwrap();
    System
        .run(
            cmd(&fixture.root, "sh", &[])
                .arg(&hook)
                .env("TERMINATOR_RELEASE_VALIDATED_TREE", &tree),
        )
        .unwrap();
    assert!(!fixture.root.join("check-ran").exists());
    assert!(
        System
            .run(
                cmd(&fixture.root, "sh", &[])
                    .arg(&hook)
                    .env("TERMINATOR_RELEASE_VALIDATED_TREE", "different-tree")
            )
            .is_err()
    );
    assert!(fixture.root.join("check-ran").exists());
}

#[test]
fn invalid_version_does_not_change_either_manifest() {
    let fixture = Fixture::new();
    let manifest = fixture.root.join("Cargo.toml");
    let source = fs::read_to_string(&manifest)
        .unwrap()
        .replace("0.1.0", "0.1.0-beta");
    fs::write(manifest, source).unwrap();
    let before = fixture.state();
    assert!(version::bump(&fixture.root).is_err());
    assert_eq!(fixture.state(), before);
}

#[test]
fn duplicate_local_lockfile_member_does_not_change_either_manifest() {
    let fixture = Fixture::new();
    let path = fixture.root.join("Cargo.lock");
    let mut source = fs::read_to_string(&path).unwrap();
    source.push_str("\n[[package]]\nname = \"sample\"\nversion = \"0.1.0\"\n");
    fs::write(path, source).unwrap();
    let before = fixture.state();
    assert!(version::bump(&fixture.root).is_err());
    assert_eq!(fixture.state(), before);
}

#[test]
fn docker_plan_keeps_source_read_only_and_records_windows_skip() {
    let fixture = Fixture::new();
    fs::write(
        fixture.root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.97.1\"\ncomponents = [\"rustfmt\", \"clippy\", \"rust-analyzer\"]\n",
    )
    .unwrap();
    fs::create_dir_all(fixture.root.join("scripts")).unwrap();
    fs::write(
        fixture.root.join("scripts/release-linux.Dockerfile"),
        "FROM ubuntu:24.04\n",
    )
    .unwrap();
    let artifacts = fixture.root.join(".artifacts/releases/run-test");
    fs::create_dir_all(&artifacts).unwrap();
    let runner = FakeRunner {
        fail: Some("inspect"),
        ..FakeRunner::default()
    };
    let options = Options {
        skip_windows: true,
        ..Options::default()
    };
    validate(&fixture.root, &artifacts, &options, &runner).unwrap();
    let commands = runner.commands.borrow();
    let build = commands
        .iter()
        .find(|args| args.starts_with(&["docker".into(), "build".into()]))
        .unwrap();
    assert!(
        build
            .iter()
            .any(|arg| arg == "RUST_COMPONENTS=clippy llvm-tools-preview rust-analyzer rustfmt")
    );
    assert_eq!(
        build.last().unwrap(),
        artifacts.join("linux-build-context").to_str().unwrap()
    );
    assert!(
        fs::read_dir(artifacts.join("linux-build-context"))
            .unwrap()
            .next()
            .is_none()
    );
    let docker = commands
        .iter()
        .find(|args| args.starts_with(&["docker".into(), "run".into()]))
        .unwrap();
    assert!(
        docker
            .iter()
            .any(|arg| arg.ends_with("dst=/snapshot,readonly"))
    );
    assert!(docker.iter().any(|arg| arg == "linux/amd64"));
    assert!(docker.windows(2).any(|args| args == ["--pull", "never"]));
    for volume in [
        "terminator-release-linux-target",
        "terminator-release-linux-registry",
        "terminator-release-linux-git",
    ] {
        assert!(docker.iter().any(|arg| arg.contains(volume)));
    }
    let results: Value =
        serde_json::from_slice(&fs::read(artifacts.join("results.json")).unwrap()).unwrap();
    assert_eq!(results["linux"]["status"], "passed");
    assert!(
        results["windows"]["status"]
            .as_str()
            .unwrap()
            .starts_with("skipped explicitly")
    );
}

#[test]
fn image_components_include_repo_requirements_and_coverage_without_duplicates() {
    let toolchain =
        "[toolchain]\ncomponents = [\"rust-analyzer\", \"rustfmt\", \"rustfmt\", \"rust-src\"]"
            .parse()
            .unwrap();
    assert_eq!(
        rust_components(&toolchain).unwrap(),
        "clippy llvm-tools-preview rust-analyzer rust-src rustfmt"
    );
}

#[test]
fn image_components_reject_invalid_values() {
    for source in [
        "[toolchain]\ncomponents = [42]",
        "[toolchain]\ncomponents = [\"--version\"]",
        "[toolchain]\ncomponents = [\"--toolchain nightly\"]",
    ] {
        assert!(rust_components(&source.parse().unwrap()).is_err());
    }
}

#[test]
fn linux_image_key_tracks_setup_but_not_application_source() {
    let dockerfile = b"FROM ubuntu:24.04\n";
    let image = linux_image_tag(dockerfile, "1.97.1", "clippy rustfmt", "linux/amd64");
    assert_eq!(
        image,
        linux_image_tag(dockerfile, "1.97.1", "clippy rustfmt", "linux/amd64")
    );
    assert_ne!(
        image,
        linux_image_tag(
            b"FROM ubuntu:26.04\n",
            "1.97.1",
            "clippy rustfmt",
            "linux/amd64"
        )
    );
    assert_ne!(
        image,
        linux_image_tag(dockerfile, "1.98.0", "clippy rustfmt", "linux/amd64")
    );
    assert_ne!(
        image,
        linux_image_tag(
            dockerfile,
            "1.97.1",
            "clippy rustfmt rust-src",
            "linux/amd64"
        )
    );
}

#[test]
fn linux_reuses_matching_image_and_explicit_refresh_rebuilds_it() {
    for refresh in [false, true] {
        let fixture = Fixture::new();
        fs::write(
            fixture.root.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.97.1\"\n",
        )
        .unwrap();
        fs::create_dir_all(fixture.root.join("scripts")).unwrap();
        fs::write(
            fixture.root.join("scripts/release-linux.Dockerfile"),
            "FROM ubuntu:24.04\n",
        )
        .unwrap();
        let artifacts = fixture.root.join(".artifacts/releases/run-cache");
        fs::create_dir_all(&artifacts).unwrap();
        let runner = FakeRunner::default();
        let options = Options {
            skip_windows: true,
            rebuild_linux_image: refresh,
            ..Options::default()
        };
        validate(&fixture.root, &artifacts, &options, &runner).unwrap();
        let commands = runner.commands.borrow();
        assert_eq!(
            commands
                .iter()
                .any(|args| args.starts_with(&["docker".into(), "build".into()])),
            refresh
        );
        assert_eq!(
            commands
                .iter()
                .any(|args| args.iter().any(|arg| arg == "inspect")),
            !refresh
        );
        let run = commands
            .iter()
            .find(|args| args.starts_with(&["docker".into(), "run".into()]))
            .unwrap();
        assert!(
            run.iter()
                .any(|arg| arg.starts_with("terminator-release-check:config-"))
        );
        let results: Value =
            serde_json::from_slice(&fs::read(artifacts.join("results.json")).unwrap()).unwrap();
        assert_eq!(
            results["linux-image"]["status"],
            if refresh { "passed" } else { "cached" }
        );
    }
}

#[test]
fn failed_linux_image_build_stops_before_linux_tests() {
    let fixture = Fixture::new();
    fs::write(
        fixture.root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.97.1\"\n",
    )
    .unwrap();
    fs::create_dir_all(fixture.root.join("scripts")).unwrap();
    fs::write(
        fixture.root.join("scripts/release-linux.Dockerfile"),
        "FROM ubuntu:24.04\n",
    )
    .unwrap();
    let artifacts = fixture.root.join(".artifacts/releases/run-build-failure");
    fs::create_dir_all(&artifacts).unwrap();
    let runner = FakeRunner {
        fail: Some("build"),
        ..FakeRunner::default()
    };
    let options = Options {
        skip_windows: true,
        rebuild_linux_image: true,
        ..Options::default()
    };
    let error = validate(&fixture.root, &artifacts, &options, &runner).unwrap_err();
    assert!(error.to_string().contains("linux-image failed"));
    assert!(
        !runner
            .commands
            .borrow()
            .iter()
            .any(|args| args.starts_with(&["docker".into(), "run".into()]))
    );
    let results: Value =
        serde_json::from_slice(&fs::read(artifacts.join("results.json")).unwrap()).unwrap();
    assert_eq!(results["linux-image"]["status"], "failed");
    assert_eq!(results["linux"]["status"], "pending");
}

#[test]
fn explicit_linux_image_uses_local_selection_and_missing_selection_stops() {
    for missing in [false, true] {
        let fixture = Fixture::new();
        fs::write(
            fixture.root.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.97.1\"\n",
        )
        .unwrap();
        let artifacts = fixture.root.join(".artifacts/releases/run-selected-image");
        fs::create_dir_all(&artifacts).unwrap();
        let runner = FakeRunner {
            fail: missing.then_some("inspect"),
            ..FakeRunner::default()
        };
        let options = Options {
            skip_windows: true,
            linux_image: Some("existing:local".into()),
            ..Options::default()
        };
        let result = validate(&fixture.root, &artifacts, &options, &runner);
        let commands = runner.commands.borrow();
        assert!(
            !commands
                .iter()
                .any(|args| args.starts_with(&["docker".into(), "build".into()]))
        );
        let run = commands
            .iter()
            .find(|args| args.starts_with(&["docker".into(), "run".into()]));
        if missing {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("not available locally")
            );
            assert!(run.is_none());
        } else {
            result.unwrap();
            assert!(run.unwrap().iter().any(|arg| arg == "existing:local"));
            assert!(
                fs::read_to_string(artifacts.join("linux-image/run.log"))
                    .unwrap()
                    .contains("explicit --linux-image override")
            );
        }
    }
}

#[test]
fn native_linux_architecture_is_required_and_normalized() {
    for arch in ["aarch64", "arm64", "aarch64\n"] {
        assert_eq!(linux_platform(arch).unwrap(), "linux/arm64");
    }
    for arch in ["amd64", "x86_64"] {
        assert_eq!(linux_platform(arch).unwrap(), "linux/amd64");
    }
    for arch in ["", "armv7l", "unexpected"] {
        assert!(linux_platform(arch).is_err());
    }
    assert_ne!(
        linux_image_tag(b"recipe", "1.97.1", "clippy", "linux/amd64"),
        linux_image_tag(b"recipe", "1.97.1", "clippy", "linux/arm64")
    );
}

#[test]
fn arm_docker_uses_native_runtime_and_separate_build_cache() {
    let fixture = Fixture::new();
    fs::write(
        fixture.root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.97.1\"\n",
    )
    .unwrap();
    fs::create_dir_all(fixture.root.join("scripts")).unwrap();
    fs::write(
        fixture.root.join("scripts/release-linux.Dockerfile"),
        "FROM ubuntu:24.04\n",
    )
    .unwrap();
    let artifacts = fixture.root.join(".artifacts/releases/run-arm");
    fs::create_dir_all(&artifacts).unwrap();
    let runner = FakeRunner {
        architecture: Some("aarch64"),
        fail: Some("inspect"),
        ..FakeRunner::default()
    };
    validate(
        &fixture.root,
        &artifacts,
        &Options {
            skip_windows: true,
            ..Options::default()
        },
        &runner,
    )
    .unwrap();
    let commands = runner.commands.borrow();
    for verb in ["build", "run"] {
        let args = commands
            .iter()
            .find(|args| args.starts_with(&["docker".into(), verb.into()]))
            .unwrap();
        assert!(
            args.windows(2)
                .any(|args| args == ["--platform", "linux/arm64"])
        );
    }
    let run = commands
        .iter()
        .find(|args| args.starts_with(&["docker".into(), "run".into()]))
        .unwrap();
    assert!(
        run.iter()
            .any(|arg| arg.contains("terminator-release-linux-target-linux-arm64"))
    );
    assert!(
        run.iter()
            .any(|arg| arg.contains("linux-arm64,dst=/artifacts"))
    );
}

#[test]
fn emulated_image_is_rejected_before_host_checks() {
    let fixture = Fixture::new();
    let artifacts = fixture.root.join(".artifacts/releases/run-emulated");
    fs::create_dir_all(&artifacts).unwrap();
    let runner = FakeRunner {
        architecture: Some("aarch64"),
        image_architecture: Some("amd64"),
        ..FakeRunner::default()
    };
    let error = validate(
        &fixture.root,
        &artifacts,
        &Options {
            skip_windows: true,
            linux_image: Some("old:x64".into()),
            ..Options::default()
        },
        &runner,
    )
    .unwrap_err();
    assert!(error.to_string().contains("requires native linux/arm64"));
    assert_eq!(runner.commands.borrow().len(), 2);
    assert!(
        runner
            .commands
            .borrow()
            .iter()
            .all(|args| args[0] == "docker")
    );
}
