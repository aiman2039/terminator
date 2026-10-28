//! Background SFTP transfers for SSH projects. They outlive the IPC request
//! that starts them; clients poll `Operation::TransferStatus`.
use super::*;
use std::{collections::HashMap, sync::Arc};
use terminator_core::remote::TransferStatus;

/// One project's transfers may run at once.
const MAX_ACTIVE: usize = 2;
/// Finished statuses are kept for polling until this many are recorded.
const MAX_RECORDED: usize = 32;
const TIMEOUT: Duration = Duration::from_hours(1);

#[derive(Default)]
pub(crate) struct Transfers {
    records: HashMap<String, (String, TransferStatus)>,
}
impl Transfers {
    pub(crate) fn active(&self, project: &str) -> usize {
        self.records
            .values()
            .filter(|(p, s)| p == project && !s.done)
            .count()
    }
    fn insert(&mut self, project: &str, status: TransferStatus) -> Result<()> {
        ensure!(
            self.active(project) < MAX_ACTIVE,
            "Only two transfers may run per SSH project"
        );
        if self.records.len() >= MAX_RECORDED {
            self.records.retain(|_, (_, s)| !s.done);
        }
        self.records
            .insert(status.id.clone(), (project.to_owned(), status));
        Ok(())
    }
    fn update(&mut self, id: &str, change: impl FnOnce(&mut TransferStatus)) {
        if let Some((_, status)) = self.records.get_mut(id) {
            change(status);
        }
    }
    fn status(&self, project: &str, id: &str) -> Result<TransferStatus> {
        self.records
            .get(id)
            .filter(|(p, _)| p == project)
            .map(|(_, s)| s.clone())
            .context("Unknown transfer ID")
    }
}

struct Recorder<'a> {
    shared: &'a Shared,
    id: &'a str,
}
impl super::transfer::Progress for Recorder<'_> {
    fn size(&self, bytes: u64) {
        relock(&self.shared.remote_transfers).update(self.id, |s| s.total = Some(bytes));
    }
    fn advance(&self, bytes: u64) {
        relock(&self.shared.remote_transfers).update(self.id, |s| s.bytes = bytes);
    }
}

/// The new remote file an upload of `local` into the resolved directory
/// `directory` publishes.
fn destination(directory: &str, local: &Path) -> Result<String> {
    let name = local
        .file_name()
        .and_then(|n| n.to_str())
        .context("Upload source needs a UTF-8 file name")?;
    terminator_core::remote::validate_file_name(name)?;
    Ok(format!("{}/{name}", directory.trim_end_matches('/')))
}

/// Checks a requested local download destination before any network work.
fn check_local_download(local: &Path) -> Result<()> {
    ensure!(local.is_absolute(), "Local download path must be absolute");
    ensure!(
        local.symlink_metadata().is_err(),
        "Local destination already exists; it was not overwritten"
    );
    ensure!(
        local.parent().is_some_and(Path::is_dir),
        "Local destination folder does not exist"
    );
    Ok(())
}

struct RunInput<'a> {
    project: &'a RemoteProject,
    control: Option<&'a Path>,
    id: &'a str,
    operation: &'a Operation,
    progress: &'a dyn super::transfer::Progress,
}
async fn run(input: RunInput<'_>) -> Result<()> {
    let RunInput {
        project,
        control,
        id,
        operation,
        progress,
    } = input;
    super::transfer::with_sftp(&project.ssh, control, async |sftp| {
        let mut fs = sftp.fs();
        match operation {
            Operation::Upload { local, remote_dir } => {
                let directory = super::files::contained(&mut fs, project, remote_dir).await?;
                ensure!(
                    fs.metadata(&directory)
                        .await?
                        .file_type()
                        .is_some_and(|t| t.is_dir()),
                    "Upload target is not a directory"
                );
                drop(fs);
                let directory = directory
                    .to_str()
                    .context("Remote path must be UTF-8")?
                    .to_owned();
                let remote = destination(&directory, local)?;
                super::transfer::upload(super::transfer::UploadInput {
                    sftp,
                    local,
                    remote: &remote,
                    id,
                    progress,
                })
                .await?;
            }
            Operation::Download { path, local } => {
                let actual = super::files::contained(&mut fs, project, path).await?;
                drop(fs);
                let actual = actual.to_str().context("Remote path must be UTF-8")?;
                super::transfer::download(super::transfer::DownloadInput {
                    sftp,
                    remote: actual,
                    local,
                    progress,
                })
                .await?;
            }
            _ => anyhow::bail!("Not a transfer operation"),
        }
        Ok(())
    })
    .await
}

impl Shared {
    /// Validates and records the transfer, then runs it on its own thread.
    pub(crate) fn start_remote_transfer(
        self: &Arc<Self>,
        project: RemoteProject,
        operation: Operation,
    ) -> Result<Reply> {
        project.validate()?;
        let (upload, name, remote_dir) = Self::validate_transfer_operation(&operation)?;
        let status = TransferStatus {
            id: terminator_core::id(),
            upload,
            name,
            remote_dir,
            done: false,
            bytes: 0,
            total: None,
            error: None,
            ssh: None,
        };
        super::transfer::validate_id(&status.id)?;
        relock(&self.remote_transfers).insert(&project.id, status.clone())?;
        let control = self.ssh_control(&project.ssh);
        let shared = Arc::clone(self);
        let id = status.id.clone();
        let spawned = std::thread::Builder::new()
            .name("ssh-transfer".into())
            .spawn(move || {
                let result = (|| -> Result<()> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    runtime.block_on(async {
                        tokio::time::timeout(
                            TIMEOUT,
                            run(RunInput {
                                project: &project,
                                control: control.as_deref(),
                                id: &id,
                                operation: &operation,
                                progress: &Recorder {
                                    shared: &shared,
                                    id: &id,
                                },
                            }),
                        )
                        .await
                        .context("SFTP transfer timed out; publication state is unknown; check the destination before retrying")?
                    })
                })();
                relock(&shared.remote_transfers).update(&id, |status| {
                    status.done = true;
                    if let Err(error) = result {
                        status.ssh = terminator_core::ssh::SshError::find(error.as_ref())
                            .cloned();
                        status.error = Some(format!("{error:#}"));
                    }
                });
            });
        if let Err(error) = spawned {
            relock(&self.remote_transfers).update(&status.id, |s| {
                s.done = true;
                s.error = Some(error.to_string());
            });
            return Err(error).context("Cannot start the transfer");
        }
        Ok(Reply::Transfer(status))
    }
    fn validate_transfer_operation(operation: &Operation) -> Result<(bool, String, String)> {
        let (upload, name, remote_dir) = match operation {
            Operation::Upload { local, remote_dir } => {
                ensure!(local.is_absolute(), "Local upload path must be absolute");
                terminator_core::remote::validate_relative(remote_dir)?;
                let name = local
                    .file_name()
                    .and_then(|n| n.to_str())
                    .context("Upload source needs a UTF-8 file name")?;
                terminator_core::remote::validate_file_name(name)?;
                (true, name.to_owned(), remote_dir.clone())
            }
            Operation::Download { path, local } => {
                terminator_core::remote::validate_relative(path)?;
                ensure!(!path.is_empty(), "Choose a remote file to download");
                check_local_download(local)?;
                let name = path.rsplit('/').next().unwrap_or(path).to_owned();
                (false, name, String::new())
            }
            _ => anyhow::bail!("Not a transfer operation"),
        };
        Ok((upload, name, remote_dir))
    }
    pub(crate) fn remote_transfer_status(&self, project: &str, id: &str) -> Result<Reply> {
        Ok(Reply::Transfer(
            relock(&self.remote_transfers).status(project, id)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status(id: &str) -> TransferStatus {
        TransferStatus {
            id: id.into(),
            upload: true,
            name: "file".into(),
            remote_dir: String::new(),
            done: false,
            bytes: 0,
            total: None,
            error: None,
            ssh: None,
        }
    }
    #[test]
    fn uploads_publish_beside_the_resolved_directory_with_the_source_name() {
        assert_eq!(
            destination("/srv/app/src", Path::new("/tmp/notes.txt")).unwrap(),
            "/srv/app/src/notes.txt"
        );
        assert_eq!(
            destination("/", Path::new("/tmp/notes.txt")).unwrap(),
            "/notes.txt"
        );
        assert!(destination("/srv", Path::new("/")).is_err());
    }
    #[test]
    fn downloads_refuse_an_existing_or_relative_local_destination() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("existing");
        std::fs::write(&existing, b"keep").unwrap();
        assert!(check_local_download(&existing).is_err());
        assert_eq!(std::fs::read(&existing).unwrap(), b"keep");
        assert!(check_local_download(Path::new("relative.txt")).is_err());
        assert!(check_local_download(&dir.path().join("missing/new.txt")).is_err());
        assert!(check_local_download(&dir.path().join("new.txt")).is_ok());
        #[cfg(unix)]
        {
            let link = dir.path().join("dangling");
            std::os::unix::fs::symlink(dir.path().join("nowhere"), &link).unwrap();
            assert!(check_local_download(&link).is_err());
        }
    }
    #[test]
    fn each_project_runs_at_most_two_transfers_and_statuses_stay_private() {
        let mut transfers = Transfers::default();
        transfers.insert("a", status("1")).unwrap();
        transfers.insert("a", status("2")).unwrap();
        assert!(transfers.insert("a", status("3")).is_err());
        transfers.insert("b", status("4")).unwrap();
        transfers.update("1", |s| s.done = true);
        transfers.insert("a", status("5")).unwrap();
        assert!(transfers.status("b", "1").is_err());
        assert!(transfers.status("a", "1").unwrap().done);
    }
}
