use super::*;
fn project(id: &str, host: &str, group: Option<&str>) -> RemoteProject {
    RemoteProject {
        id: id.into(),
        name: id.into(),
        ssh: SshOptions {
            host: host.into(),
            ..SshOptions::default()
        },
        root: format!("/srv/{id}"),
        layout: serde_json::Value::Null,
        group: group.map(Into::into),
        forwards: Vec::new(),
        last_connected: 0,
    }
}
fn names(groups: &[HostGroup<'_>]) -> Vec<(Option<String>, Vec<String>)> {
    groups
        .iter()
        .map(|g| {
            (
                g.name.map(Into::into),
                g.projects.iter().map(|p| p.name.clone()).collect(),
            )
        })
        .collect()
}

#[test]
fn hosts_group_by_folder_with_ungrouped_last() {
    let projects = vec![
        project("web", "beta", None),
        project("db", "alpha", Some("prod")),
        project("api", "alpha", Some("prod")),
        project("ci", "gamma", Some("Lab")),
    ];
    assert_eq!(
        names(&group_hosts(&projects, "", false)),
        vec![
            (Some("Lab".into()), vec!["ci".into()]),
            (Some("prod".into()), vec!["api".into(), "db".into()]),
            (None, vec!["web".into()]),
        ]
    );
}

#[test]
fn recent_first_sorts_by_last_connection_then_name() {
    let mut projects = vec![
        project("a", "alpha", None),
        project("b", "beta", None),
        project("c", "gamma", None),
    ];
    projects[1].last_connected = 200;
    projects[2].last_connected = 100;
    assert_eq!(
        names(&group_hosts(&projects, "", true)),
        vec![(None, vec!["b".into(), "c".into(), "a".into()])]
    );
}

#[test]
fn typed_targets_parse_user_host_and_port() {
    let target = parse_target(" me@build.example.com:2222 ").unwrap();
    assert_eq!(target.username, "me");
    assert_eq!(target.host, "build.example.com");
    assert_eq!(target.port, Some(2222));
    let plain = parse_target("alias").unwrap();
    assert_eq!((plain.username.as_str(), plain.port), ("", None));
    assert_eq!(parse_target("fe80::1").unwrap().host, "fe80::1");
    for bad in ["", "me@", "host:0", "host:port", "-oProxyCommand=x", "a b"] {
        assert!(parse_target(bad).is_none(), "{bad}");
    }
}

#[test]
fn host_filter_matches_name_host_root_and_group() {
    let projects = vec![
        project("web", "beta", None),
        project("db", "alpha", Some("Prod")),
    ];
    let only = |filter| {
        names(&group_hosts(&projects, filter, false))
            .into_iter()
            .flat_map(|(_, p)| p)
            .collect::<Vec<_>>()
    };
    assert_eq!(only("PROD"), ["db"]);
    assert_eq!(only(" beta "), ["web"]);
    assert_eq!(only("/srv/db"), ["db"]);
    assert!(only("nothing").is_empty());
}

#[test]
fn quick_connect_lists_only_unsaved_aliases() {
    let config = vec![
        ConfigHost {
            alias: "alpha".into(),
            hostname: "alpha.example".into(),
            ..ConfigHost::default()
        },
        ConfigHost {
            alias: "beta".into(),
            hostname: "beta.example".into(),
            ..ConfigHost::default()
        },
    ];
    let projects = vec![project("db", "alpha", None)];
    let aliases: Vec<_> = quick_aliases(&config, &projects, "")
        .into_iter()
        .map(|h| h.alias.as_str())
        .collect();
    assert_eq!(aliases, ["beta"]);
    assert!(quick_aliases(&config, &projects, "zzz").is_empty());
}
