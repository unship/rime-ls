use crate::consts::trigger_ptn;
use directories::ProjectDirs;
use once_cell::sync::OnceCell;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// Global config shared by all Backends. Updated when ~/.config/rime-ls/config.yaml changes.
static GLOBAL_CONFIG: OnceCell<Arc<RwLock<Config>>> = OnceCell::new();

/// Global regex derived from config.trigger_characters. Updated when config changes.
static GLOBAL_REGEX: OnceCell<Arc<RwLock<Arc<Regex>>>> = OnceCell::new();

/// all configs of rime-ls
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    /// if enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// rime share data dir
    #[serde(default = "default_shared_data_dir")]
    pub shared_data_dir: PathBuf,
    /// rime user data dir
    #[serde(default = "default_user_data_dir")]
    pub user_data_dir: PathBuf,
    /// rime log data dir
    #[serde(default = "default_log_dir")]
    pub log_dir: PathBuf,
    /// max number of candidates
    #[serde(default = "default_max_candidates")]
    pub max_candidates: usize,
    /// if not empty, these characters will trigger completion for paging
    #[serde(default = "default_paging_characters")]
    pub paging_characters: Vec<String>,
    /// if not empty, only trigger completion with special keys
    #[serde(default = "default_trigger_characters")]
    pub trigger_characters: Vec<String>,
    /// if set, completion request with this string will trigger「方案選單」
    #[serde(default = "default_schema_trigger_character")]
    pub schema_trigger_character: String,
    /// if set, when a delete action arrives the number of max tokens, emit a force new_typing
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    /// if CompletionItem is always incomplete
    #[serde(default = "default_always_incomplete")]
    pub always_incomplete: bool,
    /// if preselect first CompletionItem
    #[serde(default = "default_preselect_first")]
    pub preselect_first: bool,
    /// if including word prefix in filter_text
    #[serde(default = "default_long_filter_text")]
    pub long_filter_text: bool,
    /// if showing order in label
    #[serde(default = "default_show_order_in_label")]
    pub show_order_in_label: bool,
    /// if showing comment (e.g. pinyin) in completion item detail
    #[serde(default = "default_show_comment")]
    pub show_comment: bool,
    /// if true, remove paging chars from document when paging
    #[serde(default = "default_hide_paging_characters")]
    pub hide_paging_characters: bool,
    /// if true, auto-commit selection (e.g. number keys) via workspace/applyEdit
    /// to avoid needing an extra confirmation in some LSP clients.
    #[serde(default = "default_auto_commit_on_select")]
    pub auto_commit_on_select: bool,
    /// if true, when pinyin exactly matches an English word candidate, put it first
    #[serde(default = "default_prefer_english_match")]
    pub prefer_english_match: bool,
    /// if true, segment current file and use as temporary dict for candidates
    #[serde(default = "default_document_dict")]
    pub document_dict: bool,
    /// max extra candidates from document dict (when document_dict is true)
    #[serde(default = "default_document_dict_max_candidates")]
    pub document_dict_max_candidates: usize,
    /// 文档词库是否模糊匹配 n/ng（四川等地方言：pin≈ping、san≈sang）
    #[serde(default = "default_document_dict_fuzzy_n_ng")]
    pub document_dict_fuzzy_n_ng: bool,
}

/// Client initialization options - all optional, used to override server config
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ClientInitOptions {
    pub enabled: Option<bool>,
    pub shared_data_dir: Option<PathBuf>,
    pub user_data_dir: Option<PathBuf>,
    pub log_dir: Option<PathBuf>,
    pub max_candidates: Option<usize>,
    pub paging_characters: Option<Vec<String>>,
    pub trigger_characters: Option<Vec<String>>,
    pub schema_trigger_character: Option<String>,
    pub max_tokens: Option<usize>,
    pub always_incomplete: Option<bool>,
    pub preselect_first: Option<bool>,
    pub long_filter_text: Option<bool>,
    pub show_order_in_label: Option<bool>,
    pub show_comment: Option<bool>,
    pub hide_paging_characters: Option<bool>,
    pub auto_commit_on_select: Option<bool>,
    pub prefer_english_match: Option<bool>,
    pub document_dict: Option<bool>,
    pub document_dict_max_candidates: Option<usize>,
    pub document_dict_fuzzy_n_ng: Option<bool>,
}

impl Config {
    /// Merge client init options over this config. Only overrides fields that client explicitly sent.
    pub fn merge_client_options(&mut self, opts: ClientInitOptions) {
        apply_setting!(self <- opts.enabled);
        apply_setting!(self <- opts.shared_data_dir);
        apply_setting!(self <- opts.user_data_dir);
        apply_setting!(self <- opts.log_dir);
        apply_setting!(self <- opts.max_candidates);
        apply_setting!(self <- opts.paging_characters);
        apply_setting!(self <- opts.trigger_characters);
        apply_setting!(self <- opts.schema_trigger_character);
        apply_setting!(self <- opts.max_tokens);
        apply_setting!(self <- opts.always_incomplete);
        apply_setting!(self <- opts.preselect_first);
        apply_setting!(self <- opts.long_filter_text);
        apply_setting!(self <- opts.show_order_in_label);
        apply_setting!(self <- opts.show_comment);
        apply_setting!(self <- opts.hide_paging_characters);
        apply_setting!(self <- opts.auto_commit_on_select);
        apply_setting!(self <- opts.prefer_english_match);
        apply_setting!(self <- opts.document_dict);
        apply_setting!(self <- opts.document_dict_max_candidates);
        apply_setting!(self <- opts.document_dict_fuzzy_n_ng);
    }
}

/// settings that can be tweaked during running
#[derive(Debug, Serialize, Deserialize)]
pub struct Settings {
    /// enabled
    pub enabled: Option<bool>,
    /// max number of candidates
    pub max_candidates: Option<usize>,
    /// if not empty, these characters will trigger completion for paging
    pub paging_characters: Option<Vec<String>>,
    /// if not empty, only trigger completion with special keys
    pub trigger_characters: Option<Vec<String>>,
    /// if set, completion request with this string will trigger「方案選單」
    pub schema_trigger_character: Option<String>,
    /// if set, when a delete action arrives the number of max tokens, emit a force new_typing
    pub max_tokens: Option<usize>,
    /// if CompletionItem is always incomplete
    pub always_incomplete: Option<bool>,
    /// if preselect first CompletionItem
    pub preselect_first: Option<bool>,
    /// if including word prefix in filter_text
    pub long_filter_text: Option<bool>,
    /// if showing order in label
    pub show_order_in_label: Option<bool>,
    /// if showing comment in detail
    pub show_comment: Option<bool>,
    /// if true, remove paging chars from document when paging
    pub hide_paging_characters: Option<bool>,
    /// if true, auto-commit selection (e.g. number keys) via workspace/applyEdit
    pub auto_commit_on_select: Option<bool>,
    /// if true, when pinyin matches English word, put it first
    pub prefer_english_match: Option<bool>,
    pub document_dict: Option<bool>,
    pub document_dict_max_candidates: Option<usize>,
    pub document_dict_fuzzy_n_ng: Option<bool>,
}

macro_rules! apply_setting {
    ($to:ident <- $from:ident.$field:ident) => {
        if let Some(v) = $from.$field {
            $to.$field = v;
        }
    };
    ($to:ident <- $from:ident.$field:ident, |$v:ident| $b:block) => {
        if let Some($v) = $from.$field {
            $b
            $to.$field = $v;
        }
    };
}
pub(crate) use apply_setting;

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: default_enabled(),
            shared_data_dir: default_shared_data_dir(),
            user_data_dir: default_user_data_dir(),
            log_dir: default_log_dir(),
            max_candidates: default_max_candidates(),
            paging_characters: default_paging_characters(),
            trigger_characters: default_trigger_characters(),
            schema_trigger_character: default_schema_trigger_character(),
            max_tokens: default_max_tokens(),
            always_incomplete: default_always_incomplete(),
            preselect_first: default_preselect_first(),
            long_filter_text: default_long_filter_text(),
            show_order_in_label: default_show_order_in_label(),
            show_comment: default_show_comment(),
            hide_paging_characters: default_hide_paging_characters(),
            auto_commit_on_select: default_auto_commit_on_select(),
            prefer_english_match: default_prefer_english_match(),
            document_dict: default_document_dict(),
            document_dict_max_candidates: default_document_dict_max_candidates(),
            document_dict_fuzzy_n_ng: default_document_dict_fuzzy_n_ng(),
        }
    }
}

/// Path to server-side config file: ~/.config/rime-ls/config.yaml
pub fn server_config_path() -> PathBuf {
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".config"))
                .unwrap_or_else(|_| PathBuf::from(".config"))
        });
    config_dir.join("rime-ls").join("config.yaml")
}

/// Load config from server config file. Returns default config if file does not exist or fails to parse.
pub fn load_from_file(path: impl AsRef<Path>) -> Config {
    let path = path.as_ref();
    if !path.exists() {
        return Config::default();
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Config::default(),
    };
    serde_yaml::from_str(&content).unwrap_or_default()
}

pub(crate) fn compile_regex_from_trigger_chars(chars: &[String]) -> Arc<Regex> {
    if chars.is_empty() {
        Arc::new(Regex::new(crate::consts::NT_PTN).unwrap())
    } else {
        let pattern = format!(trigger_ptn!(), chars.join(""));
        Arc::new(Regex::new(&pattern).unwrap())
    }
}

/// Initialize global config. Merges client init_options on first init. No-op if already initialized.
pub fn init_global_config(init_options: Option<serde_json::Value>) {
    if GLOBAL_CONFIG.get().is_some() {
        return;
    }
    let config_path = server_config_path();
    let mut config = load_from_file(&config_path);
    if let Some(opts) = init_options {
        if let Ok(client_opts) = serde_json::from_value::<ClientInitOptions>(opts) {
            config.merge_client_options(client_opts);
        }
    }
    let trigger_chars = config.trigger_characters.clone();
    let config = Arc::new(RwLock::new(config));
    let regex = Arc::new(RwLock::new(compile_regex_from_trigger_chars(&trigger_chars)));
    let _ = GLOBAL_CONFIG.set(config);
    let _ = GLOBAL_REGEX.set(regex);
}

/// Get global config. Panics if not initialized.
pub fn get_global_config() -> Arc<RwLock<Config>> {
    GLOBAL_CONFIG.get().expect("config not initialized").clone()
}

/// Get global regex. Panics if not initialized.
pub fn get_global_regex() -> Arc<RwLock<Arc<Regex>>> {
    GLOBAL_REGEX.get().expect("regex not initialized").clone()
}

/// Reload config from file and update global state. Call when config.yaml changes.
pub fn reload_config_from_file() {
    if let (Some(config), Some(regex)) = (GLOBAL_CONFIG.get(), GLOBAL_REGEX.get()) {
        let new_config = load_from_file(server_config_path());
        *config.write().unwrap() = new_config;
        let trigger_chars = config.read().unwrap().trigger_characters.clone();
        *regex.write().unwrap() = compile_regex_from_trigger_chars(&trigger_chars);
    }
}

fn default_enabled() -> bool {
    true
}

fn default_always_incomplete() -> bool {
    false
}

fn default_max_candidates() -> usize {
    10
}

fn default_max_tokens() -> usize {
    0
}

fn default_trigger_characters() -> Vec<String> {
    Vec::default()
}

fn default_paging_characters() -> Vec<String> {
    ["-", "=", ",", "."].map(|x| x.to_string()).to_vec()
}

fn default_shared_data_dir() -> PathBuf {
    PathBuf::from(crate::utils::rime_default_shared_data_dir())
}

fn default_user_data_dir() -> PathBuf {
    let proj_dirs = ProjectDirs::from("com", "rimels", "Rime-Ls").unwrap();
    proj_dirs.data_dir().to_path_buf()
}

fn default_log_dir() -> PathBuf {
    let proj_dirs = ProjectDirs::from("com", "rimels", "Rime-Ls").unwrap();
    proj_dirs.cache_dir().to_path_buf()
}

fn default_schema_trigger_character() -> String {
    String::default()
}

fn default_preselect_first() -> bool {
    false
}

fn default_long_filter_text() -> bool {
    false
}

fn default_show_order_in_label() -> bool {
    true
}

fn default_show_comment() -> bool {
    true
}

fn default_hide_paging_characters() -> bool {
    true
}

fn default_auto_commit_on_select() -> bool {
    true
}

fn default_prefer_english_match() -> bool {
    true
}

fn default_document_dict() -> bool {
    true
}

fn default_document_dict_max_candidates() -> usize {
    5
}

fn default_document_dict_fuzzy_n_ng() -> bool {
    true
}

#[test]
fn test_load_from_file() {
    let temp_dir = std::env::temp_dir().join("rime-ls-config-test");
    std::fs::create_dir_all(&temp_dir).unwrap();
    let config_path = temp_dir.join("config.yaml");
    std::fs::write(
        &config_path,
        r#"shared_data_dir: "/usr/share/rime-data"
user_data_dir: "~/.local/share/rime-ls"
max_candidates: 9
schema_trigger_character: "&"
always_incomplete: true
"#,
    )
    .unwrap();
    let config = load_from_file(&config_path);
    assert_eq!(config.shared_data_dir, PathBuf::from("/usr/share/rime-data"));
    assert_eq!(config.user_data_dir, PathBuf::from("~/.local/share/rime-ls"));
    assert_eq!(config.max_candidates, 9);
    assert_eq!(config.schema_trigger_character, "&");
    assert!(config.always_incomplete);
    std::fs::remove_dir_all(temp_dir).ok();
}

#[test]
fn test_default_config() {
    let config: Config = Default::default();
    assert_eq!(config.enabled, default_enabled());
    assert_eq!(config.shared_data_dir, default_shared_data_dir());
    assert_eq!(config.user_data_dir, default_user_data_dir());
    assert_eq!(config.log_dir, default_log_dir());
    assert_eq!(config.max_candidates, default_max_candidates());
    assert_eq!(config.trigger_characters, default_trigger_characters());
    assert_eq!(
        config.schema_trigger_character,
        default_schema_trigger_character()
    );
    assert_eq!(config.always_incomplete, default_always_incomplete());
    assert_eq!(config.max_tokens, default_max_tokens());
    assert!(config.hide_paging_characters);
    assert!(config.auto_commit_on_select);
}

#[test]
fn test_apply_settings() {
    let mut config: Config = Default::default();
    let settings: Settings = Settings {
        enabled: Some(false),
        max_candidates: Some(100),
        paging_characters: Some(vec![",".to_string(), ".".to_string()]),
        trigger_characters: Some(vec!["foo".to_string()]),
        schema_trigger_character: Some(String::from("bar")),
        max_tokens: None,
        always_incomplete: None,
        preselect_first: None,
        long_filter_text: None,
        show_order_in_label: Some(false),
        show_comment: None,
        hide_paging_characters: Some(false),
        auto_commit_on_select: Some(false),
        prefer_english_match: None,
        document_dict: None,
        document_dict_max_candidates: None,
        document_dict_fuzzy_n_ng: None,
    };
    // apply settings with macro
    let mut test_val = vec!["baz".to_string()];
    apply_setting!(config <- settings.enabled);
    apply_setting!(config <- settings.max_candidates);
    apply_setting!(config <- settings.paging_characters);
    apply_setting!(config <- settings.trigger_characters, |v| {
        test_val = v.clone();
    });
    apply_setting!(config <- settings.schema_trigger_character);
    apply_setting!(config <- settings.show_order_in_label);
    apply_setting!(config <- settings.hide_paging_characters);
    apply_setting!(config <- settings.auto_commit_on_select);
    // verify
    assert!(!config.enabled);
    assert_eq!(config.max_candidates, 100);
    assert_eq!(
        config.paging_characters,
        vec![",".to_string(), ".".to_string()]
    );
    assert_eq!(config.trigger_characters, vec!["foo".to_string()]);
    assert_eq!(config.schema_trigger_character, String::from("bar"));
    assert!(!config.show_order_in_label);
    assert_eq!(test_val, vec!["foo".to_string()]);
    assert!(!config.hide_paging_characters);
    assert!(!config.auto_commit_on_select);
}
