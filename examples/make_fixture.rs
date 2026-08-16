//! Generates a sample workbook for trying the extension:
//! `cargo run --example make_fixture -- /tmp/sample.xlsx`
//!
//! Pass row/column counts to generate a large workbook for stress-testing
//! the output-buffer growth path:
//! `cargo run --example make_fixture -- /tmp/big.xlsx 20000 8`

use rust_xlsxwriter::Workbook;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: make_fixture <output.xlsx> [rows] [columns]");
    let rows: u32 = args.next().map(|arg| arg.parse().unwrap()).unwrap_or(0);
    let columns: u16 = args.next().map(|arg| arg.parse().unwrap()).unwrap_or(8);

    let mut workbook = Workbook::new();
    if rows == 0 {
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
    } else {
        let sheet = workbook.add_worksheet().set_name("Stress").unwrap();
        for row in 0..rows {
            for column in 0..columns {
                sheet
                    .write(row, column, format!("r{row}c{column} lorem ipsum dolor"))
                    .unwrap();
            }
        }
    }
    workbook.save(&path).unwrap();
    println!("wrote {path}");
}
