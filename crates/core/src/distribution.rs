//! Names shared by the Linux packages, the package-manager templates and
//! the GUI update chip.
use std::path::Path;

/// Package, launcher and folder name for the Linux packages, the Homebrew
/// cask and the AUR package. Distinct from `terminator`, which Linux
/// distributions already use for another terminal emulator. Change it here.
pub const LINUX_PACKAGE_NAME: &str = "terminator-workspace";

/// GitHub repository that hosts release assets.
pub const RELEASE_REPO: &str = "aiman2039/terminator";

/// The folder a `.deb`, `.rpm` or AUR package installs the GUI, daemon and
/// hook helper into, side by side.
#[must_use]
pub fn package_dir() -> String {
    format!("/usr/lib/{LINUX_PACKAGE_NAME}")
}

/// True when `exe` was installed by a Linux package manager (`.deb`,
/// `.rpm` or AUR): that install updates through the package manager only.
#[must_use]
pub fn package_managed(exe: &Path) -> bool {
    exe.starts_with(package_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_package_folder_is_package_managed() {
        assert!(package_managed(Path::new(
            "/usr/lib/terminator-workspace/terminator"
        )));
        assert!(!package_managed(Path::new("/opt/terminator/terminator")));
        assert!(!package_managed(Path::new(
            "/usr/lib/terminator-workspace-old/terminator"
        )));
    }
}
