use super::*;
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use futures_util::StreamExt;
use openssh_sftp_client::{Sftp, SftpOptions, file::TokioCompatFile, fs::Fs};
use std::process::Stdio;
use terminator_core::remote::{Entry, MAX_ENTRIES, MAX_FILE_BYTES};
use tokio::io::AsyncReadExt;

/// Resolves the project path `path` on the server, following symlinks, and
/// refuses it unless it stays inside the resolved project root.
pub(super) async fn contained(
    fs: &mut Fs,
    project: &RemoteProject,
    path: &str,
) -> Result<std::path::PathBuf> {
    let root = fs.canonicalize(&project.root).await?;
    let absolute = project.absolute(path)?;
    let actual = fs.canonicalize(absolute).await?;
    let root_text = root.to_str().context("Remote root must be UTF-8")?;
    let actual_text = actual.to_str().context("Remote path must be UTF-8")?;
    ensure!(
        within(root_text, actual_text),
        "Remote path resolves outside the project root"
    );
    Ok(actual)
}
/// Whether the resolved remote path `actual` is `root` or below it.
fn within(root: &str, actual: &str) -> bool {
    let root = root.trim_end_matches('/');
    actual == root || actual.starts_with(&format!("{root}/"))
}
pub(super) async fn read(
    sftp: &Sftp,
    fs: &mut Fs,
    project: &RemoteProject,
    path: &str,
) -> Result<Vec<u8>> {
    let actual = contained(fs, project, path).await?;
    let mut file = sftp.open(actual).await?;
    let metadata = file.metadata().await?;
    ensure!(
        metadata.file_type().is_some_and(|t| t.is_file())
            && metadata.len().is_some_and(|n| n <= MAX_FILE_BYTES as u64),
        "Remote preview requires a regular file no larger than 2 MiB"
    );
    let mut reader = Box::pin(TokioCompatFile::from(file));
    let mut bytes = Vec::new();
    reader
        .as_mut()
        .take((MAX_FILE_BYTES as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .await?;
    ensure!(
        bytes.len() <= MAX_FILE_BYTES,
        "Remote file changed beyond the preview limit"
    );
    Ok(bytes)
}
pub(super) fn operation(
    project: &RemoteProject,
    control: Option<&Path>,
    operation: Operation,
) -> Result<Reply> {
    project.validate()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(15), async {
            let mut command = tokio::process::Command::new("ssh");
            command
                .args(terminator_core::ssh::arguments(
                    &project.ssh,
                    false,
                    control,
                )?)
                .arg("-s")
                .arg("--")
                .arg(&project.ssh.host)
                .arg("sftp")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            for key in terminator_core::ssh::private_environment_keys() {
                command.env_remove(key);
            }
            let mut child = command.spawn().context("Cannot start SSH SFTP service")?;
            let stderr = super::capture_stderr(&mut child);
            let stdin = child.stdin.take().context("SFTP stdin unavailable")?;
            let stdout = child.stdout.take().context("SFTP stdout unavailable")?;
            let sftp = match Sftp::new(stdin, stdout, SftpOptions::default()).await {
                Ok(sftp) => sftp,
                Err(error) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(super::sftp_failure(error.into(), stderr).await);
                }
            };
            let mut fs = sftp.fs();
            let result = async {
                match operation {
                    Operation::List { path } => {
                        let actual = contained(&mut fs, project, &path).await?;
                        let directory = fs.open_dir(actual).await?;
                        let mut stream = Box::pin(directory.read_dir());
                        let mut entries = Vec::new();
                        while let Some(entry) = stream.next().await {
                            let entry = entry?;
                            let name = entry
                                .filename()
                                .to_str()
                                .context("Remote file names must be UTF-8")?
                                .to_owned();
                            if name == "." || name == ".." {
                                continue;
                            }
                            ensure!(
                                !name.contains('/'),
                                "Remote server returned a non-child directory entry"
                            );
                            let child = if path.is_empty() {
                                name.clone()
                            } else {
                                format!("{path}/{name}")
                            };
                            terminator_core::remote::validate_relative(&child)?;
                            ensure!(
                                entries.len() < MAX_ENTRIES,
                                "Remote directory exceeds 1000 entries"
                            );
                            entries.push(Entry {
                                name,
                                path: child,
                                directory: entry.file_type().is_some_and(|t| t.is_dir()),
                                symlink: entry.file_type().is_some_and(|t| t.is_symlink()),
                                bytes: entry.metadata().len(),
                            });
                        }
                        entries.sort_by(|a, b| {
                            b.directory.cmp(&a.directory).then(a.name.cmp(&b.name))
                        });
                        Ok(Reply::Directory { path, entries })
                    }
                    Operation::Read { path } => {
                        let bytes = read(&sftp, &mut fs, project, &path).await?;
                        Ok(Reply::File {
                            path,
                            base64: B64.encode(bytes),
                        })
                    }
                    _ => anyhow::bail!("Not a remote filesystem operation"),
                }
            }
            .await;
            drop(fs);
            let close = sftp.close().await;
            let _ = child.start_kill();
            let _ = child.wait().await;
            match (result, close) {
                (Ok(result), Ok(())) => Ok::<_, anyhow::Error>(result),
                (Err(error), _) => Err(super::sftp_failure(error, stderr).await),
                (Ok(_), Err(error)) => Err(super::sftp_failure(error.into(), stderr).await),
            }
        })
        .await
        .context("SSH file operation timed out")?
    })
}

#[cfg(test)]
mod tests {
    use super::within;
    #[test]
    fn resolved_paths_must_stay_under_the_project_root() {
        assert!(within("/srv/app", "/srv/app"));
        assert!(within("/srv/app/", "/srv/app/src/main.rs"));
        assert!(!within("/srv/app", "/srv/application/secret"));
        assert!(!within("/srv/app", "/srv"));
        assert!(!within("/srv/app", "/etc/passwd"));
        assert!(within("/", "/etc/passwd"));
    }
}
