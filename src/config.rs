use clap::{arg, command, ArgGroup, Parser};
use multiline_parser_pluginlib::{
    plugin::{self, MasterConfig, PluginActivateState, PluginManager},
    result::EncodedString,
};
use toolbox::config_loader::*;

pub fn plugin_about(pm: &mut PluginManager, plugin_name: &str) -> (String, PluginActivateState) {
    type PluginAbout = unsafe extern "C" fn() -> EncodedString;
    if let Some((_name, state_orig)) = pm.get_plugin_activate_state(plugin_name) {
        pm.set_plugin_activate_state(plugin_name, plugin::PluginActivateState::Activate);
        let result = match pm.get_plugin_function::<PluginAbout>(plugin_name, "about") {
            Ok(f) => unsafe { f() }.to_string().unwrap_or_default(),
            Err(_) => "".to_owned(),
        };
        pm.set_plugin_activate_state(plugin_name, state_orig.clone());
        (result, state_orig)
    } else {
        ("".to_owned(), PluginActivateState::Disable)
    }
}
pub fn get_config_path() -> String {
    "logic_config.toml".to_owned()
}
pub fn init() -> (RunMode, Config) {
    let args = CommandLineArgs::parse();
    if args.show_install_plugins() {
        std::process::exit(0);
    }
    let mut mode = args.configure(RunMode::default());
    let config: Config = ConfigLoader::load_file(&get_config_path());
    mode.set_config(config.clone());
    crate::default::eh_init();
    (mode, config)
}

use serde_derive::{Deserialize, Serialize};
#[clap(group(
    ArgGroup::new("run_mode")
        .required(false)
        .args(&["clipboard", "burst"]),
))]
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct CommandLineArgs {
    /// 動作モードがクリップボード経由でペーストされます（デフォルト：キーボードエミュレーションでのペースト）
    /// 本モードはバーストモードと排他です。
    #[arg(long, default_value_t = false)]
    clipboard: bool,
    /// バーストモード（フォームに対する連続入力モード）にするか選択できます。
    #[arg(long, default_value_t = false)]
    burst: bool,
    /// モディファイア(プラグイン)の一覧を表示します
    #[arg(long, default_value_t = false)]
    installed_modifiers: bool,
}

fn read_dir<P: AsRef<std::path::Path>>(path: P) -> std::io::Result<Vec<String>> {
    Ok(std::fs::read_dir(path)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            if entry.file_type().ok()?.is_file() {
                Some(entry.file_name().to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .collect())
}
impl CommandLineArgs {
    fn configure(&self, mut run_mode: RunMode) -> RunMode {
        run_mode.set_burst_mode(self.burst);
        run_mode.set_input_mode(if self.clipboard {
            InputMode::Clipboard
        } else {
            InputMode::DirectKeyInput
        });
        run_mode
    }
    fn show_install_plugins(&self) -> bool {
        if self.installed_modifiers {
            let conf: MasterConfig = ConfigLoader::load_file("config.toml");
            let mut pm = PluginManager::new(&conf.plugin_directory);
            if let Ok(files) = read_dir(&conf.plugin_directory) {
                for file in files {
                    let _ = pm.load_plugin(&file);
                    pm.set_plugin_activate_state(&file, plugin::PluginActivateState::Activate);
                }
            };
            let plugin_names = pm.get_plugin_ordered_list().clone();
            for plugin_name in plugin_names {
                let (about, _) = plugin_about(&mut pm, &plugin_name);
                println!("{plugin_name}\t{about}");
            }
        }
        self.installed_modifiers
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub tabindex_key: String,
    pub line_delay_msec: u64,
    pub char_delay_msec: u64,
    pub paste_timeout: u64,
    pub max_line_length: usize,
    pub text_modifiers_dyn_load: bool,
    pub text_modifiers: Option<Vec<String>>,
}
impl Default for Config {
    fn default() -> Self {
        Config {
            tabindex_key: "\t".to_owned(),
            line_delay_msec: 200,
            char_delay_msec: 0,
            paste_timeout: 250,
            max_line_length: 256,
            text_modifiers_dyn_load: false,
            text_modifiers: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputMode {
    Clipboard,
    DirectKeyInput,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HookMode {
    OsStandard,
    Override,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RunMode {
    input_mode: InputMode,
    burst_mode: bool,
    tabindex_keyseq: String,
    line_delay_msec: u64,
    char_delay_msec: u64,
    paste_timeout: u64,
    max_line_len: usize,
    hook_mode: HookMode,
    palette_no: usize,
}
impl Default for RunMode {
    fn default() -> Self {
        RunMode {
            input_mode: InputMode::DirectKeyInput,
            burst_mode: false,
            tabindex_keyseq: String::new(),
            line_delay_msec: 200,
            char_delay_msec: 0,
            paste_timeout: 0,
            max_line_len: 512,
            hook_mode: HookMode::Override,
            palette_no: 0,
        }
    }
}
impl RunMode {
    pub fn new() -> Self {
        RunMode::default()
    }
    pub fn set_config(&mut self, config: Config) {
        self.tabindex_keyseq = config.tabindex_key;
        self.line_delay_msec = config.line_delay_msec;
        self.char_delay_msec = config.char_delay_msec;
        self.max_line_len = config.max_line_length;
        self.paste_timeout = config.paste_timeout;
    }
    pub fn set_burst_mode(&mut self, burst_mode: bool) {
        self.burst_mode = burst_mode
    }
    pub fn is_burst_mode(&self) -> bool {
        self.burst_mode
    }
    pub fn set_input_mode(&mut self, input_mode: InputMode) {
        self.input_mode = input_mode;
    }
    pub fn get_input_mode(&self) -> InputMode {
        self.input_mode
    }
    pub fn get_tabindex_keyseq(&self) -> String {
        self.tabindex_keyseq.clone()
    }
    pub fn get_line_delay_msec(&self) -> u64 {
        self.line_delay_msec
    }
    pub fn get_char_delay_msec(&self) -> u64 {
        self.char_delay_msec
    }
    pub fn paste_timeout(&self) -> u64 {
        self.paste_timeout
    }
    pub fn get_max_line_len(&self) -> usize {
        self.max_line_len
    }
    pub fn set_hook_mode(&mut self, mode: HookMode) {
        self.hook_mode = mode;
    }
    pub fn get_hook_mode(&self) -> HookMode {
        self.hook_mode
    }
    pub fn get_palette_no(&self) -> usize {
        self.palette_no
    }
    pub fn set_palette_no(&mut self, no: usize) {
        self.palette_no = no;
    }
}
////
