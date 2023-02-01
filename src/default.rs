use crate::config::*;
use multiline_parser_pluginlib::{plugin::*, result::*};
use once_cell::unsync::*;
use send_input::keyboard::windows::*;
use std::ffi::{CString, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::sync::{Arc, Condvar};
use std::time::{Duration, Instant};
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

// ALTキーが押されているかどうかのステート
enum EhKeyState {
    None,
    Alt,
}

struct ClipboardData {
    data: VecDeque<String>,
    copied_lines: Vec<usize>,
    add_line_count: usize,
}
impl ClipboardData {
    pub fn new() -> Self {
        ClipboardData {
            data: VecDeque::new(),
            copied_lines: Vec::new(),
            add_line_count: 0,
        }
    }
    pub fn pop_back(&mut self) -> Option<String> {
        self.data.pop_back()
    }
    pub fn commit_copy_lines(&mut self) {
        self.copied_lines.push(self.add_line_count);
        self.add_line_count = 0;
    }
    pub fn add_clipboard(&mut self, data: String) {
        self.data.push_front(data);
        self.add_line_count += 1;
    }
    pub fn clipboard_clear(&mut self) {
        self.data.clear()
    }
    pub fn get_clipboard_lines(&self) -> usize {
        self.data.len()
    }
    pub fn undo_data(&mut self) -> usize {
        let lines = self.copied_lines.len();
        if lines == 0 {
            return 0;
        }
        self.remove_data(self.copied_lines[lines - 1])
    }
    pub fn remove_data(&mut self, delete_count: usize) -> usize {
        let data_total = self.data.len();

        if data_total == 0 {
            return 0;
        }
        let mut actual_total_deletes = 0;
        for i in 0..delete_count {
            if i < data_total {
                self.data.pop_front();
                actual_total_deletes += 1;
            } else {
                break;
            }
        }
        let e = self.copied_lines.len();
        let mut total_deletes = actual_total_deletes;
        for _i in 0..e {
            let lines = self.copied_lines.pop().unwrap();
            if lines <= total_deletes {
                total_deletes -= lines
            } else {
                self.copied_lines.push(lines - total_deletes);
            };
        }
        actual_total_deletes
    }
}

static mut CLIPBOARD: Lazy<Mutex<ClipboardData>> = Lazy::new(|| Mutex::new(ClipboardData::new()));
static mut THREAD_MUTEX: Lazy<Mutex<u32>> = Lazy::new(|| Mutex::new(0));
static mut KEY_MAP: Lazy<RwLock<Vec<bool>>> = Lazy::new(|| RwLock::new(vec![false; 256]));
static mut RUN_MODE: Lazy<RwLock<RunMode>> = Lazy::new(|| RwLock::new(RunMode::default()));
static mut TXT_MODIFIER: Lazy<RwLock<PluginManager>> = Lazy::new(|| {
    let conf: MasterConfig = ConfigLoader::load_file("config.toml");
    RwLock::new(PluginManager::new(&conf.plugin_directory))
});
// CTRLコンボキーのハンドラ
static mut EH_CTL: Lazy<RwLock<Vec<Box<dyn Fn(&Vec<bool>, EhKeyState) -> ComboKey>>>> =
    Lazy::new(|| RwLock::new(Vec::new()));
const MAX_MODIFIER_PALETTES: usize = 9;

// クリップボード挿入モードか、DirectInputモードで動作するか選択できるようにする。
pub fn set_mode(mode: RunMode) {
    unsafe {
        let mut locked_gmode = RUN_MODE.write().unwrap();
        *locked_gmode = mode;
    };
}
static mut CB_IN_COPY: Lazy<RwLock<bool>> = Lazy::new(|| RwLock::new(false));
pub fn update_clipboard() {
    let mut in_copy = unsafe { CB_IN_COPY.write().unwrap() };

    if *in_copy {
        #[cfg(debug_assertions)]
        println!("コピー操作によりclipboardが変更された。");
        async_std::task::spawn(copy_clipboard());
        *in_copy = false;
    } else {
        #[cfg(debug_assertions)]
        println!("その他操作によりclipboardが変更された");
    }
}
pub fn load_encoder(encoder_list: Vec<String>) {
    let mut pm = unsafe { TXT_MODIFIER.write().unwrap() };
    if encoder_list.len() == 0 {
        return;
    }
    for encoder in &encoder_list {
        if encoder.len() == 0 {
            println!("❌  モディファイアの設定に空白文字が指定されています。このモディファイアは読まれません。");
            continue;
        }
        if let Err(e) = pm.load_plugin(encoder) {
            println!("❌  モディファイア \"{encoder}\" が読み込めませんでした。({e})");
            continue;
        }
        println!("📘  {} を読み込みました。", encoder);
    }
    println!("🎉  モディファイアの読み込みが完了しました。");
    let palette_no = unsafe { RUN_MODE.read().unwrap().get_palette_no() };
    println!("📘  現在のパレット（{palette_no}番パレット）にセットされているモディファイアは以下の通りです。");
    show_current_mod_palette(&mut pm, palette_no);
}

static mut DLL: Lazy<Mutex<libloading::Library>> = Lazy::new(|| {
    Mutex::new(unsafe {
        match libloading::Library::new("ignore_key.dll") {
            Err(_e) => {
                println!("🔴  必須ライブラリ ignore_key.dll が読み込めませんでした。");
                std::process::exit(-1);
            }
            Ok(lib) => lib,
        }
    })
});

type DllCtrlNoticeApi = unsafe extern "C" fn() -> bool;
type DllSetHookApi = unsafe extern "C" fn() -> bool;
fn enable_ctrl_v() {
    let dll = unsafe { DLL.lock().unwrap() };
    // 有効化する
    unsafe {
        let notice_ctrl_v: libloading::Symbol<DllCtrlNoticeApi> =
            dll.get(b"notice_ctrl_v").unwrap();
        notice_ctrl_v();
    }
}
fn disable_ctrl_v() {
    let dll = unsafe { DLL.lock().unwrap() };
    // 無効化する
    unsafe {
        let ignore_ctrl_v: libloading::Symbol<DllCtrlNoticeApi> =
            dll.get(b"ignore_ctrl_v").unwrap();
        ignore_ctrl_v();
    }
}

pub fn key_down(_keystate: u32, stroke_msg: KBDLLHOOKSTRUCT) -> PluginResult {
    if stroke_msg.flags.0 & (LLKHF_INJECTED.0 | LLKHF_LOWER_IL_INJECTED.0) == 0
        || stroke_msg.dwExtraInfo == 0
    {
        // println!("[key down] stroke={stroke_msg:?}");
        let is_burst = unsafe {
            let mut lmap = KEY_MAP.write().unwrap();
            lmap[stroke_msg.vkCode as usize] = true;
            let mode = RUN_MODE.read().unwrap();
            mode.is_burst_mode()
        };
        if judge_combo_key(stroke_msg.vkCode as usize) != ComboKey::None && is_burst {
            return PluginResult::NoChain;
        }
    }
    PluginResult::Success
}

pub fn key_up(_keystate: u32, stroke_msg: KBDLLHOOKSTRUCT) -> PluginResult {
    if stroke_msg.flags.0 & (LLKHF_INJECTED.0 | LLKHF_LOWER_IL_INJECTED.0) == 0
        || stroke_msg.dwExtraInfo == 0
    {
        // println!("[key up] stroke={stroke_msg:?}");
        unsafe {
            let mut lmap = KEY_MAP.write().unwrap();
            lmap[stroke_msg.vkCode as usize] = false;
        }
    }
    PluginResult::Success
}

async fn undo_clipboard() {
    print!("⏪  ");
    show_operation_message("クリップボードに対するアンドゥ");
    let mut cb_data = unsafe { CLIPBOARD.lock().unwrap() };
    let actual_delete_lines = cb_data.undo_data();
    println!(
        "削除した行数 {}行 残り {}行",
        actual_delete_lines,
        cb_data.get_clipboard_lines()
    );
}

async fn copy_clipboard() {
    print!("💾  ");
    show_operation_message("コピー");
    let mut cb = unsafe { CLIPBOARD.lock().unwrap() };
    let iclip = Clipboard::open();
    unsafe {
        load_data_from_clipboard(&mut cb);
    }
}

async fn reset_clipboard() {
    print!("🧺  ");
    show_operation_message("クリップボードデータの削除");
    let mut cb = unsafe { CLIPBOARD.lock().unwrap() };
    cb.clipboard_clear();
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
            ["✅有効", "🚫無効"][state as usize]
        );
    }
}

// キーイベントハンドラの初期化を行う。初期化時に呼び出される。
pub fn eh_init() {
    let dll = unsafe { DLL.lock().unwrap() };
    unsafe {
        let sethook: libloading::Symbol<DllSetHookApi> = dll.get(b"sethook").unwrap();
        sethook();
    }
    let mut eh_table = unsafe { EH_CTL.write().unwrap() };
    for _ in 0..255 {
        eh_table.push(Box::new(move |_, _| ComboKey::None));
    }
    // CTRL+C と CTRL+ALT+Cが押された時の定義
    eh_table['C' as usize] = Box::new(move |_, ks| {
        let eh: Vec<Box<dyn Fn() -> ComboKey>> = vec![
            // EhKeyState::None
            Box::new(|| {
                let mut in_copy = unsafe { CB_IN_COPY.write().unwrap() };
                *in_copy = true;
                ComboKey::Combo(2)
            }),
            // EhKeyState::Alt
            Box::new(|| {
                async_std::task::spawn(reset_clipboard());
                ComboKey::Combo(3)
            }),
        ];
        eh[ks as usize]()
    });
    eh_table['X' as usize] = Box::new(move |_, ks| {
        let eh: Vec<Box<dyn Fn() -> ComboKey>> = vec![
            // EhKeyState::None
            Box::new(|| {
                let mut in_copy = unsafe { CB_IN_COPY.write().unwrap() };
                *in_copy = true;
                ComboKey::Combo(2)
            }),
            // EhKeyState::Alt
            Box::new(|| {
                async_std::task::spawn(reset_clipboard());
                ComboKey::Combo(3)
            }),
        ];
        eh[ks as usize]()
    });
    // CTRL+Vが押された時の定義
    eh_table['V' as usize] = Box::new(move |_, ks| {
        // 基本的に重たい操作なので非同期で行う
        // 意訳：さっさとフックプロシージャから復帰しないとキーボードがハングする。
        // ただし、Clipboardをロックしてから戻らないとだめ。
        let eh: Vec<Box<dyn Fn() -> ComboKey>> = vec![
            Box::new(|| {
                // CTRL+Vの無効化
                disable_ctrl_v();
                let cb_lock_wait = Arc::new((Mutex::new(false), Condvar::new()));
                async_std::task::spawn(paste(cb_lock_wait.clone()));
                let (lock, _cond) = &*cb_lock_wait;
                let _lock = lock.lock().unwrap(); // クリップボードがロックされるまで待つ。
                ComboKey::Combo(1)
            }),
            Box::new(|| ComboKey::None),
        ];
        eh[ks as usize]()
    });
    // 0が押されたときの定義
    eh_table['0' as usize] = Box::new(move |_, ks| {
        let eh: Vec<Box<dyn Fn() -> ComboKey>> = vec![
            // EhKeyState::None
            Box::new(|| ComboKey::None),
            // EhKeyState::Alt
            Box::new(|| {
                let mode = unsafe { &mut RUN_MODE.write().unwrap() };
                let hook_mode = mode.get_hook_mode();
                if hook_mode == HookMode::Override {
                    mode.set_hook_mode(HookMode::OsStandard);
                    println!("🔒  コピー・ペーストに関するホットキーをOSの既定動作に戻します。");
                    ComboKey::None
                } else if hook_mode == HookMode::OsStandard {
                    mode.set_hook_mode(HookMode::Override);
                    println!("📋  コピー・ペーストに関するホットキーを有効化しました。");
                    ComboKey::Combo(4)
                } else {
                    ComboKey::Combo(3)
                }
            }),
        ];
        eh[ks as usize]()
    });

    eh_table['Z' as usize] = Box::new(move |_, ks| {
        let eh: Vec<Box<dyn Fn() -> ComboKey>> = vec![
            // EhKeyState::None
            Box::new(|| ComboKey::None),
            // EhKeyState::Alt
            Box::new(|| {
                async_std::task::spawn(undo_clipboard());
                ComboKey::Combo(0)
            }),
        ];
        eh[ks as usize]()
    });
    for vkey in 0x31..=0x39 {
        eh_table[vkey] = Box::new(move |_, ks| {
            let eh: Vec<Box<dyn Fn() -> ComboKey>> = vec![
                // EhKeyState::None
                Box::new(|| ComboKey::None),
                // EhKeyState::Alt
                Box::new(|| {
                    // 初期パレットは0
                    let palette_no = unsafe { &mut RUN_MODE.read().unwrap().get_palette_no() };
                    let mut pm = unsafe { TXT_MODIFIER.write().unwrap() };
                    let key = vkey - 0x31;
                    let key = MAX_MODIFIER_PALETTES * (*palette_no) + key;
                    let state = pm.get_plugin_activate_state_with_order(key);
                    if let Some((plugin_name, state)) = state {
                        let state = if state == PluginActivateState::Activate {
                            PluginActivateState::Disable
                        } else {
                            PluginActivateState::Activate
                        };
                        let result = pm.set_plugin_activate_state_with_order(key, state);
                        let (emoji, s) = match result {
                            Some(s) => [("✅", "が有効化されました"), ("🚫", "が無効化されました")]
                                [s as usize],
                            None => ("❌", "はロードされていません"),
                        };
                        println!("{emoji}  モディファイア \"{plugin_name}\" {s}");
                    };
                    ComboKey::Combo(4)
                }),
            ];
            eh[ks as usize]()
        });
    }
    eh_table['Q' as usize] = Box::new(move |lmap, ks| {
        let eh: Vec<Box<dyn Fn() -> ComboKey>> = vec![
            // EhKeyState::None
            Box::new(|| ComboKey::None),
            // EhKeyState::Alt
            Box::new(|| {
                let mut pm = unsafe { TXT_MODIFIER.write().unwrap() };
                // 最大パレット番号
                let load_modifier_counts = pm.loaded_plugin_counts();
                if load_modifier_counts == 0 {
                    return ComboKey::Combo(4);
                }
                let max_palette_count = (load_modifier_counts - 1) / MAX_MODIFIER_PALETTES;
                let mode = unsafe { &mut RUN_MODE.write().unwrap() };
                let palette_no = mode.get_palette_no();
                // パレット番号は0-max_palette_countまでを取る。
                let palette_no = if lmap[VK_LSHIFT.0 as usize] {
                    if usize::MIN == palette_no {
                        max_palette_count
                    } else {
                        palette_no - 1
                    }
                } else {
                    (palette_no + 1) % (max_palette_count + 1)
                };
                mode.set_palette_no(palette_no);
                println!("📝  {} 番パレットに切り替わりました", palette_no);
                println!("📝  現在のパレットにセットされているモディファイアは以下の通りです。");
                show_current_mod_palette(&mut pm, palette_no);
                ComboKey::Combo(4)
            }),
        ];
        eh[ks as usize]()
    });
    eh_table['M' as usize] = Box::new(move |lmap, ks| {
        let input_mode: Vec<Box<dyn Fn(bool) -> (bool, InputMode)>> = vec![
            // EhKeyState::None
            Box::new(|current_burst| (current_burst, InputMode::DirectKeyInput)),
            Box::new(|_| (false, InputMode::Clipboard)),
        ];
        let burst_mode: Vec<Box<dyn Fn(InputMode) -> InputMode>> =
            vec![Box::new(|im| im), Box::new(|_im| InputMode::DirectKeyInput)];
        let eh: Vec<Box<dyn Fn() -> ComboKey>> = vec![
            // EhKeyState::None
            Box::new(|| ComboKey::None),
            // EhKeyState::Alt
            Box::new(|| {
                let mut mode = unsafe { RUN_MODE.write().unwrap() };
                if lmap[VK_LSHIFT.0 as usize] {
                    // InputMode: CTRL+ALT+SHIFT+M
                    let im = mode.get_input_mode();
                    let (burst, im) = input_mode[im as usize](mode.get_burst_mode());
                    mode.set_input_mode(im);
                    let old_burst = mode.get_burst_mode();
                    mode.set_burst_mode(burst);
                    println!(
                        "{}モードに切り替えました。{}",
                        ["📋  クリップボード入力", "🎹  キーボードエミュレーション"][im as usize],
                        ["", "（バーストモードが無効化されました。）"][old_burst as usize]
                    );
                } else {
                    let current_burst_mode = !mode.get_burst_mode();
                    mode.set_burst_mode(current_burst_mode);
                    // バーストモードの場合は、キーボード入力でなければならない
                    // インプットモードを変更する。
                    let im = burst_mode[current_burst_mode as usize](mode.get_input_mode());
                    mode.set_input_mode(im);
                    println!(
                        "{}入力モードに切り替えました。",
                        ["🔂  通常", "🔁  バースト"][current_burst_mode as usize]
                    );
                }
                ComboKey::Combo(4)
            }),
        ];
        eh[ks as usize]()
    });
}

fn judge_combo_key(vk: usize) -> ComboKey {
    let lmap = unsafe { &mut KEY_MAP.read().unwrap() };
    if lmap[VK_LCONTROL.0 as usize] == true {
        let eh_table = unsafe { EH_CTL.read().unwrap() };
        let hook_mode = {
            let mode = unsafe { &mut RUN_MODE.write().unwrap() };
            mode.get_hook_mode()
        };
        // CTRL+ALTキー
        if lmap[VK_LMENU.0 as usize] | lmap[VK_RMENU.0 as usize] {
            // HookMode::OsStandard時は、CTRL+ALT+0以外を全て無効化する。
            if hook_mode == HookMode::OsStandard {
                if vk == 0x30 {
                    return eh_table[vk](lmap, EhKeyState::Alt);
                }
                return ComboKey::None;
            }
            return eh_table[vk](lmap, EhKeyState::Alt);
        }
        // HookMode::OsStandard時は、CTRL+ALT+0以外を全て無効化する。
        if hook_mode == HookMode::Override {
            return eh_table[vk](lmap, EhKeyState::None);
        }
    }
    ComboKey::None
}

pub async fn paste(is_clipboard_locked: Arc<(Mutex<bool>, Condvar)>) {
    let start = Instant::now();
    let mutex = unsafe { THREAD_MUTEX.lock().unwrap() };
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
        let mut cb_data = CLIPBOARD.lock().unwrap();
        EmptyClipboard();
        if cb_data.get_clipboard_lines() == 0 {
            println!("クリップボードにデータがありません。");
            enable_ctrl_v();
            return;
        }
        // オプションをロードする
        let (is_burst_mode, tabindex_keyseq, get_line_delay_msec, char_delay_msec, input_mode) = {
            let mode = RUN_MODE.read().unwrap();
            (
                mode.is_burst_mode(),
                mode.get_tabindex_keyseq(),
                mode.get_line_delay_msec(),
                mode.get_char_delay_msec(),
                mode.get_input_mode(),
            )
        };

        if is_burst_mode && input_mode == InputMode::DirectKeyInput {
            let mut kbd = Keyboard::new();
            let len = cb_data.get_clipboard_lines();
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
                if paste_impl(&mut cb_data) != InputMode::DirectKeyInput {
                    break;
                }
                kbd.send_key();
                // キーストロークとの間に数ミリ秒の待機時間を設ける
                std::thread::sleep(Duration::from_millis(get_line_delay_msec))
            }
        } else {
            paste_impl(&mut cb_data);
        }
        // let wait = g_mode.read().unwrap().get_copy_wait_millis();
        // std::thread::sleep(Duration::from_millis(wait));
        {
            let mode = RUN_MODE.read().unwrap();
            mode.get_input_mode()
        }
    };
    // std::thread::sleep(std::time::Duration::from_millis(1000));
    enable_ctrl_v();
    // Clipboard以外ならキー入力は行わない。
    if input_mode == InputMode::DirectKeyInput {
        return;
    }
    let end = start.elapsed();
    let elapsed = end.as_millis();
    println!(
        "{}  ペースト処理にかかった時間: {} ms",
        if elapsed >= 50 { "⌛" } else { "⏳" },
        elapsed
    );
    let wait = unsafe { RUN_MODE.read().unwrap().paste_timeout() };
    if elapsed >= wait as u128 {
        println!("💨  {wait} ms以上経過しているため、強制ペーストを実行します。");
        // 処理に300ms以上かかっていたら、キー入力は捨てられているので
        // クリップボードモードの場合はもう一度CTRL+Vストロークを送信して強制的にペーストさせる。
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
        let l_ctrl = unsafe {
            let lmap = KEY_MAP.read().unwrap();
            lmap[VK_LCONTROL.0 as usize]
        };
        if l_ctrl == false {
            kbd.append_input_chain(
                KeycodeBuilder::default()
                    .vk(VK_LCONTROL.0)
                    .scan_code(virtual_key_to_scancode(VK_LCONTROL))
                    .key_send_mode(KeySendMode::KeyUp)
                    .build(),
            );
        }
        kbd.send_key();
    }
}

unsafe fn load_data_from_clipboard(cb_data: &mut ClipboardData) -> Option<()> {
    let h_text = GetClipboardData(CF_UNICODETEXT.0);
    match h_text {
        Err(_) => None,
        Ok(h_text) => {
            // クリップボードにデータがあったらロックする
            let p_text = GlobalLock(h_text.0);
            // 今クリップボードにある内容をコピーする（改行で分割される）
            // 後でここの挙動を変えても良さそう。
            let text = u16_ptr_to_string(p_text as *const _).into_string().unwrap();
            let current_len = cb_data.get_clipboard_lines();
            for line in text.lines() {
                let line_len = line.len();
                if line_len != 0 {
                    // cb.push_front(line.to_owned());
                    cb_data.add_clipboard(line.to_owned());
                } else {
                    cb_data.add_clipboard("".to_owned());
                    // cb.push_front("".to_owned());
                }
            }
            cb_data.commit_copy_lines();
            GlobalUnlock(h_text.0);
            println!(
                "クリップボードから {} 行コピーしました",
                cb_data.get_clipboard_lines() - current_len
            );
            Some(())
        }
    }
}

type EncodeFunc = unsafe extern "C" fn(*const u8, usize) -> EncodedString;
unsafe fn paste_impl(cb: &mut ClipboardData) -> InputMode {
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
                println!("🔄  モディファイアによるエンコードに失敗したため、ロールバックします（返却値がUTF-8文字列ではありません / {e}）");
                s
            }
        }
    };
    let (input_mode, char_delay_msec, line_len_max) = {
        let mode = RUN_MODE.read().unwrap();
        (
            mode.get_input_mode(),
            mode.get_char_delay_msec(),
            mode.get_max_line_len(),
        )
    };
    print!("📝  ");
    show_operation_message("ペースト");
    let input_mode = if s.len() > line_len_max && input_mode == InputMode::DirectKeyInput {
        let eh = unsafe { EH_CTL.read().unwrap() };
        let mut lmap = unsafe { KEY_MAP.write().unwrap() };
        let shift = VK_LSHIFT.0 as usize;
        let old_shift = lmap[shift];
        lmap[shift] = true;
        eh['M' as usize](&lmap, EhKeyState::Alt);
        lmap[shift] = old_shift;
        InputMode::Clipboard
    } else {
        input_mode
    };
    if input_mode == InputMode::DirectKeyInput {
        let is_key_pressed = |vk: usize| -> bool {
            let lmap = KEY_MAP.read().unwrap();
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
        enable_ctrl_v();
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
        if s.len() == 0 {
            return input_mode;
        }
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
        let _r = SetClipboardData(CF_UNICODETEXT.0, HANDLE(gdata));
        // 終わったらアンロックしてからメモリを開放する
        GlobalUnlock(gdata);
        GlobalFree(gdata);
    }
    input_mode
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
