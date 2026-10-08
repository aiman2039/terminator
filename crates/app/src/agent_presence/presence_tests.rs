use super::super::*;
use super::cache::PresentationCache;
use super::counts::{AttentionCounts, tab_attention};
use super::owner::{OwnerSupport, owner_support};
use super::queries::{live_sessions, present_session, unverified_sessions};
#[cfg(test)]
mod tests {
    use super::*;

    use terminator_core::{
        Agent, Lifecycle, Notification, SessionKind,
        agents::{PresenceOutcome, ProcessIdentity},
        now,
    };

    fn session(id: &str) -> Session {
        Session {
            review: false,
            id: id.into(),
            project_id: "p".into(),
            label: id.into(),
            cwd: "/tmp".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle: Lifecycle::Running,
            created: 1,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "g".into(),
            pid: Some(100),
            truncated: false,
            cwd_confirmed: true,
        }
    }

    fn capable(state: &mut State) {
        state.capabilities.push(AGENT_PRESENCE_CAPABILITY.into());
    }

    fn observe(state: &mut State, kinds: &[&str], at: u64) {
        state.presence.push(agents::TerminalPresence {
            session_id: "s".into(),
            generation: "g".into(),
            agents: kinds
                .iter()
                .enumerate()
                .map(|(i, kind)| agents::DetectedAgent {
                    kind: (*kind).into(),
                    process: ProcessIdentity {
                        pid: 200u32.saturating_add(u32::try_from(i).unwrap_or(u32::MAX)),
                        start_time: 500,
                    },
                    foreground: i == 0,
                })
                .collect(),
            outcome: PresenceOutcome::Verified,
            observed_at: at,
        });
    }

    fn hook(state: &mut State, kind: &str, lifecycle: AgentState, linked: bool) {
        state.agents.push(Agent {
            invocation_id: format!("agent-{kind}"),
            session_id: "s".into(),
            kind: kind.into(),
            provider_session_id: None,
            state: lifecycle,
            sequence: None,
            updated: now().saturating_sub(3),
            resume: None,
            process: linked.then_some(ProcessIdentity {
                pid: 200,
                start_time: 500,
            }),
        });
    }

    #[test]
    fn cache_reuses_presentations_and_expires_snoozes_and_snapshot_health() {
        let mut state = State::default();
        state.capabilities.push(AGENT_PRESENCE_CAPABILITY.into());
        state.sessions.push(session("s"));
        state.presence.push(agents::TerminalPresence {
            session_id: "s".into(),
            generation: "g".into(),
            agents: vec![],
            outcome: PresenceOutcome::Verified,
            observed_at: 1,
        });
        let mut cache = PresentationCache::default();
        let first = cache.get(&state, "s", 100, Some(true));
        assert!(
            first.verified,
            "successful unchanged snapshots keep presence fresh"
        );
        assert!(std::sync::Arc::ptr_eq(
            &first,
            &cache.get(&state, "s", 200, Some(true))
        ));
        assert!(!cache.get(&state, "s", 200, Some(false)).verified);
        assert!(cache.get(&state, "s", 201, Some(true)).verified);
        // A responsive daemon can explicitly expire a stalled inspector's data.
        state.presence[0].outcome = PresenceOutcome::Unavailable;
        cache.clear();
        assert!(!cache.get(&state, "s", 202, Some(true)).verified);
        state.presence[0].outcome = PresenceOutcome::Verified;
        state.presence[0].observed_at = 203;
        cache.clear();
        assert!(cache.get(&state, "s", 203, Some(true)).verified);
        state.notifications.push(Notification {
            id: "n".into(),
            session_id: "s".into(),
            invocation_id: "i".into(),
            request_id: None,
            state: AgentState::WaitingInput,
            summary: String::new(),
            details: String::new(),
            created: 100,
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: 300,
        });
        cache.clear();
        assert_eq!(cache.get(&state, "s", 299, Some(true)).attention.input, 0);
        assert_eq!(cache.get(&state, "s", 300, Some(true)).attention.input, 1);
        state.sessions[0].lifecycle = Lifecycle::Ended;
        cache.clear();
        let ended = cache.get(&state, "s", 301, Some(true));
        assert!(!ended.verified);
        assert!(ended.attention.is_empty());
    }

    #[test]
    fn verified_detection_pairs_brand_with_hook_status() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        capable(&mut state);
        observe(&mut state, &["codex"], now());
        hook(&mut state, "codex", AgentState::Running, true);
        let presented = present_session(&state, "s", now());
        assert_eq!(presented.brand_icon, Some("AgentCodex"));
        assert_eq!(presented.brand_label.as_deref(), Some("Codex"));
        assert!(presented.verified && presented.live);
        assert_eq!(presented.status_label, "Working");
        assert_eq!(presented.status_icon, "LoaderCircle");
        assert!(presented.spin);
        assert!(presented.diagnostics(now()).contains("process inspection"));
        assert!(
            presented
                .diagnostics(now())
                .contains("linked to live process")
        );
    }

    #[test]
    fn verified_presence_without_hooks_never_claims_working_or_idle() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        capable(&mut state);
        observe(&mut state, &["pi"], now());
        let presented = present_session(&state, "s", now());
        assert!(presented.live);
        assert_eq!(presented.brand_label.as_deref(), Some("Pi"));
        assert_eq!(presented.status_label, "Status unavailable");
        assert!(!presented.spin);
        assert!(presented.diagnostics(now()).contains("Hook: none reported"));
    }

    #[test]
    fn hook_only_state_on_old_daemons_is_labeled_unverified() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        hook(&mut state, "my-agent", AgentState::WaitingInput, false);
        let presented = present_session(&state, "s", now());
        assert!(!presented.verified && !presented.live);
        assert_eq!(presented.brand_icon, Some(agents::GENERIC_ICON));
        assert_eq!(presented.status_label, "Needs input");
        assert!(presented.diagnostics(now()).contains("unsupported daemon"));
        assert!(presented.diagnostics(now()).contains("last reported"));
    }

    #[test]
    fn multiple_agents_without_foreground_use_the_count_indicator() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        capable(&mut state);
        state.presence.push(agents::TerminalPresence {
            session_id: "s".into(),
            generation: "g".into(),
            agents: ["codex", "claude"]
                .into_iter()
                .enumerate()
                .map(|(i, kind)| agents::DetectedAgent {
                    kind: kind.into(),
                    process: ProcessIdentity {
                        pid: 200u32.saturating_add(u32::try_from(i).unwrap_or(u32::MAX)),
                        start_time: 500,
                    },
                    foreground: false,
                })
                .collect(),
            outcome: PresenceOutcome::Verified,
            observed_at: now(),
        });
        let presented = present_session(&state, "s", now());
        assert!(presented.live);
        assert_eq!(presented.brand_icon, Some(agents::MULTIPLE_ICON));
        assert_eq!(presented.brand_label.as_deref(), Some("2 agents"));
        assert!(
            presented.diagnostics(now()).contains("Codex")
                && presented.diagnostics(now()).contains("Claude")
        );
    }

    #[test]
    fn stale_failed_and_unavailable_observations_stay_unverified() {
        for (outcome, age, needle) in [
            (PresenceOutcome::Verified, 30, "stale observation"),
            (PresenceOutcome::Unavailable, 0, "detection unavailable"),
        ] {
            let mut state = State::default();
            state.sessions.push(session("s"));
            capable(&mut state);
            state.presence.push(agents::TerminalPresence {
                session_id: "s".into(),
                generation: "g".into(),
                agents: vec![agents::DetectedAgent {
                    kind: "codex".into(),
                    process: ProcessIdentity {
                        pid: 200,
                        start_time: 500,
                    },
                    foreground: true,
                }],
                outcome,
                observed_at: now().saturating_sub(age),
            });
            hook(&mut state, "codex", AgentState::Running, true);
            let presented = present_session(&state, "s", now());
            assert!(!presented.verified && !presented.live, "{needle}");
            assert!(
                presented.diagnostics(now()).contains("last reported")
                    || presented.diagnostics(now()).contains("Hook: none reported"),
                "{}",
                presented.diagnostics(now())
            );
            assert!(
                presented.diagnostics(now()).contains(needle),
                "{}",
                presented.diagnostics(now())
            );
        }
    }

    #[test]
    fn muse_subagent_tool_after_stop_shows_completed() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        state.agents.push(Agent {
            invocation_id: "lead".into(),
            session_id: "s".into(),
            kind: "muse".into(),
            provider_session_id: Some("01a0e846-a59d-7da0-a7c8-3de6bf83174b".into()),
            state: AgentState::Completed,
            sequence: None,
            updated: 10,
            resume: None,
            process: None,
        });
        state.agents.push(Agent {
            invocation_id: "tool".into(),
            session_id: "s".into(),
            kind: "muse".into(),
            provider_session_id: Some("ee1080b7-a350-4c2b-844b-ae1786b1b3c0".into()),
            state: AgentState::Running,
            sequence: None,
            updated: 50,
            resume: None,
            process: None,
        });
        let presented = present_session(&state, "s", 100);
        assert_eq!(presented.status_label, "Completed");
        assert!(!presented.spin);
    }

    #[test]
    fn unknown_lifecycle_shows_status_unavailable() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        hook(&mut state, "codex", AgentState::Unknown, false);
        let presented = present_session(&state, "s", now());
        assert_eq!(presented.status_label, "Status unavailable");
        assert_eq!(presented.status_icon, "CircleQuestion");
    }

    #[test]
    fn attention_counts_keep_input_permission_and_failure_distinct() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        for (i, lifecycle) in [
            AgentState::WaitingInput,
            AgentState::WaitingInput,
            AgentState::WaitingPermission,
            AgentState::Failed,
            AgentState::Completed,
        ]
        .into_iter()
        .enumerate()
        {
            state.notifications.push(Notification {
                id: format!("n{i}"),
                session_id: "s".into(),
                invocation_id: "a".into(),
                request_id: None,
                state: lifecycle,
                summary: String::new(),
                details: String::new(),
                created: i as u64,
                read: false,
                dismissed: false,
                resolved: false,
                snoozed_until: 0,
            });
        }
        // Dismissed, resolved, and snoozed notices never count.
        state.notifications[4].resolved = true;
        state.notifications.push(Notification {
            id: "snoozed".into(),
            session_id: "s".into(),
            invocation_id: "a".into(),
            request_id: None,
            state: AgentState::WaitingInput,
            summary: String::new(),
            details: String::new(),
            created: 9,
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: now() + 600,
        });
        let counts = tab_attention(&state, &["s".into(), "other".into()], now());
        assert_eq!(
            counts,
            AttentionCounts {
                input: 2,
                permission: 1,
                failed: 1,
            }
        );
        assert_eq!(present_session(&state, "s", now()).unread, 4);
        assert!(tab_attention(&state, &["other".into()], now()).is_empty());
    }

    #[test]
    fn live_and_unverified_session_lists_partition_hook_activity() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        let mut ended = session("ended");
        ended.lifecycle = Lifecycle::Ended;
        state.sessions.push(ended);
        state.sessions.push(session("plain"));
        capable(&mut state);
        observe(&mut state, &["codex"], now());
        hook(&mut state, "codex", AgentState::Running, true);
        state.agents.push(Agent {
            invocation_id: "hook-only".into(),
            session_id: "plain".into(),
            kind: "claude".into(),
            provider_session_id: None,
            state: AgentState::WaitingInput,
            sequence: None,
            updated: now(),
            resume: None,
            process: None,
        });
        let live = live_sessions(&state, now());
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].0.id, "s");
        let unverified = unverified_sessions(&state, now());
        assert_eq!(unverified.len(), 1);
        assert_eq!(unverified[0].id, "plain");
    }

    fn notice(id: &str, session: &str, lifecycle: AgentState, created: u64) -> Notification {
        Notification {
            id: id.into(),
            session_id: session.into(),
            invocation_id: "a".into(),
            request_id: None,
            state: lifecycle,
            summary: format!("note {id}"),
            details: String::new(),
            created,
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: 0,
        }
    }

    #[test]
    fn indexed_reconcile_scans_notifications_once_and_reuses_arcs() {
        let mut state = State::default();
        let sessions = 8usize;
        let notices = 20usize;
        for index in 0..sessions {
            state.sessions.push(session(&format!("s{index}")));
        }
        for index in 0..notices {
            state.notifications.push(notice(
                &format!("n{index}"),
                &format!("s{}", index % sessions),
                AgentState::WaitingInput,
                index as u64,
            ));
        }
        let mut ended = session("ended");
        ended.lifecycle = Lifecycle::Ended;
        state.sessions.push(ended);
        state.notifications.push(notice(
            "ended-note",
            "ended",
            AgentState::WaitingPermission,
            100,
        ));
        let mut cache = PresentationCache::default();
        cache.reconcile(&state, 50, Some(true));
        assert_eq!(cache.indexes_built, 1);
        assert_eq!(cache.notifications_visited, (notices + 1) as u64);
        assert_eq!(cache.presentations_rebuilt, (sessions + 1) as u64);
        let kept = cache.get(&state, "s0", 50, Some(true));
        let stable = cache.get(&state, "s1", 50, Some(true));
        for index in 0..sessions {
            let _ = cache.get(&state, &format!("s{index}"), 50, Some(true));
        }
        assert_eq!(cache.indexes_built, 1);
        assert_eq!(cache.notifications_visited, (notices + 1) as u64);
        let ids = vec!["s0".into(), "s0".into(), "s1".into(), "ended".into()];
        assert_eq!(
            cache.attention(&state, &ids, 50, Some(true)),
            tab_attention(&state, &ids, 50)
        );
        assert!(cache.attention(&state, &ids, 50, Some(true)).input > 0);
        assert_eq!(cache.indexes_built, 1);
        state.projects.push(terminator_core::Project {
            id: "p".into(),
            name: "One".into(),
            path: "/tmp".into(),
            layout: serde_json::json!({}),
        });
        state.sessions[0].label = "renamed".into();
        cache.reconcile(&state, 50, Some(true));
        assert!(!std::sync::Arc::ptr_eq(
            &kept,
            &cache.get(&state, "s0", 50, Some(true))
        ));
        assert!(std::sync::Arc::ptr_eq(
            &stable,
            &cache.get(&state, "s1", 50, Some(true))
        ));
        let other = cache.get(&state, "s1", 50, Some(true));
        state.sessions.retain(|item| item.id != "s0");
        cache.reconcile(&state, 50, Some(true));
        assert_eq!(
            cache.get(&state, "s0", 50, Some(true)).diagnostics(50),
            "Unknown session"
        );
        assert!(std::sync::Arc::ptr_eq(
            &other,
            &cache.get(&state, "s1", 50, Some(true))
        ));
        assert_eq!(cache.notifications_visited, (notices + 1) as u64 * 3);
    }

    #[test]
    fn one_session_change_leaves_unrelated_arcs_in_place() {
        let mut state = State::default();
        state.sessions.push(session("a"));
        state.sessions.push(session("b"));
        capable(&mut state);
        state.presence.push(agents::TerminalPresence {
            session_id: "a".into(),
            generation: "g".into(),
            agents: vec![],
            outcome: PresenceOutcome::Verified,
            observed_at: 1,
        });
        state
            .notifications
            .push(notice("na", "a", AgentState::Failed, 1));
        state
            .notifications
            .push(notice("nb", "b", AgentState::WaitingInput, 2));
        let mut cache = PresentationCache::default();
        cache.reconcile(&state, 10, Some(true));
        let left = cache.get(&state, "a", 10, Some(true));
        let right = cache.get(&state, "b", 10, Some(true));
        state.presence[0].observed_at = 9;
        state.notifications[0].read = true;
        cache.reconcile(&state, 10, Some(true));
        let next = cache.get(&state, "a", 10, Some(true));
        assert!(!std::sync::Arc::ptr_eq(&left, &next));
        assert!(next.verified);
        assert_eq!(next.unread, 0);
        assert_eq!(next.attention.failed, 1);
        assert!(std::sync::Arc::ptr_eq(
            &right,
            &cache.get(&state, "b", 10, Some(true))
        ));
        assert_eq!(cache.presentations_rebuilt, 3);
        assert_eq!(cache.presentations_reused, 1);
    }

    #[test]
    fn changed_generation_drops_stale_verification() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        state
            .generations
            .push(terminator_core::generations::Health {
                owner: terminator_core::generations::Generation {
                    id: "g".into(),
                    data: "/tmp".into(),
                    runtime: "/tmp".into(),
                    version: "0".into(),
                    build: "0".into(),
                    protocol: 1,
                    catalog: 1,
                    status: terminator_core::generations::Status::Active,
                    pid: None,
                },
                revision: 1,
                live_sessions: 1,
                error: None,
                capabilities: vec![AGENT_PRESENCE_CAPABILITY.into()],
                helper: None,
            });
        observe(&mut state, &["codex"], 10);
        let mut cache = PresentationCache::default();
        cache.reconcile(&state, 10, Some(true));
        let first = cache.get(&state, "s", 10, Some(true));
        assert!(first.verified);
        assert_eq!(
            owner_support(&state, &state.sessions[0]),
            OwnerSupport {
                supported: true,
                available: true,
            }
        );
        state.sessions[0].generation = "missing".into();
        cache.reconcile(&state, 10, Some(true));
        let next = cache.get(&state, "s", 10, Some(true));
        assert!(!std::sync::Arc::ptr_eq(&first, &next));
        assert!(!next.verified && !next.live);
        assert_eq!(
            owner_support(&state, &state.sessions[0]),
            OwnerSupport {
                supported: false,
                available: false,
            }
        );
    }

    #[test]
    fn fresh_connection_ignores_observation_age_until_the_timestamp_changes() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        capable(&mut state);
        observe(&mut state, &["codex"], 100);
        let mut cache = PresentationCache::default();
        let fresh = cache.get(&state, "s", 105, Some(true));
        assert!(fresh.verified);
        let later = cache.get(&state, "s", 10_000, Some(true));
        assert!(std::sync::Arc::ptr_eq(&fresh, &later));
        assert!(later.verified);
        assert_eq!(cache.indexes_built, 1);
    }

    #[test]
    fn legacy_expiry_changes_verification_at_the_deadline() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        capable(&mut state);
        observe(&mut state, &["codex"], 100);
        let mut cache = PresentationCache::default();
        let current = cache.get(&state, "s", 105, None);
        assert!(current.verified);
        assert!(std::sync::Arc::ptr_eq(
            &current,
            &cache.get(&state, "s", 105, None)
        ));
        assert_eq!(cache.indexes_built, 1);
        let expired = cache.get(&state, "s", 106, None);
        assert!(!expired.verified);
        assert_eq!(cache.indexes_built, 2);
    }

    #[test]
    fn due_snoozes_and_reads_update_in_one_pass() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        state.sessions.push(session("other"));
        let mut early = notice("early", "s", AgentState::WaitingInput, 1);
        early.snoozed_until = 20;
        let mut later = notice("later", "s", AgentState::WaitingPermission, 2);
        later.snoozed_until = 30;
        state.notifications.push(early);
        state.notifications.push(later);
        hook(&mut state, "codex", AgentState::Running, false);
        let mut cache = PresentationCache::default();
        cache.reconcile(&state, 10, Some(true));
        assert!(cache.get(&state, "s", 10, Some(true)).attention.is_empty());
        assert_eq!(
            cache
                .get(&state, "s", 10, Some(true))
                .notice_preview
                .as_deref(),
            Some("note later")
        );
        let other = cache.get(&state, "other", 10, Some(true));
        let built = cache.indexes_built;
        let at_deadline = cache.get(&state, "s", 30, Some(true));
        assert_eq!(cache.indexes_built, built + 1);
        assert_eq!(
            at_deadline.attention,
            AttentionCounts {
                input: 1,
                permission: 1,
                failed: 0,
            }
        );
        assert!(std::sync::Arc::ptr_eq(
            &other,
            &cache.get(&state, "other", 30, Some(true))
        ));
        state.notifications[0].read = true;
        state.notifications[1].dismissed = true;
        cache.reconcile(&state, 30, Some(true));
        let updated = cache.get(&state, "s", 30, Some(true));
        assert_eq!(updated.unread, 0);
        assert_eq!(updated.attention.input, 1);
        assert_eq!(updated.attention.permission, 0);
        assert_eq!(updated.notice_preview.as_deref(), Some("note early"));
    }

    #[test]
    fn empty_and_unknown_lookups_do_not_rebuild() {
        let state = State::default();
        let mut cache = PresentationCache::default();
        let missing = cache.get(&state, "missing", 10, Some(true));
        let again = cache.get(&state, "other", 10, Some(true));
        assert!(std::sync::Arc::ptr_eq(&missing, &again));
        assert_eq!(cache.indexes_built, 1);
        assert_eq!(cache.notifications_visited, 0);
        assert_eq!(missing.diagnostics(10), "Unknown session");
        let mut state = State::default();
        state.sessions.push(session("s"));
        cache.reconcile(&state, 10, Some(true));
        let _ = cache.get(&state, "nope", 10, Some(true));
        let _ = cache.get(&state, "nope", 11, Some(true));
        assert_eq!(cache.indexes_built, 2);
    }

    #[test]
    fn replaced_collections_refresh_a_warm_cache_on_lookup() {
        let mut cache = PresentationCache::default();
        let empty = State::default();
        cache.reconcile(&empty, 10, Some(true));
        assert_eq!(cache.indexes_built, 1);
        let mut state = State::default();
        state.sessions.push(session("s"));
        hook(&mut state, "codex", AgentState::Running, false);
        let presented = cache.get(&state, "s", 10, Some(true));
        assert_eq!(presented.brand_icon, Some("AgentCodex"));
        assert_eq!(presented.status_icon, "LoaderCircle");
        assert_eq!(cache.indexes_built, 2);
        let again = cache.get(&state, "s", 10, Some(true));
        assert!(std::sync::Arc::ptr_eq(&presented, &again));
        assert_eq!(cache.indexes_built, 2);
    }

    #[test]
    fn verified_empty_presence_returns_terminal_to_plain_shell() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        capable(&mut state);
        observe(&mut state, &[], now());
        hook(&mut state, "codex", AgentState::Running, false);
        let presented = present_session(&state, "s", now());
        assert!(presented.verified);
        assert!(!presented.live);
        assert_eq!(presented.brand_icon, None);
        assert_eq!(presented.brand_label, None);
        assert_eq!(presented.lifecycle, None);
        assert_eq!(presented.status_label, "Status unavailable");
        assert!(!presented.spin);
        assert!(
            presented
                .diagnostics(now())
                .contains("No live agent detected")
        );
        assert!(
            presented
                .diagnostics(now())
                .contains("Identity source: none")
        );
        assert!(unverified_sessions(&state, now()).is_empty());
        assert!(live_sessions(&state, now()).is_empty());
    }
}
