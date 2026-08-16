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

### Handle-based querying

For incremental access — previews, single columns, reactive UIs — open a
source once and query it without materializing the whole file. Sheets
load lazily on first access and stay cached on the handle.

```lua
local src, err = reaper.DataSource_Open("/path/to/book.xlsx")
if not src then
  reaper.ShowConsoleMsg("open failed: " .. err .. "\n")
  return
end
local ok, info = reaper.DataSource_GetInfo(src)      -- names/visibility, no cell data
local ok, cfg  = reaper.DataSource_GetSheetConfig(src, "Sheet1")  -- dims + hidden_rows
local ok, rows = reaper.DataSource_GetRows(src, "Sheet1", 1, 20)  -- JSON array of arrays
local ok, cell = reaper.DataSource_GetCell(src, "Sheet1", 2, 3)   -- plain string
reaper.DataSource_Close(src)
```

- Row/column arguments are 1-based and absolute, aligned with
  `hidden_rows`. `row_count <= 0` in `GetRows` means all remaining rows;
  cells beyond a sheet's extent read as `""`.
- An empty `sheet_name` selects the first sheet (convenient for CSV,
  whose one sheet is named after the file).
- Handles are opaque and validated on every call — a closed or bogus
  handle returns `(false, message)`, never a crash. They are **not**
  garbage-collected: call `DataSource_Close` when done (or
  `DataSource_CloseAll()` as an escape hatch; it returns the count
  closed).
- `reaper.DataSource_GetVersion()` returns the extension version, for
  capability checks.
- `reaper.DataSource_OpenDocs()` opens the bundled API reference
  (`Data/reaper_datasource_doc.html`, installed by ReaPack) in the default
  browser, falling back to the project page.

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
