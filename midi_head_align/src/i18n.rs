//! 多言語対応 (ja / en / zh + 外部の言語ファイル)
//!
//! - バイナリに ja/en/zh を埋め込み（言語ファイルが無くても動く）
//! - `lang/*.json` を追加すれば UI の言語メニューに並ぶ
//! - UI から任意の .json を選んで読み込むことも可能
//! - 選択した言語は `~/.midi_head_align.json` に保存
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const EMBEDDED: [(&str, &str, &str); 3] = [
    ("ja", "日本語", include_str!("../lang/ja.json")),
    ("en", "English", include_str!("../lang/en.json")),
    ("zh", "中文（简体）", include_str!("../lang/zh.json")),
];

#[derive(Clone, Debug)]
pub struct Lang {
    pub code: String,
    pub label: String,
    pub path: Option<PathBuf>,
    map: HashMap<String, String>,
}

impl Default for Lang {
    fn default() -> Self {
        Self::parse("ja", "日本語", EMBEDDED[0].2, None).expect("組み込み ja の解析に失敗")
    }
}

impl Lang {
    fn parse(fallback_code: &str, fallback_label: &str, text: &str, path: Option<PathBuf>) -> Result<Self, String> {
        let v: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("JSON エラー: {e}"))?;
        let obj = v.as_object().ok_or("最上位がオブジェクトではありません")?;
        let mut map = HashMap::new();
        for (k, val) in obj {
            match val {
                serde_json::Value::String(s) => {
                    map.insert(k.clone(), s.clone());
                }
                _ => {
                    // 文字列以外は文字列化しておく（数値など）
                    map.insert(k.clone(), val.to_string());
                }
            }
        }
        let code = map.get("_code").cloned().unwrap_or_else(|| fallback_code.to_string());
        let label = map.get("_label").cloned().unwrap_or_else(|| fallback_label.to_string());
        Ok(Self { code, label, path, map })
    }

    pub fn load_file(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("読み込み失敗: {e}"))?;
        let fallback = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "custom".into());
        Self::parse(&fallback, &fallback, &text, Some(path.to_path_buf()))
    }

    pub fn embedded(code: &str) -> Option<Self> {
        EMBEDDED
            .iter()
            .find(|(c, _, _)| *c == code)
            .and_then(|(c, l, t)| Self::parse(c, l, t, None).ok())
    }

    /// キーに対応する文字列（無ければ日本語→キー名の順でフォールバック）
    pub fn t(&self, key: &str) -> String {
        if let Some(s) = self.map.get(key) {
            return s.clone();
        }
        if let Some(s) = fallback_map().get(key) {
            return s.clone();
        }
        key.to_string()
    }

    /// {0} {1} ... を置換して返す
    pub fn tf(&self, key: &str, args: &[&str]) -> String {
        let mut s = self.t(key);
        for (i, a) in args.iter().enumerate() {
            s = s.replace(&format!("{{{i}}}"), a);
        }
        s
    }
}

/// 未翻訳キー用のフォールバック（埋め込み日本語）
fn fallback_map() -> &'static HashMap<String, String> {
    static FALLBACK: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
    FALLBACK.get_or_init(|| {
        let mut m = HashMap::new();
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(EMBEDDED[0].2) {
            if let Some(o) = v.as_object() {
                for (k, val) in o {
                    if let Some(s) = val.as_str() {
                        m.insert(k.clone(), s.to_string());
                    }
                }
            }
        }
        m
    })
}

/// 言語ファイルを探すディレクトリ
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            dirs.push(d.join("lang"));
            // .app バンドル内 (Contents/MacOS → Contents/Resources/lang)
            if let Some(d2) = d.parent() {
                dirs.push(d2.join("Resources").join("lang"));
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("lang"));
    }
    dirs
}

/// 利用可能な言語の一覧（埋め込み + lang/*.json）
pub fn discover() -> Vec<Lang> {
    let mut out: Vec<Lang> = EMBEDDED
        .iter()
        .filter_map(|(c, l, t)| Lang::parse(c, l, t, None).ok())
        .collect();
    for dir in search_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
            .collect();
        files.sort();
        for p in files {
            if let Ok(l) = Lang::load_file(&p) {
                if !out.iter().any(|x| x.code == l.code) {
                    out.push(l);
                }
            }
        }
    }
    out
}

// ------------------------------------------------------------------
// 設定の保存（選択した言語）
// ------------------------------------------------------------------
fn settings_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".midi_head_align.json"))
}

pub fn save_lang(code: &str, path: Option<&Path>) {
    let Some(p) = settings_path() else { return };
    let body = format!(
        "{{\n  \"lang\": {},\n  \"lang_file\": {}\n}}\n",
        serde_json::Value::String(code.to_string()),
        match path {
            Some(f) => serde_json::Value::String(f.to_string_lossy().to_string()).to_string(),
            None => "null".to_string(),
        }
    );
    let _ = std::fs::write(p, body);
}

pub fn load_saved_lang() -> Option<(String, Option<PathBuf>)> {
    let p = settings_path()?;
    let text = std::fs::read_to_string(p).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let code = v.get("lang")?.as_str()?.to_string();
    let file = v
        .get("lang_file")
        .and_then(|x| x.as_str())
        .map(PathBuf::from);
    Some((code, file))
}
