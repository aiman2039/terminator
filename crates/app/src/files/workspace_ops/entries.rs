//! File actions run from the sidebar. Every path stays inside the root.
use super::*;

/// Explorer delete touches only the working tree (as in `IntelliJ`): the
/// index is left alone, so a tracked file shows as deleted, unstaged.
pub(super) fn delete(root: &Path, path: &Path) -> Result<Report> {
    let path = contained_entry(root, path)?;
    move_to_trash(&path)?;
    Ok(Report::Message(format!(
        "Moved {} to Trash",
        path.display()
    )))
}

/// Recoverable removal: the system Trash on macOS and through `gio` on
/// Linux. Tests remove directly so they never fill the user's Trash.
#[cfg(all(target_os = "macos", not(test)))]
pub(super) fn move_to_trash(path: &Path) -> Result<()> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|error| anyhow::anyhow!("{}", error.localizedDescription()))
        .with_context(|| format!("move {} to Trash", path.display()))
}

#[cfg(all(target_os = "linux", not(test)))]
pub(super) fn move_to_trash(path: &Path) -> Result<()> {
    let status = std::process::Command::new("gio")
        .arg("trash")
        .arg("--")
        .arg(path)
        .status()
        .with_context(|| format!("move {} to Trash (gio)", path.display()))?;
    ensure!(status.success(), "gio trash failed for {}", path.display());
    Ok(())
}

#[cfg(any(test, not(any(target_os = "macos", target_os = "linux"))))]
pub(super) fn move_to_trash(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).with_context(|| format!("stat {}", path.display()))?;
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .with_context(|| format!("delete {}", path.display()))
}

pub(super) fn reveal(path: &Path) -> Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("open");
        command.arg("-R").arg(path);
        command
    } else {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(path.parent().unwrap_or(path));
        command
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("reveal file")?;
    Ok(())
}

pub(super) fn relative_arg(root: &Path, path: &Path) -> Result<String> {
    let resolved = contained_link(root, path)?;
    let root = root.canonicalize().context("repository root")?;
    let relative = resolved
        .strip_prefix(&root)
        .context("path is outside the repository")?;
    Ok(relative.display().to_string())
}

/// Resolve `path` inside `root` without following its final component, so
/// symlink actions affect the link itself rather than its target.
/// Intermediate components resolve through the parent, which also keeps
/// missing worktree files (staged deletions) and broken links usable.
pub(super) fn contained_join(root: &Path, path: &Path, allow_root: bool) -> Result<PathBuf> {
    let root = root.canonicalize().context("root")?;
    let name = path.file_name().context("missing file name")?;
    let parent = path.parent().context("missing parent")?;
    let parent = if parent.as_os_str().is_empty() {
        std::env::current_dir().context("current dir")?
    } else {
        parent.canonicalize().context("parent")?
    };
    ensure!(parent.starts_with(&root), "path escapes the root");
    let resolved = parent.join(name);
    ensure!(resolved.starts_with(&root), "path escapes the root");
    if !allow_root {
        ensure!(resolved != root, "refusing the repository root");
    }
    Ok(resolved)
}

pub(super) fn contained_link(root: &Path, path: &Path) -> Result<PathBuf> {
    contained_join(root, path, false)
}

/// A file, folder, or link inside the root (never the root itself).
pub(super) fn contained_entry(root: &Path, path: &Path) -> Result<PathBuf> {
    let path = contained_link(root, path)?;
    fs::symlink_metadata(&path).with_context(|| format!("stat {}", path.display()))?;
    Ok(path)
}

/// An existing directory inside the root (the root itself allowed).
pub(super) fn contained_dir(root: &Path, path: &Path) -> Result<PathBuf> {
    let path = contained_join(root, path, true)?;
    ensure!(path.is_dir(), "{} is not a folder", path.display());
    Ok(path)
}

/// Copy a file, link, or folder tree. Links are recreated, never followed,
/// so a copy cannot pull in files from outside the source.
pub(super) fn copy_entry(source: &Path, dest: &Path) -> Result<()> {
    let meta =
        fs::symlink_metadata(source).with_context(|| format!("stat {}", source.display()))?;
    if meta.file_type().is_symlink() {
        return replicate_symlink(source, dest);
    }
    if meta.is_dir() {
        fs::create_dir(dest).with_context(|| format!("create {}", dest.display()))?;
        for entry in fs::read_dir(source).with_context(|| format!("read {}", source.display()))? {
            let entry = entry?;
            copy_entry(&entry.path(), &dest.join(entry.file_name()))?;
        }
        return Ok(());
    }
    fs::copy(source, dest).with_context(|| format!("copy {}", source.display()))?;
    Ok(())
}

pub(super) fn contained_file(root: &Path, path: &Path) -> Result<PathBuf> {
    let path = contained_link(root, path)?;
    let meta = fs::symlink_metadata(&path).with_context(|| format!("stat {}", path.display()))?;
    ensure!(
        meta.is_file() || meta.file_type().is_symlink(),
        "{} is not a file",
        path.display()
    );
    Ok(path)
}

/// `Path::exists` follows links and misses broken ones; the guards need the
/// link itself to exist.
pub(super) fn link_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

#[cfg(unix)]
pub(super) fn replicate_symlink(path: &Path, dest: &Path) -> Result<()> {
    let target = fs::read_link(path).with_context(|| format!("read link {}", path.display()))?;
    std::os::unix::fs::symlink(&target, dest)
        .with_context(|| format!("copy {}", path.display()))?;
    Ok(())
}

#[cfg(windows)]
pub(super) fn replicate_symlink(path: &Path, dest: &Path) -> Result<()> {
    let target = fs::read_link(path).with_context(|| format!("read link {}", path.display()))?;
    let context = || format!("copy {}", path.display());
    if path.canonicalize().is_ok_and(|resolved| resolved.is_dir()) {
        std::os::windows::fs::symlink_dir(&target, dest).with_context(context)?;
    } else {
        std::os::windows::fs::symlink_file(&target, dest).with_context(context)?;
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
pub(super) fn replicate_symlink(path: &Path, dest: &Path) -> Result<()> {
    fs::copy(path, dest).with_context(|| format!("copy {}", path.display()))?;
    Ok(())
}

pub(super) fn new_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .context("missing file name")?
        .to_string_lossy();
    ensure!(
        !name.is_empty() && !name.contains('/') && name != "." && name != "..",
        "file name is not allowed"
    );
    let parent = path.parent().context("missing parent")?;
    let parent = if parent.exists() {
        parent.canonicalize().context("parent")?
    } else {
        bail!("parent directory does not exist")
    };
    let root = root.canonicalize().context("root")?;
    ensure!(parent.starts_with(&root), "path escapes the root");
    Ok(parent.join(name.as_ref()))
}

/// Like [`new_path`], but a typed `a/b/c.rs` creates the missing `a/b`
/// first (as `IntelliJ` New File does). Only plain names may follow the deepest
/// existing ancestor, and that ancestor must resolve inside the root, so
/// neither `..` nor a symlinked directory can escape it.
pub(super) fn new_nested_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let mut existing = path.parent().context("missing parent")?;
    let mut missing = Vec::new();
    while !existing.exists() {
        let name = existing.file_name().context("missing parent")?;
        missing.push(name.to_owned());
        existing = existing.parent().context("missing parent")?;
    }
    let base = existing.canonicalize().context("parent")?;
    let root = root.canonicalize().context("root")?;
    ensure!(base.starts_with(&root), "path escapes the root");
    let mut parent = base;
    for name in missing.iter().rev() {
        let text = name.to_string_lossy();
        ensure!(
            !text.is_empty() && text != "." && text != "..",
            "folder name is not allowed"
        );
        parent.push(name);
    }
    fs::create_dir_all(&parent).with_context(|| format!("create {}", parent.display()))?;
    new_path(
        &root,
        &parent.join(path.file_name().context("missing file name")?),
    )
}

pub(super) fn duplicate_destination(path: &Path) -> Result<PathBuf> {
    let parent = path.parent().context("missing parent")?;
    // Folders keep their whole name: `v1.2 copy`, not `v1 copy.2`.
    let folder = path.is_dir();
    let stem = if folder {
        path.file_name()
    } else {
        path.file_stem()
    }
    .context("missing name")?
    .to_string_lossy()
    .into_owned();
    let ext = path
        .extension()
        .filter(|_| !folder)
        .map(|ext| ext.to_string_lossy().into_owned());
    for n in 1..100 {
        let name = if n == 1 {
            format!("{stem} copy")
        } else {
            format!("{stem} copy {n}")
        };
        let file = match &ext {
            Some(ext) => format!("{name}.{ext}"),
            None => name,
        };
        let dest = parent.join(file);
        if !link_exists(&dest) {
            return Ok(dest);
        }
    }
    bail!("no free copy name")
}

pub(super) fn discard(root: &Path, path: &Path, untracked: bool) -> Result<Report> {
    if untracked {
        let path = contained_file(root, path)?;
        fs::remove_file(&path).with_context(|| format!("delete {}", path.display()))?;
        Ok(Report::Message(format!("Removed {}", path.display())))
    } else {
        // Restore the worktree from the index (the default source),
        // never from HEAD: partially staged files keep their staged
        // content, and newly staged files have no HEAD version.
        git_path(GitPathInput {
            root,
            prefix: &["restore", "--worktree", "--"],
            path,
            verb: "Discarded",
        })
    }
}

pub(super) fn create_file(root: &Path, path: &Path) -> Result<Report> {
    let path = new_nested_path(root, path)?;
    ensure!(!link_exists(&path), "{} already exists", path.display());
    fs::write(&path, "").with_context(|| format!("create {}", path.display()))?;
    Ok(Report::Message(format!("Created {}", path.display())))
}

pub(super) fn create_dir(root: &Path, path: &Path) -> Result<Report> {
    let path = new_nested_path(root, path)?;
    ensure!(!link_exists(&path), "{} already exists", path.display());
    fs::create_dir(&path).with_context(|| format!("create {}", path.display()))?;
    Ok(Report::Message(format!("Created {}", path.display())))
}

pub(super) fn duplicate(root: &Path, path: &Path) -> Result<Report> {
    let path = contained_entry(root, path)?;
    let dest = duplicate_destination(&path)?;
    copy_entry(&path, &dest)?;
    Ok(Report::Message(format!("Copied to {}", dest.display())))
}

pub(super) fn move_entries(root: &Path, from: &[PathBuf], to_dir: &Path) -> Result<Report> {
    let dir = contained_dir(root, to_dir)?;
    for source in from {
        let source = contained_entry(root, source)?;
        ensure!(
            !dir.starts_with(&source),
            "cannot move {} into itself",
            source.display()
        );
        let dest = dir.join(source.file_name().context("missing file name")?);
        if dest == source {
            continue;
        }
        ensure!(!link_exists(&dest), "{} already exists", dest.display());
        fs::rename(&source, &dest).with_context(|| format!("move {}", source.display()))?;
    }
    Ok(Report::Message(format!(
        "Moved {} item{} to {}",
        from.len(),
        if from.len() == 1 { "" } else { "s" },
        dir.display()
    )))
}

pub(super) fn copy_entries(root: &Path, from: &[PathBuf], to_dir: &Path) -> Result<Report> {
    let dir = contained_dir(root, to_dir)?;
    for source in from {
        let meta =
            fs::symlink_metadata(source).with_context(|| format!("stat {}", source.display()))?;
        ensure!(
            !meta.is_dir() || !dir.starts_with(source),
            "cannot copy {} into itself",
            source.display()
        );
        let name = source.file_name().context("missing file name")?;
        let dest = if link_exists(&dir.join(name)) {
            duplicate_destination(&dir.join(name))?
        } else {
            dir.join(name)
        };
        copy_entry(source, &dest)?;
    }
    Ok(Report::Message(format!(
        "Copied {} item{} to {}",
        from.len(),
        if from.len() == 1 { "" } else { "s" },
        dir.display()
    )))
}

pub(super) fn rename(root: &Path, from: &Path, to: &Path) -> Result<Report> {
    let from = contained_entry(root, from)?;
    let to = new_path(root, to)?;
    ensure!(!link_exists(&to), "{} already exists", to.display());
    fs::rename(&from, &to).with_context(|| format!("rename {}", from.display()))?;
    Ok(Report::Message(format!("Renamed to {}", to.display())))
}

pub(super) fn reveal_entry(root: &Path, path: &Path) -> Result<Report> {
    let path = contained_join(root, path, true)?;
    reveal(&path)?;
    Ok(Report::Message(format!("Revealed {}", path.display())))
}
