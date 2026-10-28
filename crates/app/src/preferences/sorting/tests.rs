use super::*;

fn project(id: &str, name: &str) -> Project {
    Project {
        id: id.into(),
        name: name.into(),
        path: format!("/{id}").into(),
        layout: serde_json::Value::Null,
    }
}

fn session(id: &str, project: &str, created: u64) -> Session {
    Session {
        remote: None,
        review: false,
        id: id.into(),
        project_id: project.into(),
        label: id.into(),
        cwd: format!("/{project}").into(),
        kind: terminator_core::SessionKind::Shell,
        file: None,
        lifecycle: terminator_core::Lifecycle::Running,
        created,
        exit_code: None,
        rows: 24,
        cols: 80,
        generation: "test".into(),
        pid: None,
        truncated: false,
        cwd_confirmed: true,
    }
}

fn ids(input: VisibleProjects<'_>) -> Vec<String> {
    sort_visible_projects(input)
        .into_iter()
        .map(|project| project.id.clone())
        .collect()
}

#[test]
fn name_sort_is_case_insensitive_and_omits_hidden_projects() {
    let hidden = HashSet::from(["skip".into()]);
    let projects = vec![
        project("z", "Banana"),
        project("skip", "aaaa"),
        project("a", "apple"),
        project("m", "Banana"),
    ];
    let empty = HashMap::new();
    let input = |sort: ProjectSort| VisibleProjects {
        projects: &projects,
        hidden: &hidden,
        sort,
        filter: "",
        activity: &empty,
        sessions: &[],
        agents: &[],
        notifications: &[],
        terminal_notices: &[],
    };
    assert_eq!(ids(input(ProjectSort::NameAsc)), ["a", "m", "z"]);
    assert_eq!(ids(input(ProjectSort::NameDesc)), ["z", "m", "a"]);
}

#[test]
fn project_filter_matches_name_or_path_case_insensitively() {
    let hidden = HashSet::new();
    let projects = vec![
        project("term", "terminator"),
        project("fomo", "fomo-rh-fe-sol-be"),
        project("other", "Other"),
    ];
    let empty = HashMap::new();
    let filtered = |filter: &str| {
        ids(VisibleProjects {
            projects: &projects,
            hidden: &hidden,
            sort: ProjectSort::NameAsc,
            filter,
            activity: &empty,
            sessions: &[],
            agents: &[],
            notifications: &[],
            terminal_notices: &[],
        })
    };
    assert_eq!(filtered(""), ["fomo", "other", "term"]);
    assert_eq!(filtered("term"), ["term"]);
    assert_eq!(filtered("FOMO"), ["fomo"]);
    assert_eq!(filtered("fomo-rh"), ["fomo"]);
    assert_eq!(filtered("/fomo"), ["fomo"]);
    assert_eq!(filtered("zzz"), Vec::<String>::new());
}

#[test]
fn latest_activity_uses_max_timestamp_and_name_tie_break() {
    let hidden = HashSet::new();
    let projects = vec![project("z", "zebra"), project("a", "alpha")];
    let sessions = [session("sz", "z", 10), session("sa", "a", 5)];
    let empty = HashMap::new();
    let mut agents = vec![Agent {
        invocation_id: "i".into(),
        session_id: "sa".into(),
        kind: "custom".into(),
        provider_session_id: None,
        state: terminator_core::AgentState::Running,
        sequence: None,
        updated: 20,
        resume: None,
        process: None,
    }];
    let ranked = |activity: &HashMap<String, u64>, agents: &[Agent]| {
        ids(VisibleProjects {
            projects: &projects,
            hidden: &hidden,
            sort: ProjectSort::LatestActivity,
            filter: "",
            activity,
            sessions: &sessions,
            agents,
            notifications: &[],
            terminal_notices: &[],
        })
    };
    assert_eq!(ranked(&empty, &[]), ["z", "a"]);
    assert_eq!(ranked(&empty, &agents), ["a", "z"]);
    agents[0].updated = 10;
    assert_eq!(ranked(&empty, &agents), ["a", "z"]);
    let mut activity = HashMap::new();
    activity.insert("z".into(), 30);
    assert_eq!(ranked(&activity, &agents), ["z", "a"]);
}

#[test]
fn latest_activity_includes_notice_timestamps() {
    let hidden = HashSet::new();
    let empty = HashMap::new();
    let sessions = [session("sa", "a", 1), session("sb", "b", 2)];
    let notifications = [Notification {
        id: "n".into(),
        session_id: "sa".into(),
        invocation_id: "i".into(),
        request_id: None,
        state: terminator_core::AgentState::Completed,
        summary: String::new(),
        details: String::new(),
        created: 8,
        read: true,
        dismissed: false,
        resolved: true,
        snoozed_until: 0,
    }];
    let terminal_notices = [TerminalNotice {
        id: "t".into(),
        session_id: "sb".into(),
        title: String::new(),
        body: String::new(),
        created: 9,
        dismissed: false,
    }];
    assert_eq!(
        ids(VisibleProjects {
            projects: &[project("a", "a"), project("b", "b")],
            hidden: &hidden,
            sort: ProjectSort::LatestActivity,
            filter: "",
            activity: &empty,
            sessions: &sessions,
            agents: &[],
            notifications: &notifications,
            terminal_notices: &terminal_notices,
        }),
        ["b", "a"]
    );
}

fn history_session(id: &str, project: &str, label: &str, created: u64) -> Session {
    let mut s = session(id, project, created);
    s.label = label.into();
    s.lifecycle = terminator_core::Lifecycle::Ended;
    s
}

fn history_agent(session: &str, updated: u64) -> Agent {
    Agent {
        invocation_id: format!("agent-{session}"),
        session_id: session.into(),
        kind: "custom".into(),
        provider_session_id: None,
        state: terminator_core::AgentState::Stopped,
        sequence: None,
        updated,
        resume: None,
        process: None,
    }
}

fn history_groups(input: HistoryGroupsInput<'_>) -> Vec<(String, Vec<String>)> {
    let HistoryGroupsInput {
        projects,
        sessions,
        agents,
        sort,
        filter,
    } = input;
    sort_history(HistoryInput {
        projects,
        sessions,
        agents,
        notifications: &[],
        terminal_notices: &[],
        sort,
        filter,
    })
    .into_iter()
    .map(|group| {
        (
            group.project.id,
            group.sessions.into_iter().map(|s| s.id).collect(),
        )
    })
    .collect()
}

#[test]
fn history_sorts_projects_by_last_activity_and_sessions_by_activity() {
    let projects = vec![project("a", "alpha"), project("b", "beta")];
    let sessions = vec![
        history_session("a-old", "a", "Terminal 1", 10),
        history_session("a-new", "a", "Terminal 4", 12),
        history_session("b-only", "b", "Terminal 2", 11),
    ];
    let agents = vec![history_agent("a-old", 30), history_agent("b-only", 20)];
    let groups = history_groups(HistoryGroupsInput {
        projects,
        sessions: &sessions,
        agents: &agents,
        sort: HistorySort::LatestActivity,
        filter: "",
    });
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].0, "a");
    assert_eq!(groups[0].1, vec!["a-old".to_string(), "a-new".to_string()]);
    assert_eq!(groups[1].0, "b");
}

#[test]
fn history_name_sort_orders_projects_and_sessions() {
    let projects = vec![project("b", "beta"), project("a", "alpha")];
    let sessions = vec![
        history_session("s2", "a", "Terminal 4", 2),
        history_session("s1", "a", "Terminal 1", 1),
    ];
    let groups = history_groups(HistoryGroupsInput {
        projects: projects.clone(),
        sessions: &sessions,
        agents: &[],
        sort: HistorySort::NameAsc,
        filter: "",
    });
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].0, "a");
    assert_eq!(groups[0].1, vec!["s1".to_string(), "s2".to_string()]);
    let groups = history_groups(HistoryGroupsInput {
        projects,
        sessions: &sessions,
        agents: &[],
        sort: HistorySort::NameDesc,
        filter: "",
    });
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].0, "a");
    assert_eq!(groups[0].1, vec!["s2".to_string(), "s1".to_string()]);
    let projects = vec![project("b", "beta"), project("a", "alpha")];
    let sessions = vec![
        history_session("sa", "a", "Terminal 1", 1),
        history_session("sb", "b", "Terminal 2", 1),
    ];
    let groups = history_groups(HistoryGroupsInput {
        projects,
        sessions: &sessions,
        agents: &[],
        sort: HistorySort::NameDesc,
        filter: "",
    });
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].0, "b");
    assert_eq!(groups[1].0, "a");
}

#[test]
fn history_filter_matches_session_and_project_names() {
    let projects = vec![project("a", "alpha"), project("b", "beta")];
    let sessions = vec![
        history_session("s1", "a", "Terminal 1", 1),
        history_session("s2", "a", "Terminal 4", 2),
        history_session("s3", "b", "Terminal 2", 3),
    ];
    let groups = history_groups(HistoryGroupsInput {
        projects: projects.clone(),
        sessions: &sessions,
        agents: &[],
        sort: HistorySort::LatestActivity,
        filter: "terminal 4",
    });
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].0, "a");
    assert_eq!(groups[0].1, vec!["s2".to_string()]);
    let groups = history_groups(HistoryGroupsInput {
        projects,
        sessions: &sessions,
        agents: &[],
        sort: HistorySort::LatestActivity,
        filter: "BETA",
    });
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].0, "b");
    assert_eq!(groups[0].1, vec!["s3".to_string()]);
}
