//! What the disk says about a copied file, read when it is selected.

use std::{fs, path::Path, path::PathBuf, time::UNIX_EPOCH};

use crate::platform::version::{self, VersionInfo};

#[derive(Debug, Default)]
pub struct FileFacts {
    pub path: PathBuf,
    pub exists: bool,
    pub is_folder: bool,
    /// Milliseconds since the Unix epoch.
    pub modified: Option<i64>,
    /// The size of a picture, in pixels.
    pub dimensions: Option<(u32, u32)>,
    pub version: Option<VersionInfo>,
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Whether the file can be drawn as a picture.
pub fn is_picture(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "svg"
    )
}

pub fn read(path: &str) -> FileFacts {
    let path = PathBuf::from(path);
    // Network shares can block for seconds when unreachable.
    if path.to_string_lossy().starts_with(r"\\") {
        return FileFacts {
            path,
            exists: true,
            ..FileFacts::default()
        };
    }
    let Ok(metadata) = fs::metadata(&path) else {
        return FileFacts {
            path,
            ..FileFacts::default()
        };
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as i64);
    let dimensions = (metadata.is_file() && is_picture(&path))
        .then(|| image::image_dimensions(&path).ok())
        .flatten();
    let version = matches!(extension(&path).as_str(), "exe" | "dll")
        .then(|| version::read(&path))
        .flatten();
    FileFacts {
        exists: true,
        is_folder: metadata.is_dir(),
        modified,
        dimensions,
        version,
        path,
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI exports a `test` attribute of its own.
    use super::{is_picture, read};
    use std::path::Path;

    #[test]
    fn describes_programs_folders_and_missing_files() {
        let windows = std::env::var("SystemRoot").unwrap();
        let program = read(&format!(r"{windows}\explorer.exe"));
        assert!(program.exists && program.modified.is_some() && program.version.is_some());
        assert!(read(&windows).is_folder);
        assert!(!read(r"C:\does\not\exist.png").exists);
        assert!(is_picture(Path::new("a.PNG")) && !is_picture(Path::new("a.txt")));
    }
}
