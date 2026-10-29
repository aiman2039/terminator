use super::*;
#[test]
fn ssh_arguments_require_trust_and_disable_implicit_forwarding() {
    let config = SshOptions {
        host: "fixture".into(),
        username: "developer".into(),
        ..SshOptions::default()
    };
    let args = arguments(&config, false, None).unwrap();
    for argument in [
        "-T",
        "-oBatchMode=yes",
        "-oStrictHostKeyChecking=yes",
        "-oForwardAgent=no",
        "-oPermitLocalCommand=no",
        "-oClearAllForwardings=yes",
        "-oControlMaster=no",
    ] {
        assert!(args.contains(&OsString::from(argument)));
    }
    assert!(
        !args
            .iter()
            .any(|a| a.to_string_lossy().starts_with("-oControlPath"))
    );
}
#[test]
fn ssh_terminals_may_prompt_but_keep_forwarding_disabled() {
    let config = SshOptions {
        host: "fixture".into(),
        ..SshOptions::default()
    };
    let args = arguments(&config, true, None).unwrap();
    for argument in [
        "-tt",
        "-oStrictHostKeyChecking=ask",
        "-oForwardAgent=no",
        "-oPermitLocalCommand=no",
        "-oClearAllForwardings=yes",
    ] {
        assert!(args.contains(&OsString::from(argument)), "{argument}");
    }
    assert!(
        !args
            .iter()
            .any(|a| a.to_string_lossy().starts_with("-oBatchMode"))
    );
    assert!(!args.contains(&OsString::from("-oStrictHostKeyChecking=yes")));
}
#[test]
fn forward_master_is_a_batch_master_that_persists_until_exit() {
    let config = SshOptions {
        host: "fixture".into(),
        ..SshOptions::default()
    };
    let control = Path::new("/tmp/terminator-x/ssh-0000abcd");
    let args = forward_master_arguments(&config, control).unwrap();
    for argument in [
        "-T",
        "-f",
        "-N",
        "-oBatchMode=yes",
        "-oStrictHostKeyChecking=yes",
        "-oClearAllForwardings=yes",
        "-oControlMaster=yes",
        "-oControlPersist=yes",
    ] {
        assert!(args.contains(&OsString::from(argument)), "{argument}");
    }
    assert!(!args.contains(&OsString::from("-oControlMaster=auto")));
    assert!(!args.contains(&OsString::from("-oControlPersist=10m")));
    assert!(
        !args
            .iter()
            .any(|a| a.to_string_lossy().starts_with("-L") || a.to_string_lossy() == "-g")
    );
}
#[test]
fn forward_control_sends_a_loopback_only_local_forward() {
    let control = Path::new("/tmp/terminator-x/ssh-0000abcd");
    let forward = crate::remote::Forward {
        local_port: 8080,
        remote_host: "db".into(),
        remote_port: 5432,
        auto_start: false,
    };
    for (start, operation) in [(true, "forward"), (false, "cancel")] {
        let command = forward_control(start, control, &forward).unwrap();
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args.get(..4).unwrap(), ["-F", "none", "-O", operation]);
        let at = args.iter().position(|a| a == "-L").unwrap();
        assert_eq!(args.get(at + 1).unwrap(), "127.0.0.1:8080:db:5432");
        assert!(!args.iter().any(|a| a.contains("ClearAllForwardings")));
        assert!(
            !args
                .iter()
                .any(|a| a.contains("0.0.0.0") || a.starts_with('*'))
        );
        assert_eq!(args.last().unwrap(), "terminator-control");
    }
    let bad = crate::remote::Forward {
        remote_host: "-oProxyCommand=x".into(),
        ..forward
    };
    assert!(forward_control(true, control, &bad).is_err());
}
#[test]
fn openssh_stderr_is_classified() {
    for (stderr, expected) in [
        (
            "No ED25519 host key is known for fixture and you have requested strict checking.\r\nHost key verification failed.\r\n",
            SshFailure::UnknownHostKey,
        ),
        (
            "Host key verification failed.\n",
            SshFailure::UnknownHostKey,
        ),
        (
            "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\nIT IS POSSIBLE THAT SOMEONE IS DOING SOMETHING NASTY!\nHost key for fixture has changed and you have requested strict checking.\nHost key verification failed.\n",
            SshFailure::ChangedHostKey,
        ),
        (
            "developer@fixture: Permission denied (publickey,password).\r\n",
            SshFailure::AuthFailed,
        ),
        (
            "Received disconnect from 10.0.0.2 port 22:2: Too many authentication failures\n",
            SshFailure::AuthFailed,
        ),
        (
            "ssh: connect to host fixture port 2222: Connection refused\r\n",
            SshFailure::Unreachable,
        ),
        (
            "ssh: Could not resolve hostname nowhere.invalid: nodename nor servname provided, or not known\r\n",
            SshFailure::Unreachable,
        ),
        (
            "ssh: connect to host 10.255.255.1 port 22: Connection timed out\r\n",
            SshFailure::Unreachable,
        ),
        (
            "ssh: connect to host 10.255.255.1 port 22: Operation timed out\r\n",
            SshFailure::Unreachable,
        ),
        (
            "  sh: git: command not found\n",
            SshFailure::Other("sh: git: command not found".into()),
        ),
    ] {
        assert_eq!(classify(stderr), expected, "{stderr}");
    }
    assert!(SshError::from_stderr(b"sh: git: command not found").is_none());
    let error = SshError::from_stderr(b"Host key verification failed.\n").unwrap();
    assert_eq!(error.failure, SshFailure::UnknownHostKey);
    assert!(error.message.ends_with("Host key verification failed."));
}
#[test]
fn known_hosts_lines_bracket_non_default_ports() {
    let key = HostKey {
        key_type: "ssh-ed25519".into(),
        key: "AAAAC3NzaC1lZDI1NTE5AAAAIHNvbWUta2V5".into(),
        fingerprint: "SHA256:ignored".into(),
    };
    let mut resolved = Resolved {
        hostname: "fixture.example".into(),
        port: 22,
        host_key_alias: None,
    };
    assert_eq!(
        known_hosts_line(&resolved.known_hosts_host(), &key),
        "fixture.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHNvbWUta2V5\n"
    );
    resolved.port = 2222;
    assert_eq!(
        known_hosts_line(&resolved.known_hosts_host(), &key),
        "[fixture.example]:2222 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHNvbWUta2V5\n"
    );
    resolved.host_key_alias = Some("alias".into());
    assert_eq!(resolved.known_hosts_host(), "alias");
}
#[test]
fn keyscan_and_keygen_output_is_parsed() {
    let scan = "# fixture.example:2222 SSH-2.0-OpenSSH_9.6\n\
        [fixture.example]:2222 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHNvbWUta2V5\n\
        [fixture.example]:2222 ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTY=\n\
        [fixture.example]:2222 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHNvbWUta2V5\n\
        malformed\n\
        host bad$type AAAA\n";
    let keys = parse_keyscan(scan);
    assert_eq!(
        keys.iter().map(|k| k.key_type.as_str()).collect::<Vec<_>>(),
        ["ssh-ed25519", "ecdsa-sha2-nistp256"]
    );
    let mut reversed = keys.clone();
    reversed.reverse();
    assert!(same_keys(&keys, &reversed));
    assert!(!same_keys(&keys, &keys[..1]));
    assert!(!same_keys(&[], &[]));
    assert_eq!(
        parse_fingerprints(
            "256 SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s fixture (ED25519)\n\
             256 SHA256:Ab/cD+ef fixture (ECDSA)\n"
        ),
        [
            "SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s",
            "SHA256:Ab/cD+ef"
        ]
    );
    let resolved =
        parse_resolved("user developer\nhostname fixture.example\nport 2222\nhostkeyalias none\n")
            .unwrap();
    assert_eq!(resolved.hostname, "fixture.example");
    assert_eq!(resolved.port, 2222);
    assert_eq!(resolved.host_key_alias, None);
    assert!(parse_resolved("hostname -oProxyCommand=x\nport 22\n").is_err());
}
#[test]
fn saved_ports_stay_explicit_and_missing_ports_defer_to_config() {
    let saved: SshOptions =
        serde_json::from_str(r#"{"host":"build","port":22,"username":"dev"}"#).unwrap();
    assert_eq!(saved.port, Some(22));
    for json in [r#"{"host":"build"}"#, r#"{"host":"build","port":null}"#] {
        let options: SshOptions = serde_json::from_str(json).unwrap();
        assert_eq!(options.port, None, "{json}");
    }
    for options in [
        saved.clone(),
        SshOptions {
            port: None,
            ..saved.clone()
        },
    ] {
        let text = serde_json::to_string(&options).unwrap();
        assert_eq!(text.contains("\"port\""), options.port.is_some());
        assert_eq!(serde_json::from_str::<SshOptions>(&text).unwrap(), options);
    }
    let port = |options: &SshOptions| {
        let args = arguments(options, false, None).unwrap();
        args.iter()
            .position(|a| a == "-p")
            .and_then(|i| args.get(i.checked_add(1)?).cloned())
    };
    assert_eq!(port(&saved), Some(OsString::from("22")));
    assert_eq!(
        port(&SshOptions {
            port: None,
            ..saved.clone()
        }),
        None
    );
    assert!(
        SshOptions {
            port: Some(0),
            ..saved.clone()
        }
        .validate()
        .is_err()
    );
    assert_eq!(saved.authority(), "dev@build:22");
    assert_eq!(
        SshOptions {
            port: None,
            username: String::new(),
            ..saved
        }
        .authority(),
        "build"
    );
}
#[test]
fn ssh_arguments_reuse_the_given_control_socket() {
    let config = SshOptions {
        host: "fixture".into(),
        ..SshOptions::default()
    };
    let args = arguments(&config, false, Some(Path::new("/tmp/t-1/ssh-0a%b"))).unwrap();
    for argument in [
        "-oControlMaster=auto",
        "-oControlPath=\"/tmp/t-1/ssh-0a%%b\"",
        "-oControlPersist=10m",
        "-oClearAllForwardings=yes",
    ] {
        assert!(args.contains(&OsString::from(argument)), "{argument}");
    }
    assert!(!args.contains(&OsString::from("-oControlMaster=no")));
}
#[test]
#[cfg(unix)]
fn control_sockets_fit_unix_limits_and_separate_endpoints() {
    let paths = crate::Paths {
        data: "/unused".into(),
        runtime: "/tmp/terminator-ffffffffffffffff".into(),
    };
    let long = SshOptions {
        host: "h".repeat(255),
        username: "u".repeat(128),
        identity_file: Some(PathBuf::from(format!("/{}", "k".repeat(4000)))),
        ..SshOptions::default()
    };
    let control = paths.ssh_control(&long).unwrap();
    assert!(control.as_os_str().len() < 100 - 17);
    assert_eq!(paths.ssh_control(&long), Some(control.clone()));
    let other = SshOptions {
        port: Some(2222),
        ..long.clone()
    };
    assert_ne!(paths.ssh_control(&other), Some(control));
    let deep = crate::Paths {
        data: "/unused".into(),
        runtime: PathBuf::from(format!("/{}", "r".repeat(80))),
    };
    assert_eq!(deep.ssh_control(&long), None);
}
#[test]
#[cfg(windows)]
fn windows_connections_do_not_use_control_sockets() {
    let dir = tempfile::tempdir().unwrap();
    let paths = crate::Paths::at(dir.path().to_path_buf());
    assert_eq!(paths.ssh_control(&SshOptions::default()), None);
}
#[test]
fn control_check_and_exit_target_only_the_socket() {
    let path = Path::new("/tmp/t/ssh-%h");
    for (command, op) in [(check_control(path), "check"), (exit_control(path), "exit")] {
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "-F",
                "none",
                "-O",
                op,
                "-oControlPath=\"/tmp/t/ssh-%%h\"",
                "--",
                "terminator-control"
            ]
        );
    }
}
