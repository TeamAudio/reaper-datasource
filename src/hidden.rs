//! Best-effort extraction of hidden row numbers from xlsx/xlsm archives.
//!
//! calamine does not surface row visibility, but consumers use hidden rows to
//! decide which lines to import, so we read the row `hidden` attributes from
//! the worksheet XML directly. Formats that are not OOXML zip archives
//! (.xls, .xlsb, .ods) yield no hidden rows, matching the previous backend's
//! behavior of silently degrading when styling could not be read.

use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use zip::ZipArchive;

/// Returns 1-based hidden row numbers per sheet name. Never fails: any
/// structural surprise yields an empty map (or skips the affected sheet).
pub fn hidden_rows_by_sheet(path: &str) -> HashMap<String, Vec<u32>> {
    try_hidden_rows(path).unwrap_or_default()
}

fn try_hidden_rows(path: &str) -> Option<HashMap<String, Vec<u32>>> {
    let file = File::open(path).ok()?;
    let mut archive = ZipArchive::new(BufReader::new(file)).ok()?;
    let workbook_xml = read_entry(&mut archive, "xl/workbook.xml")?;
    let rels_xml = read_entry(&mut archive, "xl/_rels/workbook.xml.rels")?;
    let relationship_targets = relationship_targets(&rels_xml);

    let mut result = HashMap::new();
    for (sheet_name, relationship_id) in sheet_relationships(&workbook_xml) {
        let Some(target) = relationship_targets.get(&relationship_id) else {
            continue;
        };
        let Some(sheet_xml) = read_entry(&mut archive, &normalize_target(target)) else {
            continue;
        };
        result.insert(sheet_name, hidden_rows_in_sheet(&sheet_xml));
    }
    Some(result)
}

fn read_entry(archive: &mut ZipArchive<BufReader<File>>, name: &str) -> Option<String> {
    let mut entry = archive.by_name(name).ok()?;
    let mut content = String::new();
    entry.read_to_string(&mut content).ok()?;
    Some(content)
}

/// Parses `<sheet name="..." r:id="rIdN"/>` elements from xl/workbook.xml.
fn sheet_relationships(workbook_xml: &str) -> Vec<(String, String)> {
    let mut reader = Reader::from_str(workbook_xml);
    let mut sheets = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                if element.local_name().as_ref() != b"sheet" {
                    continue;
                }
                let mut name = None;
                let mut relationship_id = None;
                for attribute in element.attributes().flatten() {
                    let key = attribute.key.as_ref();
                    if key == b"name" {
                        name = attribute_value(&attribute);
                    } else if key == b"r:id" || key.ends_with(b":id") {
                        relationship_id = attribute_value(&attribute);
                    }
                }
                if let (Some(name), Some(relationship_id)) = (name, relationship_id) {
                    sheets.push((name, relationship_id));
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    sheets
}

/// Parses `<Relationship Id="..." Target="..."/>` elements from
/// xl/_rels/workbook.xml.rels, keeping only worksheet relationships.
fn relationship_targets(rels_xml: &str) -> HashMap<String, String> {
    let mut reader = Reader::from_str(rels_xml);
    let mut targets = HashMap::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                if element.local_name().as_ref() != b"Relationship" {
                    continue;
                }
                let mut id = None;
                let mut target = None;
                let mut is_worksheet = false;
                for attribute in element.attributes().flatten() {
                    match attribute.key.as_ref() {
                        b"Id" => id = attribute_value(&attribute),
                        b"Target" => target = attribute_value(&attribute),
                        b"Type" => {
                            is_worksheet = attribute
                                .value
                                .as_ref()
                                .ends_with(b"/worksheet");
                        }
                        _ => {}
                    }
                }
                if is_worksheet {
                    if let (Some(id), Some(target)) = (id, target) {
                        targets.insert(id, target);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    targets
}

/// Relationship targets are relative to xl/ unless they start with "/".
fn normalize_target(target: &str) -> String {
    if let Some(absolute) = target.strip_prefix('/') {
        absolute.to_owned()
    } else {
        format!("xl/{target}")
    }
}

/// Scans worksheet XML for `<row r="N" hidden="1">`. The `r` attribute is
/// optional in OOXML; rows without it continue from the previous row number.
fn hidden_rows_in_sheet(sheet_xml: &str) -> Vec<u32> {
    let mut reader = Reader::from_str(sheet_xml);
    let mut hidden = Vec::new();
    let mut next_row: u32 = 1;
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                if element.local_name().as_ref() != b"row" {
                    continue;
                }
                let mut row_number = next_row;
                let mut is_hidden = false;
                for attribute in element.attributes().flatten() {
                    match attribute.key.as_ref() {
                        b"r" => {
                            if let Some(value) =
                                attribute_value(&attribute).and_then(|v| v.parse().ok())
                            {
                                row_number = value;
                            }
                        }
                        b"hidden" => {
                            is_hidden =
                                matches!(attribute.value.as_ref(), b"1" | b"true");
                        }
                        _ => {}
                    }
                }
                next_row = row_number + 1;
                if is_hidden {
                    hidden.push(row_number);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    hidden
}

fn attribute_value(attribute: &quick_xml::events::attributes::Attribute) -> Option<String> {
    std::str::from_utf8(attribute.value.as_ref())
        .ok()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_hidden_rows_with_and_without_row_numbers() {
        let sheet_xml = r#"<worksheet><sheetData>
            <row r="1"><c r="A1"/></row>
            <row r="3" hidden="1"><c r="A3"/></row>
            <row hidden="true"><c/></row>
            <row r="10" hidden="0"><c/></row>
        </sheetData></worksheet>"#;
        assert_eq!(hidden_rows_in_sheet(sheet_xml), vec![3, 4]);
    }

    #[test]
    fn maps_sheet_names_through_relationships() {
        let workbook_xml = r#"<workbook>
            <sheets>
                <sheet name="First" sheetId="1" r:id="rId1"/>
                <sheet name="Second" sheetId="2" r:id="rId2"/>
            </sheets>
        </workbook>"#;
        assert_eq!(
            sheet_relationships(workbook_xml),
            vec![
                ("First".to_owned(), "rId1".to_owned()),
                ("Second".to_owned(), "rId2".to_owned()),
            ]
        );

        let rels_xml = r#"<Relationships>
            <Relationship Id="rId1" Type="http://x/worksheet" Target="worksheets/sheet1.xml"/>
            <Relationship Id="rId2" Type="http://x/worksheet" Target="/xl/worksheets/sheet2.xml"/>
            <Relationship Id="rId3" Type="http://x/styles" Target="styles.xml"/>
        </Relationships>"#;
        let targets = relationship_targets(rels_xml);
        assert_eq!(targets.len(), 2);
        assert_eq!(
            normalize_target(&targets["rId1"]),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(
            normalize_target(&targets["rId2"]),
            "xl/worksheets/sheet2.xml"
        );
    }

    #[test]
    fn non_zip_input_yields_no_hidden_rows() {
        assert!(hidden_rows_by_sheet("/nonexistent/nope.xls").is_empty());
    }
}
