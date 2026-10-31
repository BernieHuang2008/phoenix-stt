use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::tools;

pub const WHISPER_MODELS_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/tree/main";
pub const DEFAULT_WHISPER_FILE: &str = "ggml-small-q5_1.bin";

const DEFAULT_PROMPT: &str = "你是一个中文语音输入法的后处理器。请把下面的语音识别结果整理成通顺、自然、适合直接输入到编辑器里的文本：修正错别字和标点，去掉口头禅和重复词，不要添加原文没有的信息。只输出最终文本，不要解释，不要使用 Markdown，不要输出思考过程。";
const DEFAULT_LLM_ENDPOINT: &str = "http://127.0.0.1:4444/v1/chat/completions";
const DEFAULT_LOCAL_MODEL: &str = "qwen3-4b-thinking";

pub fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("phoenix-ai/config/stt.toml")
}

pub fn default_secrets_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("phoenix-ai/secrets/models.json")
}

pub fn default_whisper_model() -> PathBuf {
    let data_dir = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    for candidate in [
        data_dir
            .join("phoenix-ai/models")
            .join(DEFAULT_WHISPER_FILE),
        data_dir.join("phoenix-stt").join(DEFAULT_WHISPER_FILE),
    ] {
        if candidate.exists() {
            return candidate;
        }
    }

    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for rel in [
        "models/ggml-small-q5_1.bin",
        "aitest/models/ggml-small-q5_1.bin",
        "../aitest/models/ggml-small-q5_1.bin",
        "../models/ggml-small-q5_1.bin",
    ] {
        let candidate = cwd.join(rel);
        if candidate.exists() {
            return candidate;
        }
    }

    data_dir
        .join("phoenix-ai/models")
        .join(DEFAULT_WHISPER_FILE)
}

fn default_audio_cmd() -> Vec<String> {
    [
        "pw-record",
        "--rate",
        "16000",
        "--channels",
        "1",
        "--format",
        "s16",
        "--raw",
        "-",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

#[derive(Debug, Clone)]
pub struct Paths {
    pub config: PathBuf,
    pub secrets: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            config: default_config_path(),
            secrets: default_secrets_path(),
        }
    }
}

impl Paths {
    pub fn resolve(config: Option<PathBuf>, secrets: Option<PathBuf>) -> Self {
        let config = config
            .or_else(|| env::var_os("PHOENIX_STT_CONFIG").map(PathBuf::from))
            .map(|p| expand_tilde(&p))
            .unwrap_or_else(default_config_path);
        let secrets = secrets
            .or_else(|| env::var_os("PHOENIX_STT_SECRETS").map(PathBuf::from))
            .or_else(|| env::var_os("PHOENIX_AI_SECRETS").map(PathBuf::from))
            .map(|p| expand_tilde(&p))
            .unwrap_or_else(default_secrets_path);
        Self { config, secrets }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WhisperConfig {
    pub model: PathBuf,
    pub bin: String,
    pub language: String,
    pub threads: u32,
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            model: default_whisper_model(),
            bin: "whisper-cli".to_string(),
            language: "zh".to_string(),
            threads: 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    pub command: Vec<String>,
    pub sample_rate: u32,
    pub chunk_secs: u32,
    pub overlap_secs: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            command: default_audio_cmd(),
            sample_rate: 16_000,
            chunk_secs: 10,
            overlap_secs: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    /// 选中的 models.json 条目名。
    pub model: Option<String>,
    /// 直接覆盖 endpoint；通常留空，由 models.json 决定。
    pub endpoint: Option<String>,
    /// 覆盖发送给 API 的 model 字段。
    pub model_name: Option<String>,
    pub prompt: String,
    pub timeout_secs: u64,
    pub max_tokens: u32,
    pub temperature: f64,
    pub curl_bin: String,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            model: None,
            endpoint: None,
            model_name: None,
            prompt: DEFAULT_PROMPT.to_string(),
            timeout_secs: 180,
            max_tokens: 1024,
            temperature: 0.2,
            curl_bin: "curl".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub enabled: bool,
    /// 胶囊整体缩放倍数。高分屏可以设成 1.5、2.0。
    pub scale: f64,
    /// 胶囊距离屏幕底部的逻辑像素。
    pub margin_bottom: i32,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scale: 1.0,
            margin_bottom: 64,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct IpcConfig {
    pub socket: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClipboardConfig {
    pub bin: String,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            bin: "wl-copy".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PasteConfig {
    /// 复制成功后是否尝试自动粘贴。
    pub enabled: bool,
    /// 发送快捷键的 Wayland 虚拟键盘工具。
    pub bin: String,
    /// 默认粘贴快捷键。
    pub shortcut: String,
    /// 终端类程序使用的粘贴快捷键。
    pub terminal_shortcut: String,
    /// 使用 terminal_shortcut 的 app_id 列表。
    pub terminal_apps: Vec<String>,
    /// 按 app_id 覆盖快捷键。
    pub apps: HashMap<String, String>,
    /// 发送快捷键前的延迟，给剪贴板留一点时间。
    pub delay_ms: u64,
    /// 用于读取焦点窗口 app_id 的命令。
    pub niri_bin: String,
}

impl Default for PasteConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bin: "wtype".to_string(),
            shortcut: "ctrl+v".to_string(),
            terminal_shortcut: "ctrl+shift+v".to_string(),
            terminal_apps: [
                "Alacritty",
                "kitty",
                "foot",
                "org.wezfurlong.wezterm",
                "WezTerm",
                "com.mitchellh.ghostty",
                "xterm",
                "URxvt",
                "st",
                "Konsole",
                "gnome-terminal",
                "org.gnome.Terminal",
                "org.gnome.Console",
                "kgx",
                "Terminator",
                "tilix",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            apps: HashMap::new(),
            delay_ms: 30,
            niri_bin: "niri".to_string(),
        }
    }
}

impl PasteConfig {
    pub fn shortcut_for(&self, app_id: Option<&str>) -> &str {
        if let Some(app_id) = app_id {
            if let Some(shortcut) = self.apps.get(app_id) {
                return shortcut;
            }
            if self
                .terminal_apps
                .iter()
                .any(|terminal| terminal.eq_ignore_ascii_case(app_id))
            {
                return &self.terminal_shortcut;
            }
        }
        &self.shortcut
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ModelRegistry {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub models: HashMap<String, ModelEntry>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ModelEntry {
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct LlmConnection {
    pub endpoint: String,
    pub api_key: Option<String>,
    pub model: String,
    pub headers: Vec<(String, String)>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    pub timeout_secs: Option<u64>,
    pub source: String,
}

impl Default for LlmConnection {
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_LLM_ENDPOINT.to_string(),
            api_key: None,
            model: DEFAULT_LOCAL_MODEL.to_string(),
            headers: Vec::new(),
            temperature: None,
            max_tokens: None,
            timeout_secs: None,
            source: "内置默认（本地 npu-llm）".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelSummary {
    pub name: String,
    pub endpoint: String,
    pub model: String,
    pub has_key: bool,
    pub selected: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub whisper: WhisperConfig,
    pub audio: AudioConfig,
    pub llm: LlmConfig,
    pub ui: UiConfig,
    pub ipc: IpcConfig,
    pub clipboard: ClipboardConfig,
    pub paste: PasteConfig,

    #[serde(skip)]
    pub paths: Paths,
    #[serde(skip)]
    pub llm_connection: LlmConnection,
    #[serde(skip)]
    pub selected_model: Option<String>,
}

impl Config {
    pub fn load_paths(paths: Paths) -> Result<Self> {
        let mut cfg = if paths.config.exists() {
            let raw = fs::read_to_string(&paths.config)
                .with_context(|| format!("读取配置 {} 失败", paths.config.display()))?;
            toml::from_str(&raw)
                .with_context(|| format!("解析配置 {} 失败", paths.config.display()))?
        } else {
            Self::default()
        };

        cfg.paths = paths;
        cfg.apply_env();
        cfg.validate()?;
        cfg.resolve_llm()?;
        Ok(cfg)
    }

    fn apply_env(&mut self) {
        if let Some(v) = env_string("PHOENIX_STT_WHISPER_MODEL") {
            self.whisper.model = expand_tilde(Path::new(&v));
        }
        if let Some(v) = env_string("PHOENIX_STT_WHISPER_BIN") {
            self.whisper.bin = v;
        }
        if let Some(v) = env_string("PHOENIX_STT_WHISPER_LANG") {
            self.whisper.language = v;
        }
        if let Some(v) = env_string("PHOENIX_STT_AUDIO_CMD") {
            self.audio.command = v.split_whitespace().map(str::to_string).collect();
        }
        if let Some(v) = env_string("PHOENIX_STT_SAMPLE_RATE").and_then(|s| s.parse().ok()) {
            self.audio.sample_rate = v;
        }
        if let Some(v) = env_string("PHOENIX_STT_CHUNK_SECS").and_then(|s| s.parse().ok()) {
            self.audio.chunk_secs = v;
        }
        if let Some(v) = env_string("PHOENIX_STT_OVERLAP_SECS").and_then(|s| s.parse().ok()) {
            self.audio.overlap_secs = v;
        }
        if let Some(v) = env_string("PHOENIX_STT_LLM_MODEL") {
            self.llm.model = Some(v);
        }
        if let Some(v) = env_string("PHOENIX_STT_LLM_MODEL_NAME") {
            self.llm.model_name = Some(v);
        }
        if let Some(v) =
            env_string("PHOENIX_STT_LLM_ENDPOINT").or_else(|| env_string("PHOENIX_STT_LLM_URL"))
        {
            self.llm.endpoint = Some(v);
        }
        if let Some(v) = env_string("PHOENIX_STT_PROMPT") {
            self.llm.prompt = v;
        }
        if let Some(v) = env_string("PHOENIX_STT_CURL_BIN") {
            self.llm.curl_bin = v;
        }
        if let Some(v) = env_string("PHOENIX_STT_SOCKET") {
            self.ipc.socket = Some(expand_tilde(Path::new(&v)));
        }
        if let Some(v) = env_string("PHOENIX_STT_CLIPBOARD_BIN") {
            self.clipboard.bin = v;
        }
        if let Some(v) = env_string("PHOENIX_STT_PASTE") {
            self.paste.enabled = !matches!(v.as_str(), "0" | "false" | "no" | "off");
        }
        if let Some(v) = env_string("PHOENIX_STT_PASTE_BIN") {
            self.paste.bin = v;
        }
        if let Some(v) = env_string("PHOENIX_STT_PASTE_SHORTCUT") {
            self.paste.shortcut = v;
        }
        if let Some(v) = env_string("PHOENIX_STT_PASTE_TERMINAL_SHORTCUT") {
            self.paste.terminal_shortcut = v;
        }
        if let Some(v) = env_string("PHOENIX_STT_PASTE_DELAY_MS").and_then(|s| s.parse().ok()) {
            self.paste.delay_ms = v;
        }
        if let Some(v) = env_string("PHOENIX_STT_UI") {
            self.ui.enabled = !matches!(v.as_str(), "0" | "false" | "no" | "off");
        }
        if let Some(v) = env_string("PHOENIX_STT_UI_SCALE").and_then(|s| s.parse().ok()) {
            self.ui.scale = v;
        }
        self.whisper.model = expand_tilde(&self.whisper.model);
    }

    pub fn validate(&self) -> Result<()> {
        if self.audio.command.is_empty() {
            bail!("[audio].command 不能为空");
        }
        if self.audio.sample_rate == 0 {
            bail!("[audio].sample_rate 必须大于 0");
        }
        if self.audio.chunk_secs == 0 {
            bail!("[audio].chunk_secs 必须大于 0");
        }
        if self.audio.overlap_secs >= self.audio.chunk_secs {
            bail!("[audio].overlap_secs 必须小于 chunk_secs");
        }
        if let Some(endpoint) = &self.llm.endpoint {
            if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
                bail!("[llm].endpoint 必须以 http:// 或 https:// 开头");
            }
        }
        if self.paste.enabled {
            if self.paste.bin.trim().is_empty() {
                bail!("[paste].bin 不能为空");
            }
            if self.paste.shortcut.trim().is_empty() {
                bail!("[paste].shortcut 不能为空");
            }
            if self.paste.terminal_shortcut.trim().is_empty() {
                bail!("[paste].terminal_shortcut 不能为空");
            }
        }
        if !(self.ui.scale.is_finite() && self.ui.scale > 0.0) {
            bail!("[ui].scale 必须是大于 0 的数字");
        }
        Ok(())
    }

    fn resolve_llm(&mut self) -> Result<()> {
        if let Some(selected) = &self.llm.model {
            if !self.paths.secrets.exists() {
                bail!(
                    "[llm].model = \"{selected}\"，但密钥文件不存在：{}\n运行 `phoenix-stt init` 生成模板，或创建该文件。",
                    self.paths.secrets.display()
                );
            }
        }

        let registry = if self.paths.secrets.exists() {
            read_registry(&self.paths.secrets)?
        } else {
            ModelRegistry::default()
        };

        let selected = self.llm.model.clone().or_else(|| registry.default.clone());
        self.selected_model = selected.clone();

        if let Some(name) = selected {
            let entry = registry.models.get(&name).ok_or_else(|| {
                let available = registry
                    .models
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                anyhow::anyhow!(
                    "models.json 中没有模型 \"{name}\"。\n可用模型：{}\n文件：{}\n运行 `phoenix-stt models` 查看，或运行 `phoenix-stt init` 生成模板。",
                    if available.is_empty() { "(空)" } else { &available },
                    self.paths.secrets.display()
                )
            })?;
            self.llm_connection = connection_from_entry(&name, entry, &self.llm);
        } else {
            self.llm_connection = LlmConnection {
                endpoint: self
                    .llm
                    .endpoint
                    .clone()
                    .unwrap_or_else(|| DEFAULT_LLM_ENDPOINT.to_string()),
                api_key: None,
                model: self
                    .llm
                    .model_name
                    .clone()
                    .unwrap_or_else(|| DEFAULT_LOCAL_MODEL.to_string()),
                headers: Vec::new(),
                temperature: None,
                max_tokens: None,
                timeout_secs: None,
                source: if self.llm.endpoint.is_some() {
                    "stt.toml [llm].endpoint".to_string()
                } else {
                    "内置默认（本地 npu-llm）".to_string()
                },
            };
        }

        if !self.llm_connection.endpoint.starts_with("http://")
            && !self.llm_connection.endpoint.starts_with("https://")
        {
            bail!(
                "模型 \"{}\" 的 endpoint 无效：{}\nendpoint 必须以 http:// 或 https:// 开头。",
                self.llm_connection.source,
                self.llm_connection.endpoint
            );
        }
        Ok(())
    }

    pub fn socket_path(&self) -> PathBuf {
        if let Some(path) = &self.ipc.socket {
            return path.clone();
        }
        if let Some(runtime) = env::var_os("XDG_RUNTIME_DIR") {
            return PathBuf::from(runtime).join("phoenix-stt.sock");
        }
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/tmp/phoenix-stt-{uid}.sock"))
    }

    pub fn check_whisper(&self) -> Result<()> {
        if !self.whisper.model.exists() {
            let download = format!(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{DEFAULT_WHISPER_FILE}"
            );
            bail!(
                "找不到 whisper 模型：{}\n\n请从 Hugging Face 下载：\n  {WHISPER_MODELS_URL}\n\n示例：\n  mkdir -p ~/.local/share/phoenix-ai/models\n  curl -L -o ~/.local/share/phoenix-ai/models/{DEFAULT_WHISPER_FILE} \\\n    {download}\n\n然后在 {} 中设置 [whisper].model。",
                self.whisper.model.display(),
                self.paths.config.display()
            );
        }

        if tools::find_in_path(&self.whisper.bin).is_none() {
            bail!(
                "找不到 whisper 可执行文件 `{}`。\n建议：在 NixOS 中安装 whisper-cpp，或运行 `nix shell nixpkgs#whisper-cpp`；也可以在 {} 中设置 [whisper].bin。",
                self.whisper.bin,
                self.paths.config.display()
            );
        }
        Ok(())
    }

    pub fn check_audio(&self) -> Result<()> {
        let program = self.audio.command.first().context("[audio].command 为空")?;
        if tools::find_in_path(program).is_none() {
            bail!(
                "找不到录音命令 `{}`。\n建议：确认 PipeWire 正在运行，并且 `pw-record` 在 PATH 中。NixOS 可启用 services.pipewire.enable，或在 {} 中修改 [audio].command。",
                program,
                self.paths.config.display()
            );
        }
        Ok(())
    }

    pub fn check_clipboard(&self) -> Result<()> {
        if tools::find_in_path(&self.clipboard.bin).is_none() {
            bail!(
                "找不到剪贴板命令 `{}`。\n建议：安装 wl-clipboard（提供 wl-copy），或在 {} 中设置 [clipboard].bin。",
                self.clipboard.bin,
                self.paths.config.display()
            );
        }
        Ok(())
    }

    pub fn check_paste(&self) -> Result<()> {
        if !self.paste.enabled {
            return Ok(());
        }
        if tools::find_in_path(&self.paste.bin).is_none() {
            bail!(
                "找不到自动粘贴命令 `{}`。\n建议：安装 wtype（Wayland 虚拟键盘工具），或在 {} 中设置 [paste].bin；也可以设置 [paste].enabled = false 只复制不粘贴。",
                self.paste.bin,
                self.paths.config.display()
            );
        }
        Ok(())
    }

    pub fn check_curl(&self) -> Result<()> {
        if tools::find_in_path(&self.llm.curl_bin).is_none() {
            bail!(
                "找不到 curl 命令 `{}`。\n建议：安装 curl，或在 {} 中设置 [llm].curl_bin。",
                self.llm.curl_bin,
                self.paths.config.display()
            );
        }
        Ok(())
    }

    pub fn list_models(&self) -> Result<Vec<ModelSummary>> {
        let registry = if self.paths.secrets.exists() {
            read_registry(&self.paths.secrets)?
        } else {
            ModelRegistry::default()
        };

        let mut models = registry
            .models
            .iter()
            .map(|(name, entry)| {
                let connection = connection_from_entry(name, entry, &self.llm);
                ModelSummary {
                    name: name.clone(),
                    endpoint: connection.endpoint,
                    model: connection.model,
                    has_key: connection.api_key.is_some(),
                    selected: self.selected_model.as_deref() == Some(name),
                }
            })
            .collect::<Vec<_>>();
        models.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(models)
    }
}

pub fn read_registry(path: &Path) -> Result<ModelRegistry> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("读取密钥文件 {} 失败", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("解析密钥文件 {} 失败（应为 JSON）", path.display()))
}

fn connection_from_entry(name: &str, entry: &ModelEntry, llm: &LlmConfig) -> LlmConnection {
    let endpoint = llm
        .endpoint
        .clone()
        .or_else(|| entry.endpoint.clone())
        .or_else(|| entry.base_url.as_deref().map(join_chat_endpoint))
        .unwrap_or_else(|| DEFAULT_LLM_ENDPOINT.to_string());
    let model = llm
        .model_name
        .clone()
        .or_else(|| entry.model.clone())
        .unwrap_or_else(|| name.to_string());
    let api_key = entry
        .api_key
        .clone()
        .filter(|key| !key.trim().is_empty() && key != "PUT_YOUR_TOKEN_HERE");
    let mut headers = entry
        .headers
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Vec<_>>();
    headers.sort_by(|a, b| a.0.cmp(&b.0));

    LlmConnection {
        endpoint,
        api_key,
        model,
        headers,
        temperature: entry.temperature,
        max_tokens: entry.max_tokens,
        timeout_secs: entry.timeout_secs,
        source: format!("models.json:{name}"),
    }
}

fn join_chat_endpoint(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    }
}

fn env_string(key: &str) -> Option<String> {
    env::var(key).ok().filter(|s| !s.is_empty())
}

fn expand_tilde(path: &Path) -> PathBuf {
    if let Ok(rest) = path.strip_prefix("~") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}
