//! Opens the bundled API documentation in the default browser.
//!
//! ReaPack installs the doc file into REAPER's Data directory; manual
//! installs may not have it, in which case the online project page opens
//! instead.

use std::ffi::CStr;
use std::path::PathBuf;
use std::process::Command;

const DOC_FILE: &str = "reaper_datasource_doc.html";
const FALLBACK_URL: &str = "https://github.com/TeamAudio/reaper-datasource#readme";

/// Opens the docs and returns the location opened (path or URL).
pub fn open() -> Result<String, String> {
    let target = target_for(resource_path());
    open_in_default_browser(&target)?;
    Ok(target)
}

fn target_for(resource_path: Option<PathBuf>) -> String {
    resource_path
        .map(|root| root.join("Data").join(DOC_FILE))
        .filter(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| FALLBACK_URL.to_owned())
}

fn resource_path() -> Option<PathBuf> {
    let low = crate::api::low()?;
    let ptr = unsafe { low.GetResourcePath() };
    if ptr.is_null() {
        return None;
    }
    let path = unsafe { CStr::from_ptr(ptr) }.to_str().ok()?;
    Some(PathBuf::from(path))
}

fn open_in_default_browser(target: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let (program, args) = ("open", vec![target]);
    #[cfg(target_os = "windows")]
    let (program, args) = ("cmd", vec!["/C", "start", "", target]);
    #[cfg(all(unix, not(target_os = "macos")))]
    let (program, args) = ("xdg-open", vec![target]);

    Command::new(program)
        .args(&args)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not open a browser: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_the_project_page() {
        assert_eq!(target_for(None), FALLBACK_URL);
        let empty = std::env::temp_dir().join("reaper-datasource-no-docs");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(target_for(Some(empty)), FALLBACK_URL);
    }

    #[test]
    fn prefers_the_installed_doc_file() {
        let root = std::env::temp_dir().join(format!(
            "reaper-datasource-docs-test-{}",
            std::process::id()
        ));
        let data = root.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        let doc = data.join(DOC_FILE);
        std::fs::write(&doc, "<html></html>").unwrap();
        assert_eq!(target_for(Some(root.clone())), doc.to_string_lossy());
        std::fs::remove_dir_all(&root).ok();
    }
}
