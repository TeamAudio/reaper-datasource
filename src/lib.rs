mod api;
mod delimited;
mod hidden;
mod parse;
mod source;

use reaper_low::PluginContext;
use reaper_macros::reaper_extension_plugin;
use reaper_medium::ReaperSession;
use std::error::Error;

#[reaper_extension_plugin]
fn plugin_main(context: PluginContext) -> Result<(), Box<dyn Error>> {
    let mut session = Box::new(ReaperSession::load(context));
    api::init(*session.reaper().low());

    unsafe {
        session.plugin_register_add_api_and_def(
            "DataSource_Parse",
            api::parse_native as *mut _,
            api::parse_vararg,
            "bool",
            "const char*,const char*,char*,int",
            "file_path,options_jsonOptional,resultOutNeedBig,resultOutNeedBig_sz",
            "Parses a tabular file (.xlsx/.xlsm/.xlsb/.xls/.ods/.csv/.tsv) into result as JSON. Returns false with an error message in result on failure.",
        )?;
        session.plugin_register_add_api_and_def(
            "DataSource_ListSheets",
            api::list_sheets_native as *mut _,
            api::list_sheets_vararg,
            "bool",
            "const char*,char*,int",
            "file_path,resultOutNeedBig,resultOutNeedBig_sz",
            "Lists the sheet names and visibility of a tabular file into result as JSON. Returns false with an error message in result on failure.",
        )?;
        session.plugin_register_add_api_and_def(
            "DataSource_Open",
            api::open_native as *mut _,
            api::open_vararg,
            "DataSource*",
            "const char*,char*,int",
            "file_path,errorOutOptional,errorOutOptional_sz",
            "Opens a tabular file for incremental querying; sheets load lazily and stay cached. Returns nil with a message in error on failure. Call DataSource_Close when done; handles are not garbage-collected.",
        )?;
        session.plugin_register_add_api_and_def(
            "DataSource_Close",
            api::close_native as *mut _,
            api::close_vararg,
            "bool",
            "DataSource*",
            "source",
            "Closes an open data source and frees its cached sheets. Returns false if the handle was not open.",
        )?;
        session.plugin_register_add_api_and_def(
            "DataSource_CloseAll",
            api::close_all_native as *mut _,
            api::close_all_vararg,
            "int",
            "",
            "",
            "Closes every open data source and returns how many were closed.",
        )?;
        session.plugin_register_add_api_and_def(
            "DataSource_GetInfo",
            api::get_info_native as *mut _,
            api::get_info_vararg,
            "bool",
            "DataSource*,char*,int",
            "source,resultOutNeedBig,resultOutNeedBig_sz",
            "File name, file type, and sheet names/visibility of an open data source, as JSON, without reading cell data. Returns false with an error message in result on failure.",
        )?;
        session.plugin_register_add_api_and_def(
            "DataSource_GetSheetConfig",
            api::get_sheet_config_native as *mut _,
            api::get_sheet_config_vararg,
            "bool",
            "DataSource*,const char*,char*,int",
            "source,sheet_name,resultOutNeedBig,resultOutNeedBig_sz",
            "Dimensions and hidden rows of one sheet as JSON. An empty sheet_name selects the first sheet. Returns false with an error message in result on failure.",
        )?;
        session.plugin_register_add_api_and_def(
            "DataSource_GetRows",
            api::get_rows_native as *mut _,
            api::get_rows_vararg,
            "bool",
            "DataSource*,const char*,int,int,char*,int",
            "source,sheet_name,start_row,row_count,resultOutNeedBig,resultOutNeedBig_sz",
            "Rows start_row..start_row+row_count-1 (1-based) of one sheet as a JSON array of arrays; row_count <= 0 means all remaining rows. An empty sheet_name selects the first sheet. Returns false with an error message in result on failure.",
        )?;
        session.plugin_register_add_api_and_def(
            "DataSource_GetCell",
            api::get_cell_native as *mut _,
            api::get_cell_vararg,
            "bool",
            "DataSource*,const char*,int,int,char*,int",
            "source,sheet_name,row,column,resultOutNeedBig,resultOutNeedBig_sz",
            "One cell's string value (1-based row and column; \"\" beyond the sheet's extent). An empty sheet_name selects the first sheet. Returns false with an error message in result on failure.",
        )?;
        session.plugin_register_add_api_and_def(
            "DataSource_GetVersion",
            api::get_version_native as *mut _,
            api::get_version_vararg,
            "bool",
            "char*,int",
            "versionOut,versionOut_sz",
            "The reaper-datasource extension version.",
        )?;
    }

    Box::leak(session);
    Ok(())
}
