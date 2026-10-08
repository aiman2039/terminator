//! Build the pinned `CodeDiff` library locally; never download executables at runtime.
#![forbid(unsafe_code)]
use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// `canonicalize` returns verbatim `\\?\` paths on Windows, which MSVC
/// (`cl`) cannot open. Strip the prefix so compiler and include args work.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

fn fail(error: impl std::fmt::Display) -> ! {
    eprintln!("daemon build failed: {error}");
    std::process::exit(1);
}

fn utf8_path(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{} is not UTF-8", path.display()))
}

fn collect(
    root: &Path,
    dir: &Path,
    entries: &mut Vec<(String, String)>,
) -> Result<(), Box<dyn Error>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir)? {
        paths.push(entry?.path());
    }
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect(root, &path, entries)?;
        } else {
            entries.push((utf8_path(path.strip_prefix(root)?)?, utf8_path(&path)?));
        }
    }
    Ok(())
}

fn run() -> Result<(), Box<dyn Error>> {
    let root = without_verbatim_prefix(Path::new("../../vendor/codediff.nvim").canonicalize()?);
    println!("cargo:rerun-if-changed={}", root.display());
    let out = std::path::PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is not set")?);
    let source = root.join("libvscode-diff");
    let version = fs::read_to_string(root.join("VERSION"))?;
    fs::write(
        out.join("version.h"),
        format!("#define VSCODE_DIFF_VERSION {:?}\n", version.trim()),
    )?;
    let target_os = env::var("CARGO_CFG_TARGET_OS")?;
    let ext = match target_os.as_str() {
        "macos" => "dylib",
        "windows" => "dll",
        _ => "so",
    };
    let library = out.join(format!("libvscode_diff.{ext}"));
    let compiler = cc::Build::new().get_compiler();
    let mut cmd = compiler.to_command();
    if compiler.is_like_msvc() {
        // MSVC: `/LD` builds the DLL; the C runtime supplies libm.
        // `BUILDING_DLL` selects `dllexport` in
        // `default_lines_diff_computer.h`; without it our own functions are
        // declared `dllimport`, matching upstream `build.cmd.in`.
        cmd.args([
            "/LD",
            "/O2",
            "/std:clatest",
            "/DNDEBUG",
            "/DUTF8PROC_STATIC",
            "/DBUILDING_DLL",
        ]);
        for dir in [source.join("include"), source.join("vendor"), out.clone()] {
            cmd.arg(format!("/I{}", dir.display()));
        }
    } else {
        cmd.args([
            "-shared",
            "-fPIC",
            "-O2",
            "-std=c11",
            "-DNDEBUG",
            "-DUTF8PROC_STATIC",
            "-D_POSIX_C_SOURCE=200809L",
        ]);
        if target_os == "windows" {
            // Non-MSVC Windows compilers (MinGW) also define `_WIN32`, so the
            // export header needs the same define.
            cmd.arg("-DBUILDING_DLL");
        }
        for dir in [source.join("include"), source.join("vendor"), out.clone()] {
            cmd.arg("-I").arg(dir);
        }
    }
    for file in [
        "default_lines_diff_computer.c",
        "src/char_level.c",
        "src/line_level.c",
        "src/myers.c",
        "src/optimize.c",
        "src/sequence.c",
        "src/range_mapping.c",
        "src/string_hash_map.c",
        "src/utils.c",
        "src/print_utils.c",
        "src/utf8_utils.c",
        "src/compute_moved_lines.c",
        "vendor/utf8proc.c",
    ] {
        cmd.arg(source.join(file));
    }
    if compiler.is_like_msvc() {
        cmd.arg(format!("/Fe{}", library.display()));
    } else {
        cmd.arg("-lm").arg("-o").arg(&library);
    }
    let status = cmd
        .status()
        .map_err(|error| format!("C compiler required for bundled CodeDiff: {error}"))?;
    if !status.success() {
        return Err("CodeDiff native build failed".into());
    }
    if ext == "dylib" {
        let arch = match env::var("CARGO_CFG_TARGET_ARCH")?.as_str() {
            "aarch64" => "arm64",
            "x86_64" => "x86_64",
            arch => return Err(format!("Unsupported macOS architecture: {arch}").into()),
        };
        let status = Command::new("lipo")
            .arg(&library)
            .args(["-verify_arch", arch])
            .status()
            .map_err(|error| format!("lipo is required: {error}"))?;
        if !status.success() {
            return Err("Embedded CodeDiff must match the daemon architecture".into());
        }
    }
    let mut entries = vec![];
    collect(&root, &root.join("lua"), &mut entries)?;
    collect(&root, &root.join("plugin"), &mut entries)?;
    entries.push(("VERSION".into(), utf8_path(&root.join("VERSION"))?));
    entries.push((format!("libvscode_diff.{ext}"), utf8_path(&library)?));
    let mut generated = String::from("const ASSETS: &[(&str, &[u8])] = &[\n");
    for (name, path) in entries {
        generated.push_str(&format!("({name:?}, include_bytes!({path:?})),\n"));
    }
    generated.push_str("];\n");
    fs::write(out.join("review_assets.rs"), generated)?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        fail(error);
    }
}
