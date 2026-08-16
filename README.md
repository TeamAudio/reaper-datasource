# reaper-datasource

Native tabular-data parsing for REAPER ReaScripts. A REAPER extension
plugin (built on [reaper-rs]) that reads spreadsheets with [calamine] and
delimited files with [csv], exposing the result to Lua/EEL/Python as
ReaScript API functions — no backend service, no temp files, no polling.

Version 1 scope: **tabular files on disk** — `.xlsx`, `.xlsm`, `.xlsb`,
`.xls`, `.ods`, `.csv`, `.tsv`. Everything is presented as sheets of rows
of string cells; a CSV is a workbook with a single sheet named after the
file.

[reaper-rs]: https://github.com/helgoboss/reaper-rs
[calamine]: https://github.com/tafia/calamine
[csv]: https://github.com/BurntSushi/rust-csv

## API

Functions appear in the `reaper` namespace once the extension is installed.
All calls are synchronous. Each returns a boolean alongside its result
string: on success the string is JSON, on failure it is an error message.

### `reaper.DataSource_Parse(file_path, options_json)`

```lua
local ok, result = reaper.DataSource_Parse("/path/to/book.xlsx", "")
if not ok then
  reaper.ShowConsoleMsg("parse failed: " .. result .. "\n")
  return
end
```

Parses a tabular file into JSON:

```json
{
  "config": { "file_type": "excel", "file_name": "book.xlsx", "sheets": 2 },
  "sheets": [
    {
      "config": {
        "name": "Sheet1",
        "column_count": 3,
        "row_count": 10,
        "hidden_rows": [4, 7],
        "bold_rows": []
      },
      "data": [["a", "b", "c"], ["", "1", "2.5"]]
    }
  ]
}
```

This is intentionally the same shape as the ReaSpeech backend's
`parse_spreadsheet` response, so existing consumers can adopt it without a
data-model change. Cells are strings ("" when empty); spreadsheet data is
anchored at cell A1, so a row's index is its absolute sheet row number and
`hidden_rows` (1-based) aligns with `data`. Integral numbers render
without a decimal point (`"42"`, not `"42.0"`).

Delimited files (`file_type: "csv"`) produce one sheet named after the
file, with no hidden rows. The delimiter is sniffed from the first line
(comma, semicolon, or tab; `.tsv` is always tab) and a UTF-8 BOM is
stripped.

`options_json` is optional. Supported keys:

- `sheets`: array of sheet names to parse (default: all sheets).

Hidden rows are read from the worksheet XML of OOXML archives
(`.xlsx`/`.xlsm`). Other formats parse fine but report no hidden rows.

### `reaper.DataSource_ListSheets(file_path)`

Returns sheet names and visibility without reading cell data:

```lua
local ok, result = reaper.DataSource_ListSheets("/path/to/book.xlsx")
-- result: { "sheets": [{ "name": "Sheet1", "visible": true }] }
```

### Result size

Results are returned through REAPER's `NeedBig` output-buffer convention
and grown with `realloc_cmd_ptr`, so ReaScript callers can receive
arbitrarily large JSON. Callers of the raw C API must provide their own
sufficiently large buffer; if the result does not fit, the call returns
false with an error message instead of truncated JSON.

## Building

```sh
cargo build --release
```

Install by copying the built library into REAPER's `UserPlugins` directory
with the platform's expected name:

| Platform | Built artifact | Install as |
| --- | --- | --- |
| macOS | `target/release/libreaper_datasource.dylib` | `reaper_datasource.dylib` |
| Windows | `target/release/reaper_datasource.dll` | `reaper_datasource.dll` |
| Linux | `target/release/libreaper_datasource.so` | `reaper_datasource.so` |

## Testing

```sh
cargo test
```

Tests cover the pure parsing layer (spreadsheet fixtures are generated
with `rust_xlsxwriter`) and the output-buffer handling, and run without
REAPER. For an in-REAPER smoke test, run `examples/datasource_demo.lua`
as a ReaScript; `cargo run --example make_fixture -- /tmp/sample.xlsx`
generates a workbook to try it with.
