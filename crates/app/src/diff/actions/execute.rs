//! Applying native-review mutations: save, hunk changes and undo.
use super::*;

async fn current(service: &Services, change: &Undo, cancel: &CancellationToken) -> Result<Version> {
    if change.index {
        let entry = git(
            service,
            &change.root,
            vec![
                "ls-files".into(),
                "--stage".into(),
                "-z".into(),
                "--".into(),
                literal(&change.path),
            ],
            None,
        )
        .await?;
        ensure!(
            entry.split(|b| *b == 0).filter(|s| !s.is_empty()).count() <= 1,
            "Resolve the file's merge conflict first"
        );
        let oid = entry
            .split(|b| *b == b' ')
            .nth(1)
            .map(std::str::from_utf8)
            .transpose()?;
        let text = if let Some(oid) = oid {
            git(
                service,
                &change.root,
                vec!["cat-file".into(), "blob".into(), oid.into()],
                None,
            )
            .await?
        } else {
            Vec::new()
        };
        Ok(Version {
            text: String::from_utf8(text)?,
            exists: !entry.is_empty(),
            mode: entry_mode(&entry),
        })
    } else {
        let path = change.root.join(&change.path);
        service
            .fs()
            .run(cancel, move || {
                let (exists, mode) = file_identity(&path)?;
                Ok(Version {
                    text: String::from_utf8(terminator_git::worktree_file(&path)?)?,
                    exists,
                    mode,
                })
            })
            .await
    }
}

async fn apply(service: &Services, change: &Undo, cancel: &CancellationToken) -> Result<()> {
    let root = change.root.clone();
    let path = change.path.clone();
    service
        .fs()
        .run(cancel, move || {
            ensure!(
                terminator_git::relative_path(&root, &root.join(&path))? == path,
                "The file path changed; refresh the diff"
            );
            Ok(())
        })
        .await?;
    ensure!(
        current(service, change, cancel).await? == change.before,
        "The file or index changed; refresh the diff before retrying"
    );
    let path = change.path.clone();
    let before = change.before.clone();
    let after = change.after.clone();
    let bytes = service
        .cpu()
        .run(cancel, move || anyhow::Ok(patch(&path, &before, &after)))
        .await?;
    let mut args: Vec<OsString> = vec!["apply".into(), "--whitespace=nowarn".into()];
    if change.index {
        args.push("--cached".into());
    }
    let mut check = args.clone();
    check.push("--check".into());
    git(service, &change.root, check, Some(bytes.clone())).await?;
    // Recheck the complete target, including changes outside the selected hunk.
    ensure!(
        current(service, change, cancel).await? == change.before,
        "The file or index changed; refresh the diff before retrying"
    );
    git(service, &change.root, args, Some(bytes)).await?;
    Ok(())
}

pub(crate) async fn execute(
    service: &Services,
    request: Request,
    staged: bool,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    // Serialize the full read/check/apply sequence across native diff tabs.
    static MUTATIONS: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    let _guard = MUTATIONS
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let actions = Actions { service, cancel };
    match request {
        Request::Save { document, text } => actions.save(&document, text, staged).await,
        Request::Hunk {
            document,
            hunk,
            kind,
            lines,
        } => {
            let pick = HunkPick { hunk, kind, lines };
            actions.change_hunk(&document, pick, staged).await
        }
        Request::Undo(change) => actions.undo(change).await,
    }
}

/// One hunk, or some of its lines, to stage, unstage or discard.
struct HunkPick {
    hunk: usize,
    kind: Kind,
    lines: Option<LineSelection>,
}

#[derive(Clone, Copy)]
struct Actions<'a> {
    service: &'a Services,
    cancel: &'a CancellationToken,
}

impl Actions<'_> {
    /// Reloads `document` and fails with `stale` unless nothing changed.
    async fn ensure_fresh(self, document: &DiffDocument, staged: bool, stale: &str) -> Result<()> {
        let source = document
            .source
            .as_ref()
            .context("Refresh the diff before editing")?;
        let fresh = crate::diff::document_async(crate::diff::DocumentAsyncInput {
            service: self.service,
            cwd: source.root.clone(),
            path: source.root.join(&source.path),
            staged,
            cancel: self.cancel,
        })
        .await?;
        ensure!(
            fresh.source == document.source
                && fresh.left_text == document.left_text
                && fresh.right_text == document.right_text,
            "{stale}"
        );
        Ok(())
    }

    async fn save(self, document: &DiffDocument, text: String, staged: bool) -> Result<Outcome> {
        ensure!(!document.read_only, "This diff is read-only");
        ensure!(!staged, "Only the working tree can be edited");
        let source = document
            .source
            .as_ref()
            .context("Refresh the diff before editing")?;
        self.ensure_fresh(
            document,
            false,
            "The file, HEAD, or index changed; your edits were kept. Reopen the file to resolve the changes",
        )
        .await?;
        let before = Version {
            text: document.right_text.clone(),
            exists: source.exists,
            mode: source.mode,
        };
        let change = Undo {
            root: source.root.clone(),
            path: source.path.clone(),
            index: false,
            after: Version {
                text,
                exists: true,
                mode: before.mode,
            },
            before,
        };
        apply(self.service, &change, self.cancel).await?;
        Ok(Outcome {
            undo: Some(change),
            message: "File saved".into(),
        })
    }

    async fn change_hunk(
        self,
        document: &DiffDocument,
        pick: HunkPick,
        staged: bool,
    ) -> Result<Outcome> {
        let HunkPick { hunk, kind, lines } = pick;
        ensure!(!document.read_only, "This diff is read-only");
        ensure!(
            (kind == Kind::Unstage) == staged,
            "This action does not match the diff side"
        );
        let source = document
            .source
            .as_ref()
            .context("Refresh the diff before changing a hunk")?;
        self.ensure_fresh(
            document,
            staged,
            "The file, HEAD, or index changed; refresh the diff before retrying",
        )
        .await?;
        let hunk = document
            .hunks
            .get(hunk)
            .context("This hunk is no longer available; refresh the diff")?;
        let change = edit(EditInput {
            source,
            doc: document,
            hunk,
            kind,
            lines: lines.as_ref(),
        });
        apply(self.service, &change, self.cancel).await?;
        Ok(Outcome {
            undo: Some(change),
            message: match &lines {
                Some(picked) if !picked.is_empty() => format!(
                    "{} complete for {} line{}",
                    kind.label(),
                    picked.len(),
                    if picked.len() == 1 { "" } else { "s" }
                ),
                _ => format!("{} complete", kind.label()),
            },
        })
    }

    async fn undo(self, change: Undo) -> Result<Outcome> {
        let reverse = Undo {
            before: change.after.clone(),
            after: change.before.clone(),
            ..change
        };
        apply(self.service, &reverse, self.cancel).await?;
        Ok(Outcome {
            undo: None,
            message: "Undo complete".into(),
        })
    }
}
