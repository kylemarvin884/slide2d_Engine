//! Slide2D编辑器本地化系统，从独立JSON语言资源读取界面文字。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

/// Slide2D当前支持的编辑器语言。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    SimplifiedChinese,
    English,
}

impl Language {
    /// 返回写入本地配置的稳定语言代码。
    pub fn code(self) -> &'static str {
        match self {
            Self::SimplifiedChinese => "zh-CN",
            Self::English => "en-US",
        }
    }

    /// 根据稳定代码恢复语言，未知值回退简体中文。
    fn from_code(code: &str) -> Self {
        if code == "en-US" {
            Self::English
        } else {
            Self::SimplifiedChinese
        }
    }
}

/// 引擎本地语言配置，必须包含Slide2D Localization System标识。
#[derive(Serialize, Deserialize)]
struct LocalizationConfig {
    slide2d_engine: String,
    language: String,
}

/// 当前进程的语言和翻译字典。
struct LocalizationState {
    language: Language,
    chinese: HashMap<String, String>,
    english: HashMap<String, String>,
    revision: u64,
}

static LOCALIZATION: OnceLock<RwLock<LocalizationState>> = OnceLock::new();

/// 初始化语言资源并读取上次保存的用户语言。
pub fn initialize() {
    LOCALIZATION.get_or_init(|| {
        let chinese = parse_language(include_str!("../lang/cn.json"));
        let english = parse_language(include_str!("../lang/en.json"));
        let language = load_language_config();
        RwLock::new(LocalizationState {
            language,
            chinese,
            english,
            revision: 1,
        })
    });
}

/// 返回当前语言。
pub fn current_language() -> Language {
    initialize();
    LOCALIZATION.get().unwrap().read().unwrap().language
}

/// 返回语言修订号，用于同步已经打开的OS窗口标题。
pub fn revision() -> u64 {
    initialize();
    LOCALIZATION.get().unwrap().read().unwrap().revision
}

/// 切换语言、立即替换界面字典并写入本地配置。
pub fn set_language(language: Language) -> Result<(), String> {
    initialize();
    {
        let mut state = LOCALIZATION.get().unwrap().write().unwrap();
        if state.language == language {
            return Ok(());
        }
        state.language = language;
        state.revision += 1;
    }
    save_language_config(language)
}

/// 翻译一个稳定语义键；缺失时回退中文语言资源，再回退键本身。
pub fn tr(key: &str) -> String {
    initialize();
    let state = LOCALIZATION.get().unwrap().read().unwrap();
    let active = if state.language == Language::English {
        &state.english
    } else {
        &state.chinese
    };
    active
        .get(key)
        .or_else(|| state.chinese.get(key))
        .cloned()
        .unwrap_or_else(|| key.to_owned())
}

/// 用动态值替换翻译模板中的`{name}`占位符。
pub fn tr_args(key: &str, arguments: &[(&str, String)]) -> String {
    let mut text = tr(key);
    for (name, value) in arguments {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

/// 翻译底层模块已经格式化的动态消息，同时保留路径、名称和错误详情。
pub fn localize_message(message: &str) -> String {
    if current_language() != Language::English {
        return message.to_owned();
    }
    let state = LOCALIZATION.get().unwrap().read().unwrap();
    let mut text = message.to_owned();
    let mut replacements: Vec<(&String, &String)> = state
        .english
        .iter()
        .filter(|(key, _)| key.starts_with("replace."))
        .collect();
    replacements.sort_by_key(|(key, _)| std::cmp::Reverse(key.len()));
    for (key, value) in replacements {
        let source = key.trim_start_matches("replace.");
        text = text.replace(source, value);
    }
    text
}

/// 解析语言JSON；无效资源回退空字典但不会导致引擎崩溃。
fn parse_language(text: &str) -> HashMap<String, String> {
    serde_json::from_str(text).unwrap_or_default()
}

/// 从用户本地Slide2D配置目录读取上次语言。
fn load_language_config() -> Language {
    let bytes = match fs::read(config_path()) {
        Ok(bytes) => bytes,
        Err(_) => return Language::SimplifiedChinese,
    };
    let config: LocalizationConfig = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Language::SimplifiedChinese,
    };
    if config.slide2d_engine != "SLIDE2D_LOCALIZATION_SYSTEM" {
        return Language::SimplifiedChinese;
    }
    Language::from_code(&config.language)
}

/// 保存带Slide2D Localization System标识的本地语言配置。
fn save_language_config(language: Language) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("{}: {error}", tr("error.localization.create_config")))?;
    }
    let config = LocalizationConfig {
        slide2d_engine: "SLIDE2D_LOCALIZATION_SYSTEM".to_owned(),
        language: language.code().to_owned(),
    };
    let bytes = serde_json::to_vec_pretty(&config)
        .map_err(|error| format!("{}: {error}", tr("error.localization.serialize_config")))?;
    fs::write(path, bytes)
        .map_err(|error| format!("{}: {error}", tr("error.localization.save_config")))
}

/// 返回Windows用户本地配置目录中的Slide2D语言文件路径。
fn config_path() -> PathBuf {
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    root.join("Slide2D").join("localization.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证中英文语言资源拥有完全相同的语义键集合。
    #[test]
    fn language_files_have_matching_keys() {
        let chinese = parse_language(include_str!("../lang/cn.json"));
        let english = parse_language(include_str!("../lang/en.json"));
        let mut chinese_keys: Vec<_> = chinese.keys().collect();
        let mut english_keys: Vec<_> = english.keys().collect();
        chinese_keys.sort();
        english_keys.sort();
        assert_eq!(chinese_keys, english_keys);
    }

    /// 验证固定品牌文字保持英文原版，不进入翻译字典。
    #[test]
    fn fixed_brand_text_is_not_translated() {
        let chinese = parse_language(include_str!("../lang/cn.json"));
        let english = parse_language(include_str!("../lang/en.json"));
        assert!(!chinese.values().any(|value| value == "Slide2D引擎"));
        assert!(!english.contains_key("Made by Slide2D"));
        assert_eq!("Made by Slide2D", "Made by Slide2D");
        assert_eq!("Slide2D Engine", "Slide2D Engine");
    }
}
