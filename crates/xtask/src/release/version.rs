use anyhow::{Context, Result, ensure};
use std::{collections::BTreeSet, fs, path::Path};
use toml_edit::{DocumentMut, Item};

pub(super) fn document(path: &Path) -> Result<DocumentMut> {
    fs::read_to_string(path)?
        .parse()
        .with_context(|| format!("Invalid TOML: {}", path.display()))
}

fn replace(item: &mut Item, version: &str) -> Result<()> {
    let decor = item
        .as_value()
        .context("Expected a version value")?
        .decor()
        .clone();
    let mut value = toml_edit::Value::from(version);
    *value.decor_mut() = decor;
    *item = Item::Value(value);
    Ok(())
}

pub(super) fn field<'a>(document: &'a DocumentMut, path: &[&str]) -> Result<&'a Item> {
    path.iter().try_fold(document.as_item(), |item, key| {
        item.get(key)
            .with_context(|| format!("Missing TOML field: {}", path.join(".")))
    })
}

fn field_mut<'a>(document: &'a mut DocumentMut, path: &[&str]) -> Result<&'a mut Item> {
    path.iter().try_fold(document.as_item_mut(), |item, key| {
        item.get_mut(key)
            .with_context(|| format!("Missing TOML field: {}", path.join(".")))
    })
}

pub fn bump(root: &Path) -> Result<String> {
    let mut manifest = document(&root.join("Cargo.toml"))?;
    let old = field(&manifest, &["workspace", "package", "version"])?
        .as_str()
        .context("Missing workspace version")?
        .to_owned();
    let parsed = semver::Version::parse(&old)?;
    ensure!(
        parsed.pre.is_empty() && parsed.build.is_empty(),
        "Expected a stable major.minor.patch workspace version"
    );
    let next = format!(
        "{}.{}.0",
        parsed.major,
        parsed
            .minor
            .checked_add(1)
            .context("Workspace minor version overflow")?
    );
    let workspace = field(&manifest, &["workspace"])?;
    let members = workspace
        .get("members")
        .context("Missing workspace members")?
        .as_array()
        .context("Missing workspace members")?;
    let excluded = workspace
        .get("exclude")
        .and_then(Item::as_array)
        .map(|values| {
            values
                .iter()
                .map(|v| {
                    glob::Pattern::new(v.as_str().context("Invalid workspace exclusion")?)
                        .map_err(Into::into)
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    let mut names = BTreeSet::new();
    for member in members {
        let pattern = format!(
            "{}/{}",
            glob::Pattern::escape(&root.to_string_lossy()),
            member.as_str().context("Invalid workspace member")?
        );
        for directory in glob::glob(&pattern)? {
            let directory = directory?;
            let relative = directory.strip_prefix(root)?;
            if excluded
                .iter()
                .any(|pattern| pattern.matches_path(relative))
            {
                continue;
            }
            let member = document(&directory.join("Cargo.toml"))?;
            let package = field(&member, &["package"])?;
            if package
                .get("version")
                .and_then(|v| v.get("workspace"))
                .and_then(Item::as_bool)
                == Some(true)
            {
                names.insert(
                    package
                        .get("name")
                        .context("Missing package name")?
                        .as_str()
                        .context("Missing package name")?
                        .to_owned(),
                );
            }
        }
    }
    ensure!(
        !names.is_empty(),
        "No workspace packages inherit the version"
    );
    let mut lock = document(&root.join("Cargo.lock"))?;
    let mut seen = BTreeSet::new();
    for package in field_mut(&mut lock, &["package"])?
        .as_array_of_tables_mut()
        .context("Missing lockfile packages")?
        .iter_mut()
    {
        let name = package
            .get("name")
            .context("Missing lockfile package name")?
            .as_str()
            .context("Missing lockfile package name")?
            .to_owned();
        if !names.contains(&name) || package.contains_key("source") {
            continue;
        }
        ensure!(
            package.get("version").and_then(Item::as_str) == Some(old.as_str())
                && seen.insert(name.clone()),
            "Unexpected lockfile entry for {name}; no files changed"
        );
        replace(
            package
                .get_mut("version")
                .context("Missing lockfile version")?,
            &next,
        )?;
    }
    ensure!(
        seen == names,
        "Workspace packages missing from Cargo.lock; no files changed"
    );
    replace(
        field_mut(&mut manifest, &["workspace", "package", "version"])?,
        &next,
    )?;
    fs::write(root.join("Cargo.toml"), manifest.to_string())?;
    fs::write(root.join("Cargo.lock"), lock.to_string())?;
    println!("Bumped workspace version: {old} -> {next}");
    Ok(next)
}
