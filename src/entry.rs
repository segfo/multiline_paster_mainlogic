use std::sync::Mutex;

use multiline_parser_pluginlib::result::*;
use once_cell::sync::Lazy;
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
    println!("🟢  起動しました。");
    if let Some(encoder_list) = config.text_modifiers {
        crate::default::load_encoder(encoder_list);
    }
    crate::default::set_mode(run_mode);
}

static mut about_string: Lazy<Mutex<Vec<u8>>> = Lazy::new(|| Mutex::new(Vec::new()));
#[no_mangle]
pub extern "C" fn about()->EncodedString {
    let mut s=unsafe{about_string.lock().unwrap()};
    *s="メインロジックDLL".as_bytes().to_vec();
    EncodedString::new(s.as_ptr(), s.len())
}

#[no_mangle]
extern "C" fn update_clipboard(){
    crate::default::update_clipboard();
}