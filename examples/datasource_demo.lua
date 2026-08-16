-- Smoke test for the reaper_datasource extension.
-- Prompts for a tabular file, lists its sheets, and prints the parse result.

if not reaper.DataSource_Parse then
  reaper.ShowConsoleMsg("reaper_datasource extension is not loaded\n")
  return
end

local ok, path = reaper.GetUserFileNameForRead("", "Choose a spreadsheet or CSV", "")
if not ok then return end

local listed, sheets = reaper.DataSource_ListSheets(path)
if not listed then
  reaper.ShowConsoleMsg("DataSource_ListSheets error: " .. sheets .. "\n")
  return
end
reaper.ShowConsoleMsg("sheets: " .. sheets .. "\n")

local parsed, result = reaper.DataSource_Parse(path, "")
if not parsed then
  reaper.ShowConsoleMsg("DataSource_Parse error: " .. result .. "\n")
  return
end

reaper.ShowConsoleMsg(("parsed %d bytes of JSON\n"):format(#result))
reaper.ShowConsoleMsg(result:sub(1, 2000) .. "\n")
