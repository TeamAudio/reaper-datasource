use calamine::{open_workbook_auto, Data, Reader, SheetVisible};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ParseOptions {
    /// Restrict parsing to these sheet names; all sheets when omitted.
    #[serde(default)]
    pub sheets: Option<Vec<String>>,
}

/// One parsed worksheet: a dense rectangle of string cells. A CSV/TSV file
/// produces exactly one, named after the file.
pub struct SheetData {
    pub name: String,
    pub data: Vec<Vec<String>>,
    pub hidden_rows: Vec<u32>,
}

/// Parses a tabular file into the same JSON shape the ReaSpeech backend's
/// parse_spreadsheet endpoint produces, so existing consumers can switch over
/// without a data-model change:
///
/// { "config": { "file_type", "file_name", "sheets" },
///   "sheets": [ { "config": { "name", "column_count", "row_count",
///                             "hidden_rows", "bold_rows" },
///                 "data": [ [ cell, ... ], ... ] } ] }
///
/// Cells are strings; empty cells are "". Spreadsheet data is anchored at
/// cell A1 so row indices are absolute sheet row numbers (1-based in
/// hidden_rows), keeping row-based locators stable across re-parses.
pub fn parse_workbook(path: &str, options: &ParseOptions) -> Result<Value, String> {
    let file = Path::new(path);
    if !file.is_file() {
        return Err("file_path does not name a readable file".into());
    }
    let (file_type, sheets) = if is_delimited(file) {
        ("csv", filter_single(crate::delimited::read(file)?, options)?)
    } else {
        ("excel", read_excel(path, options)?)
    };
    Ok(json!({
        "config": {
            "file_type": file_type,
            "file_name": file
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            "sheets": sheets.len(),
        },
        "sheets": sheets.iter().map(sheet_json).collect::<Vec<_>>(),
    }))
}

pub fn list_sheets(path: &str) -> Result<Value, String> {
    let file = Path::new(path);
    if !file.is_file() {
        return Err("file_path does not name a readable file".into());
    }
    if is_delimited(file) {
        return Ok(json!({"sheets": [
            {"name": crate::delimited::sheet_name(file), "visible": true},
        ]}));
    }
    let workbook =
        open_workbook_auto(path).map_err(|error| format!("could not open workbook: {error}"))?;
    let sheets = workbook
        .sheets_metadata()
        .iter()
        .map(|sheet| {
            json!({
                "name": sheet.name,
                "visible": matches!(sheet.visible, SheetVisible::Visible),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "sheets": sheets }))
}

pub fn is_delimited(file: &Path) -> bool {
    file.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("csv") || extension.eq_ignore_ascii_case("tsv")
        })
}

fn filter_single(sheet: SheetData, options: &ParseOptions) -> Result<Vec<SheetData>, String> {
    match &options.sheets {
        None => Ok(vec![sheet]),
        Some(wanted) => {
            for name in wanted {
                if name != &sheet.name {
                    return Err(format!("no sheet named {name:?} in workbook"));
                }
            }
            Ok(if wanted.is_empty() { vec![] } else { vec![sheet] })
        }
    }
}

fn read_excel(path: &str, options: &ParseOptions) -> Result<Vec<SheetData>, String> {
    let mut workbook =
        open_workbook_auto(path).map_err(|error| format!("could not open workbook: {error}"))?;
    let sheet_names = workbook.sheet_names().to_owned();
    let selected = match &options.sheets {
        Some(wanted) => {
            for name in wanted {
                if !sheet_names.contains(name) {
                    return Err(format!("no sheet named {name:?} in workbook"));
                }
            }
            wanted.clone()
        }
        None => sheet_names,
    };
    let hidden = crate::hidden::hidden_rows_by_sheet(path);

    let mut sheets = Vec::with_capacity(selected.len());
    for name in &selected {
        sheets.push(load_sheet(&mut workbook, name, &hidden)?);
    }
    Ok(sheets)
}

/// Reads one worksheet from an open workbook into a dense A1-anchored
/// rectangle of string cells.
pub fn load_sheet(
    workbook: &mut calamine::Sheets<std::io::BufReader<std::fs::File>>,
    name: &str,
    hidden: &std::collections::HashMap<String, Vec<u32>>,
) -> Result<SheetData, String> {
    let range = workbook
        .worksheet_range(name)
        .map_err(|error| format!("could not read sheet {name:?}: {error}"))?;
    let (row_count, column_count) = match range.end() {
        Some((row, column)) => (row as usize + 1, column as usize + 1),
        None => (0, 0),
    };
    let mut data = Vec::with_capacity(row_count);
    for row in 0..row_count {
        let mut cells = Vec::with_capacity(column_count);
        for column in 0..column_count {
            let cell = range.get_value((row as u32, column as u32));
            cells.push(cell.map(cell_to_string).unwrap_or_default());
        }
        data.push(cells);
    }
    Ok(SheetData {
        name: name.to_owned(),
        data,
        hidden_rows: hidden.get(name).cloned().unwrap_or_default(),
    })
}

fn sheet_json(sheet: &SheetData) -> Value {
    json!({
        "config": sheet_config_json(sheet),
        "data": sheet.data,
    })
}

pub fn sheet_config_json(sheet: &SheetData) -> Value {
    json!({
        "name": sheet.name,
        "column_count": sheet.data.first().map(Vec::len).unwrap_or(0),
        "row_count": sheet.data.len(),
        "hidden_rows": sheet.hidden_rows,
        "bold_rows": [],
    })
}

fn cell_to_string(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(value) => value.clone(),
        Data::Float(value) => float_to_string(*value),
        Data::Int(value) => value.to_string(),
        Data::Bool(value) => if *value { "TRUE" } else { "FALSE" }.to_owned(),
        Data::DateTime(value) => value
            .as_datetime()
            .map(|datetime| datetime.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| value.as_f64().to_string()),
        Data::DateTimeIso(value) => value.clone(),
        Data::DurationIso(value) => value.clone(),
        Data::Error(error) => error.to_string(),
    }
}

/// Integral floats render without a decimal point ("42", not "42.0"): Excel
/// stores all numbers as doubles, and "42.0" for a cell displaying 42 was a
/// long-standing artifact of the pandas-based parser.
fn float_to_string(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_xlsxwriter::Workbook;

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("reaper-datasource-test-{}-{name}", std::process::id()))
    }

    fn write_fixture(name: &str) -> std::path::PathBuf {
        let path = fixture_path(name);
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
    fn parses_workbook_into_backend_shape() {
        let path = write_fixture("shape.xlsx");
        let parsed = parse_workbook(path.to_str().unwrap(), &ParseOptions::default()).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(parsed["config"]["file_type"], "excel");
        assert_eq!(parsed["config"]["sheets"], 2);
        let alpha = &parsed["sheets"][0];
        assert_eq!(alpha["config"]["name"], "Alpha");
        assert_eq!(alpha["config"]["column_count"], 2);
        assert_eq!(alpha["config"]["row_count"], 3);
        assert_eq!(alpha["config"]["hidden_rows"], json!([3]));
        assert_eq!(alpha["config"]["bold_rows"], json!([]));
        assert_eq!(
            alpha["data"],
            json!([
                ["Header A", "Header B"],
                ["hello", "42"],
                ["world", "2.5"],
            ])
        );
        let beta = &parsed["sheets"][1];
        assert_eq!(beta["config"]["name"], "Beta");
        assert_eq!(beta["data"], json!([["TRUE"]]));
    }

    #[test]
    fn sheet_filter_selects_and_validates() {
        let path = write_fixture("filter.xlsx");
        let options = ParseOptions {
            sheets: Some(vec!["Beta".into()]),
        };
        let parsed = parse_workbook(path.to_str().unwrap(), &options).unwrap();
        assert_eq!(parsed["config"]["sheets"], 1);
        assert_eq!(parsed["sheets"][0]["config"]["name"], "Beta");

        let missing = ParseOptions {
            sheets: Some(vec!["Gamma".into()]),
        };
        let error = parse_workbook(path.to_str().unwrap(), &missing).unwrap_err();
        std::fs::remove_file(&path).ok();
        assert!(error.contains("no sheet named \"Gamma\""), "{error}");
    }

    #[test]
    fn lists_sheets_with_visibility() {
        let path = write_fixture("list.xlsx");
        let listed = list_sheets(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(
            listed,
            json!({"sheets": [
                {"name": "Alpha", "visible": true},
                {"name": "Beta", "visible": true},
            ]})
        );
    }

    #[test]
    fn missing_file_is_an_error() {
        let error = parse_workbook("/nonexistent/nope.xlsx", &ParseOptions::default());
        assert_eq!(
            error.unwrap_err(),
            "file_path does not name a readable file"
        );
    }

    #[test]
    fn options_reject_unknown_fields() {
        let error = serde_json::from_str::<ParseOptions>(r#"{"sheet":"Alpha"}"#).unwrap_err();
        assert!(error.to_string().contains("unknown field `sheet`"));
    }

    #[test]
    fn cells_stringify_like_spreadsheet_display() {
        assert_eq!(cell_to_string(&Data::Empty), "");
        assert_eq!(cell_to_string(&Data::String("text".into())), "text");
        assert_eq!(cell_to_string(&Data::Float(42.0)), "42");
        assert_eq!(cell_to_string(&Data::Float(2.5)), "2.5");
        assert_eq!(cell_to_string(&Data::Float(-0.0)), "0");
        assert_eq!(cell_to_string(&Data::Int(7)), "7");
        assert_eq!(cell_to_string(&Data::Bool(true)), "TRUE");
        assert_eq!(cell_to_string(&Data::Bool(false)), "FALSE");
        assert_eq!(
            cell_to_string(&Data::DateTimeIso("2026-01-02T03:04:05".into())),
            "2026-01-02T03:04:05"
        );
    }
}
