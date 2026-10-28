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
    }
}
fn names(groups: &[HostGroup<'_>]) -> Vec<(Option<String>, String, Vec<String>)> {
    groups
        .iter()
        .flat_map(|g| {
            g.hosts.iter().map(|(host, projects)| {
                (
                    g.name.map(Into::into),
                    host.clone(),
                    projects.iter().map(|p| p.name.clone()).collect(),
                )
            })
        })
        .collect()
}

#[test]
fn hosts_group_by_folder_then_host_with_ungrouped_last() {
    let projects = vec![
        project("web", "beta", None),
        project("db", "alpha", Some("prod")),
        project("api", "alpha", Some("prod")),
        project("ci", "gamma", Some("Lab")),
    ];
    assert_eq!(
        names(&group_hosts(&projects, "")),
        vec![
            (Some("Lab".into()), "gamma".into(), vec!["ci".into()]),
            (
                Some("prod".into()),
                "alpha".into(),
                vec!["api".into(), "db".into()]
            ),
            (None, "beta".into(), vec!["web".into()]),
        ]
    );
}

#[test]
fn host_filter_matches_name_host_root_and_group() {
    let projects = vec![
        project("web", "beta", None),
        project("db", "alpha", Some("Prod")),
    ];
    let only = |filter| {
        names(&group_hosts(&projects, filter))
            .into_iter()
            .flat_map(|(_, _, p)| p)
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
