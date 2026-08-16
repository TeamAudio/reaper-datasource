//! CSV/TSV support. A delimited file is treated as a workbook with a single
//! sheet named after the file, with no hidden rows — configuring a CSV is
//! the same as configuring a one-worksheet spreadsheet without enrichments.

use crate::parse::SheetData;
use std::path::Path;

pub fn sheet_name(file: &Path) -> String {
    file.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| "Sheet1".to_owned())
}

pub fn read(file: &Path) -> Result<SheetData, String> {
    let bytes = std::fs::read(file).map_err(|error| format!("could not read file: {error}"))?;
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
    let text = String::from_utf8_lossy(bytes);
    let delimiter = delimiter_for(file, &text);

    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut data = Vec::new();
    let mut column_count = 0;
    for record in reader.records() {
        let record = record.map_err(|error| format!("could not parse file: {error}"))?;
        let row: Vec<String> = record.iter().map(str::to_owned).collect();
        column_count = column_count.max(row.len());
        data.push(row);
    }
    for row in &mut data {
        row.resize(column_count, String::new());
    }
    Ok(SheetData {
        name: sheet_name(file),
        data,
        hidden_rows: Vec::new(),
    })
}

fn delimiter_for(file: &Path, text: &str) -> u8 {
    let is_tsv = file
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("tsv"));
    if is_tsv {
        b'\t'
    } else {
        sniff_delimiter(text.lines().next().unwrap_or(""))
    }
}

/// Picks the most frequent candidate delimiter outside quoted sections of
/// the first line. Semicolon-delimited "CSV" is common (Excel exports in
/// locales where comma is the decimal separator), as are tab-delimited
/// files with a .csv extension. Comma wins ties.
fn sniff_delimiter(first_line: &str) -> u8 {
    let mut counts = [(b',', 0usize), (b';', 0), (b'\t', 0)];
    let mut in_quotes = false;
    for byte in first_line.bytes() {
        if byte == b'"' {
            in_quotes = !in_quotes;
        } else if !in_quotes {
            for (delimiter, count) in &mut counts {
                if byte == *delimiter {
                    *count += 1;
                }
            }
        }
    }
    counts
        .iter()
        .max_by_key(|(_, count)| *count)
        .filter(|(_, count)| *count > 0)
        .map(|(delimiter, _)| *delimiter)
        .unwrap_or(b',')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{list_sheets, parse_workbook, ParseOptions};
    use serde_json::json;

    fn write_fixture(name: &str, content: &[u8]) -> std::path::PathBuf {
        // The file stem becomes the sheet name, so uniqueness goes in the
        // directory rather than the file name.
        let dir = std::env::temp_dir().join(format!(
            "reaper-datasource-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    fn parse_fixture(name: &str, content: &[u8]) -> serde_json::Value {
        let path = write_fixture(name, content);
        let parsed = parse_workbook(path.to_str().unwrap(), &ParseOptions::default()).unwrap();
        std::fs::remove_file(&path).ok();
        parsed
    }

    #[test]
    fn parses_csv_as_single_sheet_workbook() {
        let parsed = parse_fixture("basic.csv", b"Header A,Header B\nhello,42\nworld,2.5\n");
        assert_eq!(parsed["config"]["file_type"], "csv");
        assert_eq!(parsed["config"]["sheets"], 1);
        let sheet = &parsed["sheets"][0];
        assert_eq!(sheet["config"]["name"], "basic");
        assert_eq!(sheet["config"]["column_count"], 2);
        assert_eq!(sheet["config"]["row_count"], 3);
        assert_eq!(sheet["config"]["hidden_rows"], json!([]));
        assert_eq!(
            sheet["data"],
            json!([
                ["Header A", "Header B"],
                ["hello", "42"],
                ["world", "2.5"],
            ])
        );
    }

    #[test]
    fn handles_quoting_bom_and_ragged_rows() {
        let parsed = parse_fixture(
            "tricky.csv",
            b"\xef\xbb\xbfname,\"note, quoted\"\n\"multi\nline\",x,extra\nshort\n",
        );
        let sheet = &parsed["sheets"][0];
        assert_eq!(sheet["config"]["column_count"], 3);
        assert_eq!(
            sheet["data"],
            json!([
                ["name", "note, quoted", ""],
                ["multi\nline", "x", "extra"],
                ["short", "", ""],
            ])
        );
    }

    #[test]
    fn sniffs_semicolon_and_tab_delimiters() {
        let parsed = parse_fixture("euro.csv", b"a;b;c\n1;2;3\n");
        assert_eq!(parsed["sheets"][0]["data"][0], json!(["a", "b", "c"]));

        let parsed = parse_fixture("tabbed.tsv", b"a\tb\n1,5\t2\n");
        assert_eq!(
            parsed["sheets"][0]["data"],
            json!([["a", "b"], ["1,5", "2"]])
        );
    }

    #[test]
    fn sheet_filter_applies_to_the_single_sheet() {
        let path = write_fixture("filtered.csv", b"a,b\n");
        let matching = ParseOptions {
            sheets: Some(vec!["filtered".into()]),
        };
        let parsed = parse_workbook(path.to_str().unwrap(), &matching).unwrap();
        assert_eq!(parsed["config"]["sheets"], 1);

        let missing = ParseOptions {
            sheets: Some(vec!["Alpha".into()]),
        };
        let error = parse_workbook(path.to_str().unwrap(), &missing).unwrap_err();
        std::fs::remove_file(&path).ok();
        assert!(error.contains("no sheet named \"Alpha\""), "{error}");
    }

    #[test]
    fn lists_the_single_sheet() {
        let path = write_fixture("listing.csv", b"a,b\n");
        let listed = list_sheets(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(
            listed,
            json!({"sheets": [{"name": "listing", "visible": true}]})
        );
    }

    #[test]
    fn empty_file_yields_empty_sheet() {
        let parsed = parse_fixture("empty.csv", b"");
        let sheet = &parsed["sheets"][0];
        assert_eq!(sheet["config"]["row_count"], 0);
        assert_eq!(sheet["config"]["column_count"], 0);
        assert_eq!(sheet["data"], json!([]));
    }

    #[test]
    fn delimiter_sniffing_ignores_quoted_sections() {
        assert_eq!(sniff_delimiter("a,b,c"), b',');
        assert_eq!(sniff_delimiter("a;b;c"), b';');
        assert_eq!(sniff_delimiter("a\tb\tc"), b'\t');
        assert_eq!(sniff_delimiter("\"x;y;z\",b"), b',');
        assert_eq!(sniff_delimiter("plain"), b',');
        assert_eq!(sniff_delimiter("a,b;c,d"), b',');
    }
}
