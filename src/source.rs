//! Handle-based data sources: an opened file whose sheets load lazily and
//! stay cached for incremental querying, without re-parsing or materializing
//! the whole workbook.
//!
//! Handles given to scripts are registry keys, not real pointers — every
//! call looks the value up and never dereferences it, so a stale or bogus
//! handle produces an error instead of a crash. REAPER offers no
//! garbage-collection hook for custom pointer types, so scripts must call
//! close (or close_all) to release memory.

use crate::delimited;
use crate::parse::{self, SheetData};
use calamine::{open_workbook_auto, Reader, SheetVisible, Sheets};
use serde_json::json;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

pub struct Source {
    path: PathBuf,
    file_type: &'static str,
    /// (name, visible) per sheet, in workbook order.
    sheets: Vec<(String, bool)>,
    hidden: HashMap<String, Vec<u32>>,
    cache: HashMap<String, SheetData>,
    /// None for delimited files, which re-read from path on first query.
    workbook: Option<Sheets<BufReader<File>>>,
}

static SOURCES: OnceLock<Mutex<HashMap<u64, Source>>> = OnceLock::new();
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

fn sources() -> MutexGuard<'static, HashMap<u64, Source>> {
    SOURCES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn open(path: &str) -> Result<u64, String> {
    let file = Path::new(path);
    if !file.is_file() {
        return Err("file_path does not name a readable file".into());
    }
    let source = if parse::is_delimited(file) {
        Source {
            path: file.to_owned(),
            file_type: "csv",
            sheets: vec![(delimited::sheet_name(file), true)],
            hidden: HashMap::new(),
            cache: HashMap::new(),
            workbook: None,
        }
    } else {
        let workbook = open_workbook_auto(path)
            .map_err(|error| format!("could not open workbook: {error}"))?;
        let sheets = workbook
            .sheets_metadata()
            .iter()
            .map(|sheet| {
                (
                    sheet.name.clone(),
                    matches!(sheet.visible, SheetVisible::Visible),
                )
            })
            .collect();
        Source {
            path: file.to_owned(),
            file_type: "excel",
            sheets,
            hidden: crate::hidden::hidden_rows_by_sheet(path),
            cache: HashMap::new(),
            workbook: Some(workbook),
        }
    };
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    sources().insert(handle, source);
    Ok(handle)
}

pub fn close(handle: u64) -> bool {
    sources().remove(&handle).is_some()
}

pub fn close_all() -> usize {
    let mut sources = sources();
    let count = sources.len();
    sources.clear();
    count
}

fn with_source<T>(
    handle: u64,
    work: impl FnOnce(&mut Source) -> Result<T, String>,
) -> Result<T, String> {
    let mut sources = sources();
    let source = sources
        .get_mut(&handle)
        .ok_or_else(|| "source is not an open data source handle".to_owned())?;
    work(source)
}

impl Source {
    /// Resolves a sheet name ("" selects the first sheet) and loads it into
    /// the cache on first access.
    fn sheet(&mut self, name: &str) -> Result<&SheetData, String> {
        let resolved = if name.is_empty() {
            self.sheets
                .first()
                .map(|(sheet_name, _)| sheet_name.clone())
                .ok_or_else(|| "the file has no sheets".to_owned())?
        } else {
            name.to_owned()
        };
        if !self.sheets.iter().any(|(sheet_name, _)| *sheet_name == resolved) {
            return Err(format!("no sheet named {resolved:?} in workbook"));
        }
        if !self.cache.contains_key(&resolved) {
            let data = match &mut self.workbook {
                Some(workbook) => parse::load_sheet(workbook, &resolved, &self.hidden)?,
                None => delimited::read(&self.path)?,
            };
            self.cache.insert(resolved.clone(), data);
        }
        Ok(&self.cache[&resolved])
    }
}

pub fn info_json(handle: u64) -> Result<String, String> {
    with_source(handle, |source| {
        Ok(json!({
            "file_name": source
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            "file_type": source.file_type,
            "sheets": source
                .sheets
                .iter()
                .map(|(name, visible)| json!({"name": name, "visible": visible}))
                .collect::<Vec<_>>(),
        })
        .to_string())
    })
}

pub fn sheet_config_json(handle: u64, sheet_name: &str) -> Result<String, String> {
    with_source(handle, |source| {
        Ok(parse::sheet_config_json(source.sheet(sheet_name)?).to_string())
    })
}

/// Rows start_row..start_row+row_count as a JSON array of arrays. start_row
/// is 1-based and absolute (aligned with hidden_rows); row_count <= 0 means
/// all remaining rows. Rows beyond the sheet yield an empty array.
pub fn rows_json(
    handle: u64,
    sheet_name: &str,
    start_row: i32,
    row_count: i32,
) -> Result<String, String> {
    if start_row < 1 {
        return Err("start_row must be 1 or greater".into());
    }
    with_source(handle, |source| {
        let data = &source.sheet(sheet_name)?.data;
        let begin = (start_row as usize - 1).min(data.len());
        let end = if row_count <= 0 {
            data.len()
        } else {
            begin.saturating_add(row_count as usize).min(data.len())
        };
        serde_json::to_string(&data[begin..end])
            .map_err(|error| format!("could not serialize rows: {error}"))
    })
}

/// A single cell's string value, 1-based. Cells beyond the sheet's extent
/// are "", matching how a spreadsheet treats unset cells.
pub fn cell(handle: u64, sheet_name: &str, row: i32, column: i32) -> Result<String, String> {
    if row < 1 || column < 1 {
        return Err("row and column must be 1 or greater".into());
    }
    with_source(handle, |source| {
        let data = &source.sheet(sheet_name)?.data;
        Ok(data
            .get(row as usize - 1)
            .and_then(|cells| cells.get(column as usize - 1))
            .cloned()
            .unwrap_or_default())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_xlsxwriter::Workbook;
    use serde_json::Value;

    fn fixture_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "reaper-datasource-source-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_workbook(name: &str) -> PathBuf {
        let path = fixture_dir().join(name);
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet().set_name("Alpha").unwrap();
        sheet.write(0, 0, "Header A").unwrap();
        sheet.write(0, 1, "Header B").unwrap();
        sheet.write(1, 0, "hello").unwrap();
        sheet.write(1, 1, 42.0).unwrap();
        sheet.write(2, 0, "world").unwrap();
        sheet.write(2, 1, 2.5).unwrap();
        sheet.set_row_hidden(2).unwrap();
        let sheet2 = workbook.add_worksheet().set_name("Beta").unwrap();
        sheet2.write(0, 0, true).unwrap();
        workbook.save(&path).unwrap();
        path
    }

    #[test]
    fn opens_queries_and_closes_a_workbook() {
        let path = write_workbook("roundtrip.xlsx");
        let handle = open(path.to_str().unwrap()).unwrap();

        let info: Value = serde_json::from_str(&info_json(handle).unwrap()).unwrap();
        assert_eq!(info["file_type"], "excel");
        assert_eq!(info["sheets"][0]["name"], "Alpha");
        assert_eq!(info["sheets"][1]["name"], "Beta");

        let config: Value =
            serde_json::from_str(&sheet_config_json(handle, "Alpha").unwrap()).unwrap();
        assert_eq!(config["row_count"], 3);
        assert_eq!(config["column_count"], 2);
        assert_eq!(config["hidden_rows"], serde_json::json!([3]));

        assert_eq!(
            rows_json(handle, "Alpha", 2, 1).unwrap(),
            r#"[["hello","42"]]"#
        );
        assert_eq!(
            rows_json(handle, "Alpha", 2, 0).unwrap(),
            r#"[["hello","42"],["world","2.5"]]"#
        );
        assert_eq!(rows_json(handle, "Alpha", 9, 5).unwrap(), "[]");
        assert_eq!(cell(handle, "Alpha", 2, 2).unwrap(), "42");
        assert_eq!(cell(handle, "Alpha", 200, 2).unwrap(), "");
        // "" selects the first sheet
        assert_eq!(cell(handle, "", 1, 1).unwrap(), "Header A");

        assert!(close(handle));
        assert!(!close(handle), "closing twice reports false");
        let error = info_json(handle).unwrap_err();
        assert_eq!(error, "source is not an open data source handle");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn opens_csv_as_single_sheet_source() {
        let path = fixture_dir().join("source.csv");
        std::fs::write(&path, b"a,b\n1,2\n").unwrap();
        let handle = open(path.to_str().unwrap()).unwrap();

        let info: Value = serde_json::from_str(&info_json(handle).unwrap()).unwrap();
        assert_eq!(info["file_type"], "csv");
        assert_eq!(info["sheets"], serde_json::json!([{"name": "source", "visible": true}]));
        assert_eq!(cell(handle, "source", 2, 1).unwrap(), "1");
        assert_eq!(
            cell(handle, "Alpha", 1, 1).unwrap_err(),
            "no sheet named \"Alpha\" in workbook"
        );

        assert!(close(handle));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn validates_arguments_and_handles() {
        assert_eq!(
            open("/nonexistent/nope.xlsx").unwrap_err(),
            "file_path does not name a readable file"
        );
        assert_eq!(
            info_json(u64::MAX).unwrap_err(),
            "source is not an open data source handle"
        );
        assert_eq!(
            rows_json(u64::MAX, "", 0, 1).unwrap_err(),
            "start_row must be 1 or greater"
        );
        assert_eq!(
            cell(u64::MAX, "", 1, 0).unwrap_err(),
            "row and column must be 1 or greater"
        );
    }
}
