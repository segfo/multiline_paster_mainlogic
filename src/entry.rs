use multiline_parser_pluginlib::result::*;
use windows::Win32::UI::WindowsAndMessaging::KBDLLHOOKSTRUCT;
#[no_mangle]
pub extern "C" fn key_down(keystate: u32, stroke_msg: KBDLLHOOKSTRUCT) -> PluginResult {
    crate::default::key_down(keystate, stroke_msg)
}

#[no_mangle]
pub extern "C" fn key_up(keystate: u32, stroke_msg: KBDLLHOOKSTRUCT) -> PluginResult {
    crate::default::key_up(keystate, stroke_msg)
}

#[no_mangle]
pub extern "C" fn init_plugin() {
    let (run_mode,config) = crate::config::init();
    if let Some(encoder_list) = config.text_encoders {
        crate::default::load_encoder(encoder_list);
    }
    crate::default::set_mode(run_mode);
}
