use super::submit::Services;
#[cfg(unix)]
use crate::player;
use crate::{Job, Update};
use eframe::egui;
use std::path::PathBuf;
#[cfg(unix)]
use std::time::{Duration, Instant};
use terminator_core::Paths;
#[cfg(unix)]
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_checks_reject_files_missing_parents_and_broken_links() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, "fixture").unwrap();
        let paths = vec![
            ("directory".into(), dir.path().to_owned()),
            ("missing".into(), dir.path().join("deleted")),
            ("file".into(), file.clone()),
            ("parent-is-file".into(), file.join("child")),
        ];
        let result = crate::gui_services::project_directories(paths);
        assert_eq!(
            result
                .iter()
                .map(|(_, _, available)| *available)
                .collect::<Vec<_>>(),
            vec![true, false, false, false]
        );
        #[cfg(unix)]
        {
            let link = dir.path().join("alias");
            std::os::unix::fs::symlink(dir.path().join("target"), &link).unwrap();
            let check =
                || crate::gui_services::project_directories(vec![("alias".into(), link.clone())]);
            assert!(!check()[0].2);
            std::fs::create_dir(dir.path().join("target")).unwrap();
            assert!(check()[0].2);
        }
    }

    #[cfg(unix)]
    use crate::nvim_rpc;
    use crate::preferences::UiPreferences;
    use std::sync::mpsc;
    #[cfg(unix)]
    use terminator_core::CommandOptions;
    #[cfg(unix)]
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    /// Accepts one peer, waits for its first request, signals `started`,
    /// then never replies until `token` is cancelled.
    #[cfg(unix)]
    async fn hold_after_first_read<S: tokio::io::AsyncRead + Unpin>(
        mut peer: S,
        started: tokio::sync::oneshot::Sender<()>,
        token: CancellationToken,
    ) {
        let mut bytes = [0; 4096];
        assert!(peer.read(&mut bytes).await.unwrap() > 0);
        let _ = started.send(());
        token.cancelled().await;
    }

    #[cfg(unix)]
    fn spawn_stalled_http(
        http: tokio::net::TcpListener,
        token: CancellationToken,
    ) -> (
        tokio::task::JoinHandle<()>,
        tokio::sync::oneshot::Receiver<()>,
    ) {
        let (started, ready) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (peer, _) = http.accept().await.unwrap();
            hold_after_first_read(peer, started, token).await;
        });
        (server, ready)
    }

    #[cfg(unix)]
    fn spawn_stalled_nvim(
        listener: tokio::net::UnixListener,
        token: CancellationToken,
    ) -> (
        tokio::task::JoinHandle<()>,
        tokio::sync::oneshot::Receiver<()>,
    ) {
        let (started, ready) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (peer, _) = listener.accept().await.unwrap();
            hold_after_first_read(peer, started, token).await;
        });
        (server, ready)
    }

    #[cfg(unix)]
    fn spawn_good_nvim(listener: tokio::net::UnixListener) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let (mut peer, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            assert!(peer.read(&mut bytes).await.unwrap() > 0);
            peer.write_all(
                &rmp_serde::to_vec(&(
                    1,
                    1,
                    serde_json::Value::Null,
                    serde_json::json!({"blocking":false}),
                ))
                .unwrap(),
            )
            .await
            .unwrap();
        })
    }

    #[cfg(unix)]
    struct SubmitFixtureInput<
        'a,
        F: std::future::Future<Output = anyhow::Result<Vec<Update>>> + Send + 'static,
    > {
        service: &'a Services,
        resource: &'a str,
        token: CancellationToken,
        work: F,
    }

    #[cfg(unix)]
    fn submit_fixture(
        input: SubmitFixtureInput<
            '_,
            impl std::future::Future<Output = anyhow::Result<Vec<Update>>> + Send + 'static,
        >,
    ) {
        let SubmitFixtureInput {
            service,
            resource,
            token,
            work,
        } = input;
        service
            .handle()
            .submit(
                OperationContext::new("fixture", resource.into(), Policy::ReplaceableRead),
                token,
                work,
            )
            .unwrap();
    }

    #[cfg(unix)]
    fn submit_stalled_git(service: &Services, token: CancellationToken) {
        let processes = service.processes().clone();
        submit_fixture(SubmitFixtureInput {
            service,
            resource: "git",
            token,
            work: async move {
                let mut command = std::process::Command::new("sh");
                command.args(["-c", "sleep 10"]);
                processes
                    .run(
                        command,
                        CommandOptions::default(),
                        Some("stalled-repo".into()),
                    )
                    .await?;
                Ok(Vec::new())
            },
        });
    }

    #[cfg(unix)]
    async fn wait_for_other_editor(owner: &mut crate::gui_services::Owner) {
        let deadline = Instant::now().checked_add(Duration::from_secs(1)).unwrap();
        loop {
            assert!(
                Instant::now() < deadline,
                "Unrelated editor stalled behind radio/Git/Neovim"
            );
            if let Some(mut completion) = owner.supervisor.try_recv()
                && let Some(Ok(updates)) = completion.result.take()
                && updates
                    .iter()
                    .any(|u| matches!(u, Update::Info(s) if s == "other editor progressed"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    #[cfg(unix)]
    async fn wait_until_reaped(service: &Services, owner: &mut crate::gui_services::Owner) {
        let deadline = Instant::now().checked_add(Duration::from_secs(2)).unwrap();
        loop {
            while owner.supervisor.try_recv().is_some() {}
            if service.processes().children() == 0 && service.handle().diagnostics().active == 0 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Cancelled fixture work was not reaped"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn stalled_radio_git_and_neovim_do_not_block_another_editor() {
        let directory = tempfile::Builder::new()
            .prefix("async-load-")
            .tempdir_in("/tmp")
            .unwrap();
        let (updates, _) = mpsc::channel();
        let (service, mut owner) = Services::new(
            Paths::at(directory.path().into()),
            egui::Context::default(),
            updates,
        )
        .unwrap();
        let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", http.local_addr().unwrap());
        let slow_path = directory.path().join("slow.nvim");
        let good_path = directory.path().join("good.nvim");
        let slow = tokio::net::UnixListener::bind(&slow_path).unwrap();
        let good = tokio::net::UnixListener::bind(&good_path).unwrap();
        let stop = CancellationToken::new();
        let (http_server, http_ready) = spawn_stalled_http(http, stop.clone());
        let (slow_server, nvim_ready) = spawn_stalled_nvim(slow, stop.clone());
        let good_server = spawn_good_nvim(good);
        let radio = CancellationToken::new();
        submit_fixture(SubmitFixtureInput {
            service: &service,
            resource: "radio",
            token: radio.clone(),
            work: async move {
                player::fixture_download(url).await?;
                Ok(Vec::new())
            },
        });
        let nvim = CancellationToken::new();
        let cpu = service.cpu().clone();
        submit_fixture(SubmitFixtureInput {
            service: &service,
            resource: "nvim",
            token: nvim.clone(),
            work: async move {
                let mut connection =
                    nvim_rpc::AsyncConnection::connect(&slow_path, Duration::from_secs(3), cpu)
                        .await?;
                connection
                    .call("nvim_get_mode", serde_json::json!([]), 4096)
                    .await?;
                Ok(Vec::new())
            },
        });
        let git = CancellationToken::new();
        submit_stalled_git(&service, git.clone());
        http_ready.await.unwrap();
        nvim_ready.await.unwrap();
        let cpu = service.cpu().clone();
        submit_fixture(SubmitFixtureInput {
            service: &service,
            resource: "other-editor",
            token: CancellationToken::new(),
            work: async move {
                let mut connection =
                    nvim_rpc::AsyncConnection::connect(&good_path, Duration::from_millis(400), cpu)
                        .await?;
                let value = connection
                    .call("nvim_get_mode", serde_json::json!([]), 4096)
                    .await?;
                anyhow::ensure!(
                    value["blocking"] == false,
                    "other editor returned invalid state"
                );
                Ok(vec![Update::Info("other editor progressed".into())])
            },
        });
        wait_for_other_editor(&mut owner).await;
        radio.cancel();
        nvim.cancel();
        git.cancel();
        stop.cancel();
        http_server.await.unwrap();
        slow_server.await.unwrap();
        good_server.await.unwrap();
        wait_until_reaped(&service, &mut owner).await;
    }

    #[test]
    fn replaceable_read_admission_failure_does_not_emit_status_error() {
        let directory = tempfile::Builder::new()
            .prefix("busy-banner-")
            .tempdir_in(if cfg!(unix) {
                std::path::PathBuf::from("/tmp")
            } else {
                std::env::temp_dir()
            })
            .unwrap();
        let (updates, rx) = mpsc::channel();
        let (service, _owner) = Services::new(
            Paths::at(directory.path().into()),
            egui::Context::default(),
            updates,
        )
        .unwrap();
        service.handle().close_admission();
        while rx.try_recv().is_ok() {}
        assert!(
            service
                .send(Job::ResolveTarget(
                    "hover".into(),
                    "lib.rs".into(),
                    PathBuf::from("/tmp"),
                ))
                .is_err()
        );
        let received: Vec<_> = rx.try_iter().collect();
        assert!(
            received.iter().any(
                |update| matches!(update, Update::ResolvedTarget(key, None) if key == "hover")
            ),
            "rejected hover still resolves empty"
        );
        assert!(
            received
                .iter()
                .all(|update| !matches!(update, Update::Error(_))),
            "hover admission failure must not use the status banner"
        );
        assert!(
            service
                .send(Job::Preferences(UiPreferences::default()))
                .is_err()
        );
        assert!(
            rx.try_iter().any(
                |update| matches!(update, Update::Error(message) if message == "Services are closing")
            ),
            "mutations still report admission failure"
        );
    }

    #[test]
    fn rejected_service_start_reports_without_status_error() {
        let directory = tempfile::Builder::new()
            .prefix("busy-service-start-")
            .tempdir_in(if cfg!(unix) {
                std::path::PathBuf::from("/tmp")
            } else {
                std::env::temp_dir()
            })
            .unwrap();
        let (updates, rx) = mpsc::channel();
        let (service, _owner) = Services::new(
            Paths::at(directory.path().into()),
            egui::Context::default(),
            updates,
        )
        .unwrap();
        service.handle().close_admission();
        while rx.try_recv().is_ok() {}
        assert!(service.send(Job::StartSessionService).is_err());
        let received: Vec<_> = rx.try_iter().collect();
        assert!(
            received
                .iter()
                .any(|update| matches!(update, Update::ServiceStarted(Err(_)))),
            "rejected service start still reports completion"
        );
    }
}
