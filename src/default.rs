use crate::config::*;
use multiline_parser_pluginlib::{plugin::*, result::*};
use once_cell::unsync::*;
use send_input::keyboard::windows::*;
use std::ffi::{CString, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::sync::{Arc, Condvar};
use std::time::Duration;
use std::{
    collections::VecDeque,
    sync::{Mutex, RwLock},
};
use toolbox::config_loader::ConfigLoader;
use windows::Win32::{
    Foundation::*,
    System::{DataExchange::*, Memory::*, SystemServices::*, WindowsProgramming::*},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
static mut clipboard: Lazy<Mutex<VecDeque<String>>> = Lazy::new(|| Mutex::new(VecDeque::new()));
static mut thread_mutex: Lazy<Mutex<u32>> = Lazy::new(|| Mutex::new(0));
static mut map: Lazy<RwLock<Vec<bool>>> = Lazy::new(|| RwLock::new(vec![false; 256]));
static mut g_mode: Lazy<RwLock<RunMode>> = Lazy::new(|| RwLock::new(RunMode::default()));
static mut TXT_MODIFIER: Lazy<RwLock<PluginManager>> = Lazy::new(|| {
    let conf: MasterConfig = ConfigLoader::load_file("config.toml");
    RwLock::new(PluginManager::new(&conf.plugin_directory))
});
const MAX_MODIFIER_PALETTES: usize = 9;
// クリップボード挿入モードか、DirectInputモードで動作するか選択できるようにする。
pub fn set_mode(mode: RunMode) {
    unsafe {
        let mut locked_gmode = g_mode.write().unwrap();
        *locked_gmode = mode;
    };
}

pub fn load_encoder(encoder_list: Vec<String>) {
    let mut pm = unsafe { TXT_MODIFIER.write().unwrap() };
    for encoder in &encoder_list {
        if encoder.len() == 0 {
            println!("🔥警告: モディファイアの設定に空白文字が指定されています。このモディファイアは読まれません。");
            continue;
        }
        if let Err(e) = pm.load_plugin(encoder) {
            println!("🔥警告: モディファイア \"{encoder}\" が読み込めませんでした。({e})");
            continue;
        }
        println!("📝情報： {} を読み込みました。", encoder);
    }
    println!("モディファイアの読み込みが完了しました。🎉");
    let palette_no = unsafe { g_mode.read().unwrap().get_palette_no() };
    println!("📝現在のパレットに存在するモディファイアは以下の通りです。");
    show_current_mod_palette(&mut pm, palette_no);
}

////
pub fn key_down(keystate: u32, stroke_msg: KBDLLHOOKSTRUCT) -> PluginResult {
    if stroke_msg.flags.0 & (LLKHF_INJECTED.0 | LLKHF_LOWER_IL_INJECTED.0) == 0
        || stroke_msg.dwExtraInfo == 0
    {
        // println!("[key down] stroke={stroke_msg:?}");
        let is_burst = unsafe {
            let mut lmap = map.write().unwrap();
            lmap[stroke_msg.vkCode as usize] = true;
            let mode = g_mode.read().unwrap();
            mode.is_burst_mode()
        };
        if judge_combo_key() != ComboKey::None && is_burst {
            return PluginResult::NoChain;
        }
    }
    PluginResult::Success
}

pub fn key_up(keystate: u32, stroke_msg: KBDLLHOOKSTRUCT) -> PluginResult {
    if stroke_msg.flags.0 & (LLKHF_INJECTED.0 | LLKHF_LOWER_IL_INJECTED.0) == 0
        || stroke_msg.dwExtraInfo == 0
    {
        // println!("[key up] stroke={stroke_msg:?}");
        unsafe {
            let mut lmap = map.write().unwrap();
            lmap[stroke_msg.vkCode as usize] = false;
        }
    }
    PluginResult::Success
}

async fn undo_clipboard() {
    show_operation_message("クリップボードに対するアンドゥ");
    let mut cb = unsafe { clipboard.lock().unwrap() };
    cb.pop_front();
    println!("アンドゥ後のクリップボード内データ行数: {}行", cb.len());
}

async fn copy_clipboard() {
    show_operation_message("コピー");
    // WindowsがCTRL+Cして、クリップボードにデータを格納するまで待機する。
    let wait = unsafe { g_mode.read().unwrap().get_copy_wait_millis() };
    std::thread::sleep(Duration::from_millis(wait));
    let mut cb = unsafe { clipboard.lock().unwrap() };
    let iclip = Clipboard::open();
    unsafe {
        load_data_from_clipboard(&mut *cb);
    }
}

async fn reset_clipboard() {
    show_operation_message("クリップボードデータの削除");
    let mut cb = unsafe { clipboard.lock().unwrap() };
    cb.clear();
}
pub struct Clipboard {}
impl Clipboard {
    fn open() -> Self {
        unsafe {
            OpenClipboard(HWND::default());
        }
        Clipboard {}
    }
}
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

fn show_operation_message<T: Into<String>>(operation: T) {
    let active_window = unsafe { GetForegroundWindow() };
    if active_window.0 != 0 {
        println!(
            "ウィンドウ「{}」上で{}操作が行われました。",
            get_window_text(active_window),
            operation.into()
        );
    } else {
        println!("アクティブウィンドウに対するフォーカスが失われています。");
    }
}

#[derive(PartialEq, Debug)]
enum ComboKey {
    None,
    Combo(u64),
}
fn show_current_mod_palette(pm: &mut PluginManager, palette_no: usize) {
    let plugin_list = pm.get_plugin_ordered_list().clone();
    let current_palette_max = palette_no * MAX_MODIFIER_PALETTES + MAX_MODIFIER_PALETTES;
    let current_palette_min = palette_no * MAX_MODIFIER_PALETTES;
    let plugin_list_len = plugin_list.len();
    let current_palette_max = if plugin_list_len < current_palette_max {
        plugin_list_len
    } else {
        current_palette_max
    };
    let current_palette = &plugin_list[current_palette_min..current_palette_max];

    for (slot_no, plugin_name) in current_palette.iter().enumerate() {
        let (about, state) = plugin_about(pm, plugin_name);
        println!(
            "[{}] {plugin_name} {about} ({})",
            slot_no + 1,
            if state == PluginActivateState::Activate {
                "有効"
            } else {
                "無効"
            }
        );
    }
}

fn judge_combo_key() -> ComboKey {
    let lmap = unsafe { &mut map.read().unwrap() };
    // 0xA2:CTRL
    if lmap[0xA2] == true {
        let hook_mode = {
            let mode = unsafe { &mut g_mode.write().unwrap() };
            mode.get_hook_mode()
        };
        // CTRL+ALTキー
        if lmap[VK_LMENU.0 as usize] | lmap[VK_RMENU.0 as usize] {
            if lmap[0x30] {
                let mode = unsafe { &mut g_mode.write().unwrap() };
                let hook_mode = mode.get_hook_mode();
                if hook_mode == HookMode::Override {
                    mode.set_hook_mode(HookMode::OsStandard);
                    println!("コピー・ペーストに関するホットキーをOSの既定動作に戻します。");
                } else if hook_mode == HookMode::OsStandard {
                    mode.set_hook_mode(HookMode::Override);
                    println!("コピー・ペーストに関するホットキーを有効化しました。");
                }
            }
            // HookMode::OsStandard時は、CTRL+ALT+0以外を全て無効化する。
            if hook_mode == HookMode::OsStandard {
                return ComboKey::None;
            }
            if lmap[0x43] || lmap[0x58] {
                async_std::task::spawn(reset_clipboard());
                return ComboKey::Combo(3);
            }
            if lmap[0x5A] {
                // Z
                async_std::task::spawn(undo_clipboard());
                return ComboKey::Combo(0);
            }
            // 1-9キーのどれか
            for vk in 0x31..=0x39 {
                if lmap[vk] {
                    // 初期パレットは0
                    let palette_no = unsafe { &mut g_mode.read().unwrap().get_palette_no() };
                    dbg!(&palette_no);
                    let mut pm = unsafe { TXT_MODIFIER.write().unwrap() };
                    let key = vk - 0x31;
                    let key = MAX_MODIFIER_PALETTES * (*palette_no) + key;
                    let state = pm.get_plugin_activate_state_with_order(key);
                    if let Some((plugin_name, state)) = state {
                        let state = if state == PluginActivateState::Activate {
                            PluginActivateState::Disable
                        } else {
                            PluginActivateState::Activate
                        };
                        let result = pm.set_plugin_activate_state_with_order(key, state);
                        let s = match result {
                            Some(s) => {
                                if s == PluginActivateState::Activate {
                                    "が有効化されました"
                                } else {
                                    "が無効化されました"
                                }
                            }
                            None => "はロードされていません",
                        };
                        println!("モディファイア \"{plugin_name}\" {s}");
                    };
                }
            }
            if lmap['Q' as usize] {
                let mut pm = unsafe { TXT_MODIFIER.write().unwrap() };
                // 最大パレット番号
                let max_palette_count = (pm.loaded_plugin_counts()) / MAX_MODIFIER_PALETTES; // 9はキーボードの1-9の意味
                let mode = unsafe { &mut g_mode.write().unwrap() };
                let palette_no = mode.get_palette_no();
                // パレット番号は0-max_palette_countまでを取る。
                if lmap[VK_LSHIFT.0 as usize] {
                    let palette_no = if usize::MIN == palette_no {
                        max_palette_count
                    } else {
                        (palette_no - 1) % (max_palette_count + 1)
                    };
                    mode.set_palette_no(palette_no);
                    println!("前に戻る");
                } else {
                    mode.set_palette_no((palette_no + 1) % (max_palette_count + 1));
                    println!("先に進む");
                }
                println!("📝パレット番号が {} に切り替わりました", palette_no);
                println!("📝現在のパレットに存在するモディファイアは以下のとおりです");
                let palette_no = mode.get_palette_no();
                show_current_mod_palette(&mut pm, palette_no);
            }
            return ComboKey::Combo(4);
        }
        // HookMode::OsStandard時は、CTRL+ALT+0以外を全て無効化する。
        if hook_mode == HookMode::OsStandard {
            return ComboKey::None;
        }
        if lmap[0x43] || lmap[0x58] {
            // 0x43:C
            // 0x58:X
            async_std::task::spawn(copy_clipboard());
            return ComboKey::Combo(2);
        }
        if lmap[0x56] {
            // 0x56: V
            // 基本的に重たい操作なので非同期で行う
            // 意訳：さっさとフックプロシージャから復帰しないとキーボードがハングする。
            // ただし、Clipboardをロックしてから戻らないとだめ。
            let cb_lock_wait = Arc::new((Mutex::new(false), Condvar::new()));
            async_std::task::spawn(paste(cb_lock_wait.clone()));
            let (lock, _cond) = &*cb_lock_wait;
            lock.lock().unwrap(); // クリップボードがロックされるまで待つ。
            return ComboKey::Combo(1);
        }
    }
    ComboKey::None
}

pub async fn paste(is_clipboard_locked: Arc<(Mutex<bool>, Condvar)>) {
    let mutex = unsafe { thread_mutex.lock().unwrap() };
    let input_mode = unsafe {
        // DropTraitを有効にするために変数に束縛する
        // 束縛先の変数は未使用だが、最適化によってOpenClipboardが実行されなくなるので変数束縛は必ず行う。
        // ここでクリップボードを開いている理由は、CTRL+VによってWindowsがショートカットに反応してペーストしないようにロックする意図がある。
        // タイミングによってはロックできないので、条件変数を使用してメインスレッドを待機させておく。
        // ロックが完了した瞬間にnotify_oneをする必要がある。可能な限り早く実施する。
        // ロックするまでの間にsleepはもちろんのこと、MutexLock/RwLockなどの重たい処理を行ってはならない。
        let (lock, cond) = &*is_clipboard_locked;
        let iclip = Clipboard::open();
        let mut is_lock = lock.lock().unwrap();
        *is_lock = true;
        cond.notify_one();
        // クリップボードを開く
        let mut cb = clipboard.lock().unwrap();
        EmptyClipboard();
        if cb.len() == 0 {
            println!("クリップボードにデータがありません。");
            return;
        }
        // オプションをロードする
        let (is_burst_mode, tabindex_keyseq, get_line_delay_msec, char_delay_msec, input_mode) = {
            let mode = g_mode.read().unwrap();
            (
                mode.is_burst_mode(),
                mode.get_tabindex_keyseq(),
                mode.get_line_delay_msec(),
                mode.get_char_delay_msec(),
                mode.get_input_mode(),
            )
        };

        if is_burst_mode {
            let mut kbd = Keyboard::new();
            let len = cb.len();
            kbd.new_delay(char_delay_msec);
            kbd.append_input_chain(
                KeycodeBuilder::default()
                    .vk(VK_LCONTROL.0)
                    .scan_code(virtual_key_to_scancode(VK_LCONTROL))
                    .build(),
            );
            for key in tabindex_keyseq.chars() {
                KeycodeBuilder::default()
                    .char_build(key)
                    .iter()
                    .for_each(|keycode| kbd.append_input_chain(keycode.clone()));
            }
            for _i in 0..len {
                paste_impl(&mut cb);
                kbd.send_key();
                // キーストロークとの間に数ミリ秒の待機時間を設ける
                std::thread::sleep(Duration::from_millis(get_line_delay_msec))
            }
        } else {
            paste_impl(&mut cb);
        }
        let wait = g_mode.read().unwrap().get_copy_wait_millis();
        std::thread::sleep(Duration::from_millis(wait));
        input_mode
    };
    // Clipboard以外ならキー入力は行わない。
    if input_mode == InputMode::DirectKeyInput {
        return;
    }
    // クリップボードモードなら
    // 強制的にペーストさせる。
    let mut kbd = Keyboard::new();
    kbd.append_input_chain(
        KeycodeBuilder::default()
            .vk(VK_LCONTROL.0)
            .scan_code(virtual_key_to_scancode(VK_LCONTROL))
            .key_send_mode(KeySendMode::KeyDown)
            .build(),
    );
    KeycodeBuilder::default()
        .char_build('v')
        .iter()
        .for_each(|key_code| kbd.append_input_chain(key_code.clone()));
    kbd.append_input_chain(
        KeycodeBuilder::default()
            .vk(VK_LCONTROL.0)
            .scan_code(virtual_key_to_scancode(VK_LCONTROL))
            .key_send_mode(KeySendMode::KeyUp)
            .build(),
    );
    kbd.send_key();
}

unsafe fn load_data_from_clipboard(cb: &mut VecDeque<String>) -> Option<()> {
    let h_text = GetClipboardData(CF_UNICODETEXT.0);
    let line_len_max = unsafe { g_mode.read().unwrap().get_max_line_len() };
    match h_text {
        Err(_) => None,
        Ok(h_text) => {
            // クリップボードにデータがあったらロックする
            let p_text = GlobalLock(h_text.0);
            // 今クリップボードにある内容をコピーする（改行で分割される）
            // 後でここの挙動を変えても良さそう。
            let text = u16_ptr_to_string(p_text as *const _).into_string().unwrap();
            let current_len = cb.len();
            for line in text.lines() {
                let line_len = line.len();
                if line_len != 0 {
                    if line_len_max > 0 && line_len >= line_len_max {
                        println!("1行が長過ぎる文字列({}文字以上の行)をコピーしようとしたため、当該行はスキップしました。",line_len_max);
                        continue;
                    }
                    cb.push_front(line.to_owned());
                } else {
                    cb.push_front("".to_owned());
                }
            }
            GlobalUnlock(h_text.0);
            println!(
                "クリップボードから {} 行コピーしました",
                cb.len() - current_len
            );
            Some(())
        }
    }
}

type EncodeFunc = unsafe extern "C" fn(*const u8, usize) -> EncodedString;
unsafe fn paste_impl(cb: &mut VecDeque<String>) {
    let s = cb.pop_back().unwrap();
    // Encoderモディファイア（仮）を呼び出す。
    let s = unsafe {
        let pm = TXT_MODIFIER.read().unwrap();
        let func_list =
            pm.get_all_plugin_func_with_order::<EncodeFunc>("do_encode", CallOrder::Asc);

        let mut encoded = CString::new(s.clone()).unwrap().to_bytes().to_vec();
        for f in func_list {
            let e = f(encoded.as_ptr(), encoded.len());
            encoded = e.to_vec();
        }
        match String::from_utf8(encoded) {
            Ok(s) => s,
            Err(e) => {
                println!("🔥警告: モディファイアによるエンコードに失敗したため、ロールバックします（返却値がUTF-8文字列ではありません / {e}）");
                s
            }
        }
    };
    let (input_mode, char_delay_msec) = {
        let mode = g_mode.read().unwrap();
        (mode.get_input_mode(), mode.get_char_delay_msec())
    };

    show_operation_message("ペースト");
    if input_mode == InputMode::DirectKeyInput {
        let is_key_pressed = |vk: usize| -> bool {
            let lmap = map.read().unwrap();
            lmap[vk]
        };
        // 現在のキーボードの状況（KeyboardLLHookから取得した状況）に合わせて制御キーの解除と設定を行う。
        // その後に、ペースト対象のデータを送る
        // さらに、現在のキーボードの状況に合わせて今度は制御キーを復旧させる。
        let mut kbd = Keyboard::new();
        // CTRLキーを一旦解除する
        kbd.new_delay(char_delay_msec);
        kbd.append_input_chain(
            KeycodeBuilder::default()
                .vk(VK_LCONTROL.0)
                .scan_code(virtual_key_to_scancode(VK_LCONTROL))
                .build(),
        );
        // ペースト対象の文字列を登録する
        for c in s.as_str().chars() {
            KeycodeBuilder::default()
                .char_build(char::from_u32(c as u32).unwrap())
                .iter()
                .for_each(|key_code| kbd.append_input_chain(key_code.clone()));
        }
        kbd.send_key();
        kbd.clear_input_chain();
        // CTRLキーが押されている状況をチェックしてチェーンに登録する
        let mode = if is_key_pressed(162) {
            KeySendMode::KeyDown
        } else {
            KeySendMode::KeyUp
        };
        kbd.append_input_chain(
            KeycodeBuilder::default()
                .vk(VK_LCONTROL.0)
                .scan_code(virtual_key_to_scancode(VK_LCONTROL))
                .key_send_mode(mode)
                .build(),
        );
        kbd.send_key();
    } else {
        let data = OsString::from(s).encode_wide().collect::<Vec<u16>>();
        let strdata_len = data.len() * 2;
        let data_ptr = data.as_ptr();
        let gdata = GlobalAlloc(GHND | GLOBAL_ALLOC_FLAGS(GMEM_SHARE), strdata_len + 2);
        let locked_data = GlobalLock(gdata);
        std::ptr::copy_nonoverlapping(
            data_ptr as *const u8,
            locked_data as *mut u8,
            strdata_len + 2,
        );
        match SetClipboardData(CF_UNICODETEXT.0, HANDLE(gdata)) {
            Ok(_handle) => {
                println!("set clipboard success.");
            }
            Err(e) => {
                println!("SetClipboardData failed. {:?}", e);
            }
        }
        // 終わったらアンロックしてからメモリを開放する
        GlobalUnlock(gdata);
        GlobalFree(gdata);
    }
}

fn virtual_key_to_scancode(vk: VIRTUAL_KEY) -> u16 {
    unsafe { MapVirtualKeyA(vk.0 as u32, MAPVK_VK_TO_VSC as u32) as u16 }
}

fn get_window_text(hwnd: HWND) -> String {
    unsafe {
        // GetWindowTextLengthW + GetWindowTextWは別プロセスへの取得を意図したものではないとの記述がMSDNにあるので
        // SendMessageWで取得することにする。
        let len = SendMessageW(hwnd, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0)).0 as usize + 1;
        let mut buf = vec![0u16; len];
        SendMessageW(
            hwnd,
            WM_GETTEXT,
            WPARAM(len),
            LPARAM(buf.as_mut_ptr() as isize),
        );
        OsString::from_wide(&buf[0..buf.len() - 1])
            .to_os_string()
            .into_string()
            .unwrap()
    }
}

unsafe fn u16_ptr_to_string(ptr: *const u16) -> OsString {
    let len = (0..).take_while(|&i| *ptr.offset(i) != 0).count();
    let slice = std::slice::from_raw_parts(ptr, len);
    OsString::from_wide(slice)
}
