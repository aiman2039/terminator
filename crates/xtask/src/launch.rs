//! Compose launch graphics from freshly validated native fixture captures.
use crate::harness::{artifacts, root};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{fs, path::Path};

pub fn run() -> Result<()> {
    let kit = root().join("launch/product-hunt");
    let copy: serde_json::Value = serde_json::from_slice(&fs::read(kit.join("copy.json"))?)?;
    for (key, limit) in [("tagline", 60), ("description", 260)] {
        let count = copy[key]
            .as_str()
            .context("Missing launch copy")?
            .chars()
            .count();
        ensure!(count <= limit, "{key} exceeds {limit} characters");
        println!("{key}: {count}/{limit} characters");
    }
    let staging = tempfile::Builder::new()
        .prefix(".assets-")
        .tempdir_in(&kit)?;
    let thumbnail = image::open(root().join("crates/app/assets/branding/terminator-512.png"))?;
    thumbnail
        .resize_exact(240, 240, image::imageops::FilterType::Lanczos3)
        .save(staging.path().join("thumbnail.png"))?;
    let native = artifacts().join("native");
    for (file, title, subtitle, source) in [
        (
            "01-projects-splits.png",
            "Keep each project in view",
            "Project tabs and native terminal splits",
            "launch/projects.png",
        ),
        (
            "02-agent-attention.png",
            "Know when a tool needs you",
            "Supported hooks bring agent attention into the workspace · sample event",
            "launch/attention.png",
        ),
        (
            "03-editing-previews.png",
            "Read and edit in the same workspace",
            "Neovim editing and native Markdown previews",
            "launch/editing.png",
        ),
        (
            "04-reconnection.png",
            "Come back to running sessions",
            "Close the GUI, reopen it, and attach to the same daemon-owned sessions",
            "launch/reconnect.png",
        ),
    ] {
        let source = native.join(source);
        ensure!(
            source.exists(),
            "Run native fixtures first; missing {}",
            source.display()
        );
        render(&source, title, subtitle, &staging.path().join(file))?;
    }
    for entry in fs::read_dir(staging.path())? {
        let entry = entry?;
        let dimensions = image::image_dimensions(entry.path())?;
        let expected = if entry.file_name() == "thumbnail.png" {
            (240, 240)
        } else {
            (1270, 760)
        };
        ensure!(dimensions == expected, "Unexpected image dimensions");
        fs::rename(entry.path(), kit.join(entry.file_name()))?;
    }
    println!("Launch assets: {}", kit.display());
    Ok(())
}
fn render(source: &Path, title: &str, subtitle: &str, destination: &Path) -> Result<()> {
    let encoded = STANDARD.encode(fs::read(source)?);
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="1270" height="760" viewBox="0 0 1270 760">
<rect width="1270" height="760" fill="#0b0f15"/>
<rect x="42" y="40" width="5" height="58" rx="2" fill="#ffac76"/>
<text x="65" y="68" fill="#f4f5f8" font-family="Inter" font-size="31" font-weight="600">{title}</text>
<text x="65" y="96" fill="#aeb9c9" font-family="Inter" font-size="17">{subtitle}</text>
<rect x="39" y="127" width="1192" height="574" rx="12" fill="#151a22" stroke="#323b49"/>
<image x="46" y="134" width="1178" height="560" preserveAspectRatio="xMidYMid meet" xlink:href="data:image/png;base64,{encoded}"/>
<text x="42" y="737" fill="#ffac76" font-family="Inter" font-size="15" letter-spacing="3">TERMINATOR</text>
<text x="1228" y="737" fill="#aeb9c9" font-family="Inter" font-size="14" text-anchor="end">Native Rust · Free and open source</text>
</svg>"##
    );
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    options
        .fontdb_mut()
        .load_fonts_dir(root().join("crates/app/assets/fonts"));
    let tree = resvg::usvg::Tree::from_str(&svg, &options)?;
    let mut pixels =
        resvg::tiny_skia::Pixmap::new(1270, 760).context("Cannot allocate gallery image")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixels.as_mut(),
    );
    pixels.save_png(destination)?;
    Ok(())
}
