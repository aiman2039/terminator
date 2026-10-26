//! SFTP is an explicit user operation. Completed destinations are never overwritten.
use super::*;
use anyhow::{Context, Result, ensure};
use openssh_sftp_client::{Sftp, SftpOptions, file::TokioCompatFile};
use std::process::Stdio;
use terminator_core::vnc::{
    MAX_TRANSFER_BYTES, TransferDirection, Transport, VncTransfer, VncTransferStatus,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

fn validate(transfer: &VncTransfer) -> Result<()> {
    ensure!(
        !transfer.id.is_empty()
            && transfer.id.len() <= 128
            && transfer
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)),
        "Invalid transfer ID"
    );
    ensure!(
        transfer.local.is_absolute(),
        "Local transfer path must be absolute"
    );
    // SFTP paths use POSIX separators, independent of the client OS.
    ensure!(
        transfer.remote.starts_with('/')
            && transfer.remote.len() <= 4096
            && !transfer.remote.chars().any(char::is_control)
            && !transfer.remote.split('/').any(|part| part == ".."),
        "Remote transfer path must be absolute and contain no parent traversal"
    );
    Ok(())
}
impl Shared {
    pub(crate) fn vnc_transfer(self: &Arc<Self>, sid: &str, transfer: VncTransfer) -> Result<()> {
        validate(&transfer)?;
        let runtime = relock(&self.vnc_sessions)
            .get(sid)
            .cloned()
            .context("VNC session is not live")?;
        let options = {
            let mut rt = relock(&runtime);
            ensure!(
                rt.status == VncSessionStatus::Connected && !rt.view_only,
                "File transfer requires an active writable VNC session"
            );
            ensure!(
                rt.options.transport == Transport::Ssh,
                "SFTP requires a configured SSH connection"
            );
            ensure!(
                !rt.transfers.contains_key(&transfer.id),
                "Transfer ID is already in use"
            );
            ensure!(
                rt.transfers.values().filter(|s| !s.done).count() < 2,
                "Only two transfers may run per VNC session"
            );
            if rt.transfers.len() >= 16 {
                rt.transfers.retain(|_, status| !status.done);
            }
            rt.transfers.insert(
                transfer.id.clone(),
                VncTransferStatus {
                    id: transfer.id.clone(),
                    done: false,
                    bytes: 0,
                    error: None,
                },
            );
            rt.options.clone()
        };
        thread::spawn(move || {
            let result = (|| -> Result<()> {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                rt.block_on(async {
                    tokio::time::timeout(
                        Duration::from_mins(5),
                        run(&runtime, &options.ssh, &transfer),
                    )
                    .await
                    .context("SFTP transfer timed out; publication state is unknown; check the destination before retrying")?
                })
            })();
            if let Some(status) = relock(&runtime).transfers.get_mut(&transfer.id) {
                status.done = true;
                status.error = result.err().map(|e| format!("{e:#}"));
            }
        });
        Ok(())
    }
    pub(crate) fn vnc_transfer_status(&self, sid: &str, id: &str) -> Result<VncTransferStatus> {
        let runtime = relock(&self.vnc_sessions)
            .get(sid)
            .cloned()
            .context("VNC session is not live")?;

        relock(&runtime)
            .transfers
            .get(id)
            .cloned()
            .context("Unknown transfer ID")
    }
}
async fn copy<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut input: R,
    mut output: W,
    runtime: &Arc<Mutex<VncRuntime>>,
    id: &str,
) -> Result<u64> {
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        ensure!(
            relock(runtime).status == VncSessionStatus::Connected,
            "VNC session closed; transfer stopped"
        );
        let read = input.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(read)?)
            .context("Transfer size overflow")?;
        ensure!(total <= MAX_TRANSFER_BYTES, "Transfer exceeds 1 GiB");
        output
            .write_all(buffer.get(..read).context("Invalid transfer chunk")?)
            .await?;
        if let Some(status) = relock(runtime).transfers.get_mut(id) {
            status.bytes = total;
        }
    }
    output.flush().await?;
    Ok(total)
}
async fn run(
    runtime: &Arc<Mutex<VncRuntime>>,
    ssh: &terminator_core::vnc::SshOptions,
    transfer: &VncTransfer,
) -> Result<()> {
    let mut command = super::transport::ssh_command(ssh)?;
    command
        .arg("-s")
        .arg("--")
        .arg(&ssh.host)
        .arg("sftp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().context("Cannot start SSH SFTP subsystem")?;
    let input = child.stdin.take().context("SFTP stdin unavailable")?;
    let output = child.stdout.take().context("SFTP stdout unavailable")?;
    let result=async {
        let sftp=Sftp::new(input,output,SftpOptions::default()).await.context("SFTP negotiation failed; verify SSH host key and authentication")?;
        let mut fs=sftp.fs();
        match transfer.direction {
            TransferDirection::Upload=>{
                ensure!(sftp.support_hardlink()&&sftp.support_fsync(),"Server must support hard-link publication and fsync for strict SFTP uploads");
                let source=tokio::fs::File::open(&transfer.local).await?;
                let metadata=source.metadata().await?;
                ensure!(metadata.is_file()&&metadata.len()<=MAX_TRANSFER_BYTES,"Upload source must be a regular file no larger than 1 GiB");
                let temporary=format!("{}.terminator-{}.partial",transfer.remote,transfer.id);
                let remote=sftp.options().write(true).create_new(true).open(&temporary).await?;
                fs.set_permissions(&temporary,openssh_sftp_client::metadata::Permissions::from(0o600)).await?;
                let mut remote=Box::pin(TokioCompatFile::from(remote));
                let result=async {
                    let bytes=copy(source,remote.as_mut(),runtime,&transfer.id).await?;
                    ensure!(bytes==metadata.len(),"Upload source changed during transfer");
                    remote.as_mut().shutdown().await?;
                    remote.as_mut().as_mut_file().sync_all().await?;
                    drop(remote);
                    fs.hard_link(&temporary,&transfer.remote).await.context("Remote destination already exists or cannot be published; it was not overwritten")?;
                    Ok::<(),anyhow::Error>(())
                }.await;
                let _=fs.remove_file(&temporary).await;
                result?;
            }
            TransferDirection::Download=>{
                let mut remote=sftp.open(&transfer.remote).await?;
                let metadata=remote.metadata().await?;
                ensure!(metadata.file_type().is_some_and(|t|t.is_file())&&metadata.len().is_some_and(|n|n<=MAX_TRANSFER_BYTES),"Download source must be a regular file no larger than 1 GiB");
                let parent=transfer.local.parent().context("Download destination has no parent")?;
                let temporary=tempfile::NamedTempFile::new_in(parent)?;
                let mut output=tokio::fs::File::from_std(temporary.reopen()?);
                let bytes=copy(Box::pin(TokioCompatFile::from(remote)),&mut output,runtime,&transfer.id).await?;
                ensure!(Some(bytes)==metadata.len(),"Remote file changed during download");
                output.sync_all().await?;drop(output);
                temporary.persist_noclobber(&transfer.local).map_err(|e|e.error).context("Local destination already exists or cannot be published; it was not overwritten")?;
            }
        }
        // Sftp::close waits for every SftpHandle. Fs owns one, so release it first.
        drop(fs);
        sftp.close().await?;Ok::<(),anyhow::Error>(())
    }.await;
    let _ = child.start_kill();
    let _ = child.wait().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transfers_require_explicit_absolute_paths_and_safe_identity() {
        let mut transfer = VncTransfer {
            id: "fixture".into(),
            direction: TransferDirection::Upload,
            local: std::env::temp_dir().join("source"),
            remote: "/tmp/destination".into(),
        };
        assert!(validate(&transfer).is_ok());
        for path in [
            "relative",
            "/tmp/../escape",
            "/tmp/name\nother",
            "C:\\tmp\\destination",
            "\\\\server\\share\\destination",
        ] {
            transfer.remote = path.into();
            assert!(validate(&transfer).is_err());
        }
        transfer.remote = "/tmp/destination".into();
        transfer.local = "relative".into();
        assert!(validate(&transfer).is_err());
        transfer.local = std::env::temp_dir().join("source");
        transfer.id = "../../other".into();
        assert!(validate(&transfer).is_err());
    }
}
