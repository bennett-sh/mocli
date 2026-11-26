//! The version resource of a program or library.

use std::{ffi::c_void, path::Path, ptr, slice};

use windows::{
    Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW},
    core::HSTRING,
};

#[derive(Debug, Default)]
pub struct VersionInfo {
    /// The friendly name, such as "Google Chrome".
    pub description: Option<String>,
    pub product: Option<String>,
    pub version: Option<String>,
    pub company: Option<String>,
}

/// `None` if the file has no version resource.
pub fn read(path: &Path) -> Option<VersionInfo> {
    let path = HSTRING::from(path);
    // SAFETY: the block is as large as Windows asks for, and every pointer
    // `VerQueryValueW` returns points into it.
    unsafe {
        let size = GetFileVersionInfoSizeW(&path, None);
        if size == 0 {
            return None;
        }
        let mut block = vec![0u8; size as usize];
        GetFileVersionInfoW(&path, None, size, block.as_mut_ptr().cast()).ok()?;

        let query = |key: &str| {
            let mut value: *mut c_void = ptr::null_mut();
            let mut length = 0;
            let found = VerQueryValueW(
                block.as_ptr().cast(),
                &HSTRING::from(key),
                &mut value,
                &mut length,
            );
            (found.as_bool() && !value.is_null() && length > 0)
                .then_some((value.cast::<u16>(), length as usize))
        };

        // The first language and code page pair; its length is in bytes.
        let (translation, bytes) = query(r"\VarFileInfo\Translation")?;
        if bytes < 4 {
            return None;
        }
        let language = translation.read_unaligned();
        let code_page = translation.add(1).read_unaligned();
        let string = |name: &str| {
            // The length of a string value is in characters.
            let (text, length) = query(&format!(
                r"\StringFileInfo\{language:04x}{code_page:04x}\{name}"
            ))?;
            let text = String::from_utf16_lossy(slice::from_raw_parts(text, length));
            let text = text.trim_matches(['\0', ' ']);
            (!text.is_empty()).then(|| text.to_owned())
        };

        Some(VersionInfo {
            description: string("FileDescription"),
            product: string("ProductName"),
            version: string("ProductVersion").or_else(|| string("FileVersion")),
            company: string("CompanyName"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_system_program() {
        let windows = std::env::var("SystemRoot").unwrap();
        let info = read(&Path::new(&windows).join("explorer.exe")).unwrap();
        let name = info.description.unwrap();
        assert!(!name.contains('\0'), "{name:?}");
        assert!(info.version.is_some() && info.company.is_some());
        assert!(read(Path::new(r"C:\does\not\exist.exe")).is_none());
    }
}
