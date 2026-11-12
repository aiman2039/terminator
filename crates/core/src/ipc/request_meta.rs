//! Per-request client deadlines and diagnostic names.
use std::time::Duration;

use super::Request;

impl Request {
    /// How long a client waits for this request's response. Requests whose
    /// daemon work runs Git or SSH get longer than the 3 s default: a client
    /// that gives up early reports failure for work that then succeeds.
    #[must_use]
    pub fn response_deadline(&self) -> Duration {
        Duration::from_secs(match self {
            // Above the longest SSH command timeout the daemon allows.
            Self::RemoteProject { .. } | Self::ProbeRemoteProject { .. } => 130,
            Self::ScanHostKey { .. }
            | Self::TrustHostKey { .. }
            | Self::StartForward { .. }
            | Self::StopForward { .. }
            | Self::ForwardStatus
            | Self::WorktreeAdd { .. }
            | Self::WorktreeRemove { .. } => 40,
            // Connect runs the host's first SSH sign-in.
            Self::RemoteAppsConnect { .. } => 40,
            Self::VncFrame { .. }
            | Self::CloseIdleSessions { .. }
            | Self::LocalHistoryRevert { .. } => 30,
            // Writing the OS keychain can wait on a macOS permission prompt.
            Self::VncProfileSave { .. } | Self::VncForgetCredential { .. } => 120,
            _ => 3,
        })
    }
    /// Short stable name for timeouts and diagnostics. Never includes IDs,
    /// paths, or labels.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::VncProfileSave { .. } => "VncProfileSave",
            Self::VncProfileRemove { .. } => "VncProfileRemove",
            Self::VncConnect { .. } => "VncConnect",
            Self::VncForgetCredential { .. } => "VncForgetCredential",
            Self::VncTransfer { .. } => "VncTransfer",
            Self::VncTransferStatus { .. } => "VncTransferStatus",
            Self::VncDisconnect { .. } => "VncDisconnect",
            Self::VncInput { .. } => "VncInput",
            Self::VncFrame { .. } => "VncFrame",
            Self::RemoteAppsConnect { .. } => "RemoteAppsConnect",
            Self::RemoteAppsDisconnect { .. } => "RemoteAppsDisconnect",
            Self::RemoteAppsInput { .. } => "RemoteAppsInput",
            Self::RemoteAppsClipboard { .. } => "RemoteAppsClipboard",
            Self::RemoteAppsState { .. } => "RemoteAppsState",
            Self::PruneHistory { .. } => "PruneHistory",
            Self::Archived { request, .. } => request.name(),
            Self::CloseIdleSessions { .. } => "CloseIdleSessions",
            Self::ShellCommand { .. } => "ShellCommand",
            Self::ShellPrompt { .. } => "ShellPrompt",
            Self::CommandBlocks { .. } => "CommandBlocks",
            Self::WorktreeList { .. } => "WorktreeList",
            Self::WorktreeAdd { .. } => "WorktreeAdd",
            Self::WorktreeRemove { .. } => "WorktreeRemove",
            Self::Screen { .. } => "Screen",
            Self::TerminalNotify { .. } => "TerminalNotify",
            Self::DismissTerminalNotice { .. } => "DismissTerminalNotice",
            Self::Snapshot => "Snapshot",
            Self::SessionForPids { .. } => "SessionForPids",
            Self::CreateReview { .. } => "CreateReview",
            Self::AddProject { .. } => "AddProject",
            Self::AddRemoteProject { .. } => "AddRemoteProject",
            Self::RemoteProject { .. } => "RemoteProject",
            Self::ProbeRemoteProject { .. } => "ProbeRemoteProject",
            Self::RemoveRemoteProject { .. } => "RemoveRemoteProject",
            Self::SetRemoteProjectGroup { .. } => "SetRemoteProjectGroup",
            Self::UpdateRemoteProject { .. } => "UpdateRemoteProject",
            Self::SetRemoteProjectForwards { .. } => "SetRemoteProjectForwards",
            Self::StartForward { .. } => "StartForward",
            Self::StopForward { .. } => "StopForward",
            Self::ForwardStatus => "ForwardStatus",
            Self::ScanHostKey { .. } => "ScanHostKey",
            Self::TrustHostKey { .. } => "TrustHostKey",
            Self::SaveLayout { .. } => "SaveLayout",
            Self::SelectProject { .. } => "SelectProject",
            Self::SetInitCommand { .. } => "SetInitCommand",
            Self::Create { .. } => "Create",
            Self::Stop { .. } => "Stop",
            Self::Rename { .. } => "Rename",
            Self::Remove { .. } => "Remove",
            Self::Focus { .. } => "Focus",
            Self::Notice { .. } => "Notice",
            Self::Settings(_) => "Settings",
            Self::Heartbeat { .. } => "Heartbeat",
            Self::Hook(_) => "Hook",
            Self::Cwd { .. } => "Cwd",
            Self::Attach { .. } => "Attach",
            Self::Input { .. } => "Input",
            Self::Resize { .. } => "Resize",
            Self::History { .. } => "History",
            Self::ClearHistory { .. } => "ClearHistory",
            Self::EditorCompare { .. } => "EditorCompare",
            Self::EditorSave { .. } => "EditorSave",
            Self::EditorStatus { .. } => "EditorStatus",
            Self::Shutdown => "Shutdown",
            Self::ShutdownIfIdle => "ShutdownIfIdle",
            Self::RetireIfDraining => "RetireIfDraining",
            Self::LocalHistoryList { .. } => "LocalHistoryList",
            Self::LocalHistoryRead { .. } => "LocalHistoryRead",
            Self::LocalHistoryLabel { .. } => "LocalHistoryLabel",
            Self::LocalHistoryRevert { .. } => "LocalHistoryRevert",
            Self::LocalHistoryPage { .. } => "LocalHistoryPage",
            Self::NtfyToken { .. } => "NtfyToken",
            Self::CreateRestored { .. } => "CreateRestored",
        }
    }
}
