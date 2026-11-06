//! Checks for project-relative paths and upload file names.
use super::RemoteError;

pub fn validate_relative(path: &str) -> Result<(), RemoteError> {
    let valid = path.len() <= 4096
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && !path.split('/').any(|part| part == ".." || part == ".");
    if !valid {
        return Err(RemoteError::InvalidRelativePath {
            path: path.to_owned(),
        });
    }
    Ok(())
}

/// A file name an upload may create: one path component, no controls.
pub fn validate_file_name(name: &str) -> Result<(), RemoteError> {
    let valid = !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.chars().any(char::is_control);
    if !valid {
        return Err(RemoteError::InvalidFileName {
            name: name.to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_names_are_single_safe_components() {
        for name in ["", ".", "..", "a/b", "a\\b", "a\nb", &"x".repeat(256)] {
            assert!(validate_file_name(name).is_err(), "{name:?}");
        }
        assert!(validate_file_name("report final.pdf").is_ok());
        assert!(validate_file_name(".env").is_ok());
    }
}
