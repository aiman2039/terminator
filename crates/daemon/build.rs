//! Build the pinned `CodeDiff` library locally; never download executables at runtime.
#![forbid(unsafe_code)]
use std::{env, fs, path::Path};
fn collect(root: &Path, dir: &Path, entries: &mut Vec<(String, String)>) {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect(root, &path, entries);
        } else {
            entries.push((
                path.strip_prefix(root).unwrap().to_str().unwrap().into(),
                path.to_str().unwrap().into(),
            ));
        }
    }
}
fn main() {
    let root = Path::new("../../vendor/codediff.nvim")
        .canonicalize()
        .unwrap();
    println!("cargo:rerun-if-changed={}", root.display());
    let out = std::path::PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let source = root.join("libvscode-diff");
    let version = fs::read_to_string(root.join("VERSION")).unwrap();
    fs::write(
        out.join("version.h"),
        format!("#define VSCODE_DIFF_VERSION {:?}\n", version.trim()),
    )
    .unwrap();
    let ext = if env::var("CARGO_CFG_TARGET_OS").unwrap() == "macos" {
        "dylib"
    } else {
        "so"
    };
    let library = out.join(format!("libvscode_diff.{ext}"));
    let compiler = cc::Build::new().get_compiler();
    let mut cmd = compiler.to_command();
    cmd.args([
        "-shared",
        "-fPIC",
        "-O2",
        "-std=c11",
        "-DNDEBUG",
        "-DUTF8PROC_STATIC",
        "-D_POSIX_C_SOURCE=200809L",
    ]);
    for dir in [source.join("include"), source.join("vendor"), out.clone()] {
        cmd.arg("-I").arg(dir);
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
    cmd.arg("-lm").arg("-o").arg(&library);
    assert!(
        cmd.status()
            .expect("C compiler required for bundled CodeDiff")
            .success(),
        "CodeDiff native build failed"
    );
    if ext == "dylib" {
        let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
            "aarch64" => "arm64",
            "x86_64" => "x86_64",
            arch => panic!("Unsupported macOS architecture: {arch}"),
        };
        assert!(
            std::process::Command::new("lipo")
                .arg(&library)
                .args(["-verify_arch", arch])
                .status()
                .expect("lipo is required")
                .success(),
            "Embedded CodeDiff must match the daemon architecture"
        );
    }
    let mut entries = vec![];
    collect(&root, &root.join("lua"), &mut entries);
    collect(&root, &root.join("plugin"), &mut entries);
    entries.push((
        "VERSION".into(),
        root.join("VERSION").to_str().unwrap().into(),
    ));
    entries.push((
        format!("libvscode_diff.{ext}"),
        library.to_str().unwrap().into(),
    ));
    let mut generated = String::from("const ASSETS: &[(&str, &[u8])] = &[\n");
    for (name, path) in entries {
        generated.push_str(&format!("({name:?}, include_bytes!({path:?})),\n"));
    }
    generated.push_str("];\n");
    fs::write(out.join("review_assets.rs"), generated).unwrap();
}
