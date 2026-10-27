//! SFTP is an explicit user operation. Completed destinations are never overwritten.
use super::*;
use crate::remote::transfer::{self, Progress};
use anyhow::{Context, Result, ensure};
use terminator_core::vnc::{TransferDirection, Transport, VncTransfer, VncTransferStatus};

fn validate(transfer: &VncTransfer) -> Result<()> {
    transfer::validate_id(&transfer.id)?;
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
        let control = self.ssh_control(&options.ssh);
        thread::spawn(move || {
            let result = (|| -> Result<()> {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                rt.block_on(async {
                    tokio::time::timeout(
                        Duration::from_mins(5),
                        run(&runtime, &options.ssh, control.as_deref(), &transfer),
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
/// Stops the copy when the VNC session leaves `Connected`.
struct Session<'a> {
    runtime: &'a Arc<Mutex<VncRuntime>>,
    id: &'a str,
}
impl Progress for Session<'_> {
    fn check(&self) -> Result<()> {
        ensure!(
            relock(self.runtime).status == VncSessionStatus::Connected,
            "VNC session closed; transfer stopped"
        );
        Ok(())
    }
    fn advance(&self, bytes: u64) {
        if let Some(status) = relock(self.runtime).transfers.get_mut(self.id) {
            status.bytes = bytes;
        }
    }
}
async fn run(
    runtime: &Arc<Mutex<VncRuntime>>,
    ssh: &terminator_core::vnc::SshOptions,
    control: Option<&std::path::Path>,
    transfer: &VncTransfer,
) -> Result<()> {
    let progress = Session {
        runtime,
        id: &transfer.id,
    };
    transfer::with_sftp(ssh, control, async |sftp| {
        match transfer.direction {
            TransferDirection::Upload => {
                transfer::upload(transfer::UploadInput {
                    sftp,
                    local: &transfer.local,
                    remote: &transfer.remote,
                    id: &transfer.id,
                    progress: &progress,
                })
                .await?;
            }
            TransferDirection::Download => {
                transfer::download(sftp, &transfer.remote, &transfer.local, &progress).await?;
            }
        }
        Ok(())
    })
    .await
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
