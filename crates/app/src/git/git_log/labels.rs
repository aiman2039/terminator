//! Ref labels on commit rows, parsed from `%D` with `--decorate=full`.

/// What a ref label names, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefLabelKind {
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
}

/// A ref that points at a commit, shown as a pill on the commit's row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefLabel {
    pub name: String,
    pub kind: RefLabelKind,
}

impl RefLabel {
    fn head() -> Self {
        Self {
            name: "HEAD".into(),
            kind: RefLabelKind::Head,
        }
    }
}

/// Labels from a full `%D` list such as `HEAD -> refs/heads/main,
/// tag: refs/tags/v1, refs/remotes/origin/main`: HEAD first, then local
/// branches, remote branches, and tags. Full names tell a local `a/b` from
/// a remote `origin/b`. Remote `HEAD` aliases and other refs (notes,
/// `grafted`) are left out.
pub fn parse_ref_labels(decorations: &str) -> Vec<RefLabel> {
    let mut labels = Vec::new();
    for item in decorations.split(", ").map(str::trim) {
        let item = match item.strip_prefix("HEAD -> ") {
            Some(branch) => {
                labels.push(RefLabel::head());
                branch
            }
            None if item == "HEAD" => {
                labels.push(RefLabel::head());
                continue;
            }
            None => item,
        };
        if let Some(label) = ref_label(item.strip_prefix("tag: ").unwrap_or(item)) {
            labels.push(label);
        }
    }
    // Stable: Git's order stays within each kind.
    labels.sort_by_key(|label| label.kind);
    labels
}

fn ref_label(full: &str) -> Option<RefLabel> {
    let (kind, name) = if let Some(name) = full.strip_prefix("refs/heads/") {
        (RefLabelKind::LocalBranch, name)
    } else if let Some(name) = full.strip_prefix("refs/remotes/") {
        if name.ends_with("/HEAD") {
            return None;
        }
        (RefLabelKind::RemoteBranch, name)
    } else {
        (RefLabelKind::Tag, full.strip_prefix("refs/tags/")?)
    };
    Some(RefLabel {
        name: name.to_owned(),
        kind,
    })
}
