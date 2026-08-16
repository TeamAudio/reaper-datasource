mod api;
mod delimited;
mod hidden;
mod parse;

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
    }

    Box::leak(session);
    Ok(())
}
