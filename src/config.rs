use clap::{ArgGroup, Parser};
use once_cell::unsync::*;
use send_input::keyboard::windows::*;
use toolbox::config_loader::*;
pub fn init() -> (RunMode,Config) {
    let args = CommandLineArgs::parse();
    let mut mode = args.configure(RunMode::default());
    let config:Config = ConfigLoader::load_file("logic_config.toml");
    mode.set_config(config.clone());
    (mode,config)
}

use serde_derive::{Deserialize, Serialize};

// #[clap(group(
//     ArgGroup::new("run_mode")
//         .required(false)
//         .args(&["clipboard", "burst"]),
// ))]
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
}
// use crate::CommandLineArgs;
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub tabindex_key: String,
    pub line_delay_msec: u64,
    pub char_delay_msec: u64,
    pub copy_wait_msec: u64,
    pub max_line_length: usize,
    pub text_encoders: Option<Vec<String>>,
}
impl Default for Config {
    fn default() -> Self {
        Config {
            tabindex_key: "\t".to_owned(),
            line_delay_msec: 200,
            char_delay_msec: 0,
            copy_wait_msec: 250,
            max_line_length: 512,
            text_encoders: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputMode {
    Clipboard,
    DirectKeyInput,
}
#[derive(Debug, PartialEq)]
pub struct RunMode {
    input_mode: InputMode,
    burst_mode: bool,
    tabindex_keyseq: String,
    line_delay_msec: u64,
    char_delay_msec: u64,
    copy_wait_msec: u64,
    max_line_len: usize,
}
impl Default for RunMode {
    fn default() -> Self {
        RunMode {
            input_mode: InputMode::DirectKeyInput,
            burst_mode: false,
            tabindex_keyseq: String::new(),
            line_delay_msec: 200,
            char_delay_msec: 0,
            copy_wait_msec: 250,
            max_line_len: 512,
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
    }
    pub fn set_burst_mode(&mut self, burst_mode: bool) {
        self.burst_mode = burst_mode
    }
    pub fn set_input_mode(&mut self, input_mode: InputMode) {
        self.input_mode = input_mode;
    }
    pub fn is_burst_mode(&self) -> bool {
        self.burst_mode
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
    pub fn get_copy_wait_millis(&self) -> u64 {
        self.copy_wait_msec
    }
    pub fn get_max_line_len(&self) -> usize {
        self.max_line_len
    }
}
////
