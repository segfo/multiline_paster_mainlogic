use ::windows::Win32::UI::WindowsAndMessaging::KBDLLHOOKSTRUCT;
use multiline_parser_pluginlib::result::*;
use notify::event::{DataChange, ModifyKind};
use once_cell::sync::Lazy;
use std::path::Path;
use std::sync::Mutex;
use toolbox::config_loader::ConfigLoader;
#[no_mangle]
pub extern "C" fn key_down(keystate: u32, stroke_msg: KBDLLHOOKSTRUCT) -> PluginResult {
    crate::default::key_down(keystate, stroke_msg)
}

#[no_mangle]
pub extern "C" fn key_up(keystate: u32, stroke_msg: KBDLLHOOKSTRUCT) -> PluginResult {
    crate::default::key_up(keystate, stroke_msg)
}
use crate::config::get_config_path;
use crate::default::{get_mode, set_mode};
use notify::*;
static mut EVENT_CHATTER: Lazy<Mutex<usize>> = Lazy::new(|| Mutex::new(0));
static mut CONFIG_WATCHER: Lazy<Mutex<ReadDirectoryChangesWatcher>> = Lazy::new(|| {
    Mutex::new(
        notify::recommended_watcher(|res: std::result::Result<Event, Error>| match res {
            Ok(event) => {
                let config_path = get_config_path();
                let conf = std::fs::canonicalize(Path::new(&config_path)).unwrap();
                let mut do_reload = false;
                match event.kind {
                    EventKind::Modify(ModifyKind::Any) => {
                        for path in event.paths {
                            if path.cmp(&conf) == std::cmp::Ordering::Equal {
                                do_reload = true;
                            }
                        }
                    }
                    t => {
                        println!("{t:?}");
                    }
                }
                if do_reload {
                    // ファイル変更イベントはアホみたいに来るので、最後の1個だけ処理するようにする。
                    // チャタリング対策というやつ
                    unsafe {
                        let mut chatter_cnt = EVENT_CHATTER.lock().unwrap();
                        *chatter_cnt += 1;
                    }
                    async_std::task::spawn(wait_flush());
                }
            }
            Err(e) => {
                eprintln!("❌  設定ファイルの監視/リロードに失敗しました。\n{e:?}");
            }
        })
        .unwrap(),
    )
});
// ファイルがフラッシュされるであろう時まで待つ
// 100msも待てばまぁ良いでしょう。
async fn wait_flush() {
    let wait_time = 100;
    std::thread::sleep(std::time::Duration::from_millis(wait_time));
    unsafe {
        let mut chatter_cnt = EVENT_CHATTER.lock().unwrap();
        *chatter_cnt -= 1;
        if *chatter_cnt == 0 {
            // 100ms（チャタリング判定時間）以内に到達したイベントの一番最後なので設定ファイルをロードする
            let mut mode = get_mode();
            let config: crate::config::Config = ConfigLoader::load_file(&get_config_path());
            mode.set_config(config.clone());
            set_mode(mode);
            println!("🔄  設定ファイルをリロードしました。");
            if config.text_modifiers_hot_reload {
                if let Some(encoder_list) = config.text_modifiers {
                    println!("🔄  モディファイアモジュールをホットリロードし、モディファイアの状態を初期化しました。");
                    crate::default::load_encoder(encoder_list);
                }
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn init_plugin() {
    let (run_mode, config) = crate::config::init();
    println!("🟢  起動しました。");
    if let Some(encoder_list) = config.text_modifiers {
        crate::default::load_encoder(encoder_list);
    }
    crate::default::set_mode(run_mode);
    let config_path = get_config_path();
    let p = std::fs::canonicalize(Path::new(&config_path)).unwrap();
    let mut watcher = unsafe { CONFIG_WATCHER.lock().unwrap() };
    watcher
        .watch(p.as_path(), RecursiveMode::NonRecursive)
        .unwrap();
}

static mut ABOUT_STRING: Lazy<Mutex<Vec<u8>>> = Lazy::new(|| Mutex::new(Vec::new()));
#[no_mangle]
pub extern "C" fn about() -> EncodedString {
    let mut s = unsafe { ABOUT_STRING.lock().unwrap() };
    *s = "メインロジックDLL".as_bytes().to_vec();
    EncodedString::new(s.as_ptr(), s.len())
}

#[no_mangle]
extern "C" fn update_clipboard() {
    crate::default::update_clipboard();
}
