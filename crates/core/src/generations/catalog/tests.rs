use super::*;
#[test]
fn an_older_workspace_writer_does_not_erase_remote_projects() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::at(dir.path().into());
    paths.init().unwrap();
    Catalog::import(&paths, &State::default()).unwrap();
    let catalog = Catalog::open(&paths).unwrap();
    catalog
        .0
        .execute("UPDATE control SET active='fixture' WHERE id=1", [])
        .unwrap();
    let p = crate::remote::RemoteProject {
        id: "remote-fixture".into(),
        name: "Remote".into(),
        root: "/workspace/project".into(),
        ssh: crate::ssh::SshOptions {
            host: "fixture".into(),
            ..crate::ssh::SshOptions::default()
        },
        layout: serde_json::Value::Null,
        group: Some("Prod".into()),
        forwards: vec![crate::remote::Forward {
            local_port: 5432,
            remote_host: "db.internal".into(),
            remote_port: 5432,
            auto_start: true,
        }],
    };
    let state = State {
        generation: "fixture".into(),
        remote_projects: vec![p.clone()],
        ..State::default()
    };
    catalog.save_workspace(&state).unwrap();
    let mut legacy = serde_json::to_value(State::default()).unwrap();
    legacy.as_object_mut().unwrap().remove("remote_projects");
    catalog
        .0
        .execute(
            "UPDATE workspace SET json=?1 WHERE id=1",
            [serde_json::to_string(&legacy).unwrap()],
        )
        .unwrap();
    let mut loaded = State::default();
    catalog.refresh(&mut loaded).unwrap();
    assert_eq!(loaded.remote_projects, vec![p.clone()]);
    catalog.remove_remote_project(&p.id).unwrap();
    let mut removed = State::default();
    catalog.refresh(&mut removed).unwrap();
    assert!(removed.remote_projects.is_empty());
}
