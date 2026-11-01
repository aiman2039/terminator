use super::*;
#[test]
fn default_release_leaves_linux_to_ci_without_docker() {
    let fixture = Fixture::new();
    let artifacts = fixture.root.join(".artifacts/releases/run-test");
    fs::create_dir_all(&artifacts).unwrap();
    let runner = FakeRunner::default();
    let options = Options {
        skip_windows: true,
        ..Options::default()
    };
    assert!(!options.runs_linux());
    validate(ValidateInput {
        source: &fixture.root,
        artifacts: &artifacts,
        options: &options,
        runner: &runner,
    })
    .unwrap();
    let commands = runner.commands.lock().unwrap();
    assert!(
        !commands
            .iter()
            .any(|args| args.first().is_some_and(|program| program == "docker"))
    );
    let results: Value =
        serde_json::from_slice(&fs::read(artifacts.join("results.json")).unwrap()).unwrap();
    assert!(
        results["linux"]["status"]
            .as_str()
            .unwrap()
            .contains("CI runs Linux checks")
    );
}

#[test]
fn linux_image_options_require_explicit_local_linux() {
    use clap::Parser;
    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        options: Options,
    }
    assert!(Cli::try_parse_from(["release", "--rebuild-linux-image"]).is_err());
    assert!(Cli::try_parse_from(["release", "--linux-image", "x:y"]).is_err());
    assert!(Cli::try_parse_from(["release", "--with-linux", "--skip-linux"]).is_err());
    let cli = Cli::try_parse_from(["release", "--with-linux", "--rebuild-linux-image"]).unwrap();
    assert!(cli.options.runs_linux());
    let cli = Cli::try_parse_from(["release", "--skip-linux"]).unwrap();
    assert!(!cli.options.runs_linux());
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
        with_linux: true,
        ..Options::default()
    };
    validate(ValidateInput {
        source: &fixture.root,
        artifacts: &artifacts,
        options: &options,
        runner: &runner,
    })
    .unwrap();
    let commands = runner.commands.lock().unwrap();
    let build = commands
        .iter()
        .find(|args| args.starts_with(&["docker".into(), "build".into()]))
        .unwrap();
    assert!(
        build
            .iter()
            .any(|arg| arg == "RUST_COMPONENTS=clippy rust-analyzer rustfmt")
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
    assert!(docker.iter().any(|arg| arg.ends_with("dst=/src,readonly")));
    assert!(docker.iter().any(|arg| arg == "linux/amd64"));
    assert!(docker.windows(2).any(|args| args == ["--pull", "never"]));
    for volume in [
        "terminator-release-linux-target",
        "terminator-release-linux-nextest",
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
fn linux_image_key_tracks_setup_but_not_application_source() {
    let dockerfile = b"FROM ubuntu:24.04\n";
    let image = linux_image_tag(LinuxImageTagInput {
        dockerfile,
        rust: "1.97.1",
        components: "clippy rustfmt",
        platform: "linux/amd64",
    });
    assert_eq!(
        image,
        linux_image_tag(LinuxImageTagInput {
            dockerfile,
            rust: "1.97.1",
            components: "clippy rustfmt",
            platform: "linux/amd64"
        })
    );
    assert_ne!(
        image,
        linux_image_tag(LinuxImageTagInput {
            dockerfile: b"FROM ubuntu:26.04\n",
            rust: "1.97.1",
            components: "clippy rustfmt",
            platform: "linux/amd64"
        })
    );
    assert_ne!(
        image,
        linux_image_tag(LinuxImageTagInput {
            dockerfile,
            rust: "1.98.0",
            components: "clippy rustfmt",
            platform: "linux/amd64"
        })
    );
    assert_ne!(
        image,
        linux_image_tag(LinuxImageTagInput {
            dockerfile,
            rust: "1.97.1",
            components: "clippy rustfmt rust-src",
            platform: "linux/amd64"
        })
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
            with_linux: true,
            rebuild_linux_image: refresh,
            ..Options::default()
        };
        validate(ValidateInput {
            source: &fixture.root,
            artifacts: &artifacts,
            options: &options,
            runner: &runner,
        })
        .unwrap();
        let commands = runner.commands.lock().unwrap();
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
        with_linux: true,
        rebuild_linux_image: true,
        ..Options::default()
    };
    let error = validate(ValidateInput {
        source: &fixture.root,
        artifacts: &artifacts,
        options: &options,
        runner: &runner,
    })
    .unwrap_err();
    assert!(error.to_string().contains("linux-image failed"));
    assert!(
        !runner
            .commands
            .lock()
            .unwrap()
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
            with_linux: true,
            linux_image: Some("existing:local".into()),
            ..Options::default()
        };
        let result = validate(ValidateInput {
            source: &fixture.root,
            artifacts: &artifacts,
            options: &options,
            runner: &runner,
        });
        let commands = runner.commands.lock().unwrap();
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
