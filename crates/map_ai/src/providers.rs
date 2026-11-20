use url::{Host, Url};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Api {
    Gemini,
    OpenAi,
    Responses,
    Anthropic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Provider {
    Gemini,
    DeepSeek,
    OpenAi,
    OpenAiChat,
    Anthropic,
    Compatible,
    Ollama,
}

impl Provider {
    pub const ALL: [Self; 7] = [
        Self::Gemini,
        Self::DeepSeek,
        Self::OpenAi,
        Self::OpenAiChat,
        Self::Anthropic,
        Self::Compatible,
        Self::Ollama,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::DeepSeek => "deepseek",
            Self::OpenAi => "openai",
            Self::OpenAiChat => "openai-chat",
            Self::Anthropic => "anthropic",
            Self::Compatible => "compatible",
            Self::Ollama => "ollama",
        }
    }
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Gemini => "Gemini",
            Self::DeepSeek => "DeepSeek",
            Self::OpenAi => "OpenAI Responses",
            Self::OpenAiChat => "OpenAI Chat Completions",
            Self::Anthropic => "Anthropic Claude",
            Self::Compatible => "OpenAI 兼容接口",
            Self::Ollama => "本地 Ollama",
        }
    }
    pub fn api(self) -> Api {
        match self {
            Self::Gemini => Api::Gemini,
            Self::OpenAi => Api::Responses,
            Self::Anthropic => Api::Anthropic,
            Self::DeepSeek | Self::OpenAiChat | Self::Compatible | Self::Ollama => Api::OpenAi,
        }
    }
    pub fn default_url(self) -> &'static str {
        match self {
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta",
            Self::DeepSeek => "https://api.deepseek.com",
            Self::OpenAi | Self::OpenAiChat => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com/v1",
            Self::Compatible => "",
            Self::Ollama => "http://127.0.0.1:11434/v1",
        }
    }
    pub fn default_model(self) -> &'static str {
        match self {
            Self::Gemini => "gemini-4-hydrogen",
            Self::DeepSeek => "deepseek-flash",
            Self::OpenAi | Self::OpenAiChat => "gpt-5-mini",
            Self::Anthropic => "claude-sonnet-4-6",
            Self::Compatible | Self::Ollama => "",
        }
    }
    pub fn key_env(self) -> &'static str {
        match self {
            Self::Gemini => "GEMINI_API_KEY",
            Self::DeepSeek => "DEEPSEEK_API_KEY",
            Self::OpenAi | Self::OpenAiChat => "OPENAI_API_KEY",
            Self::Anthropic => "ANTHROPIC_API_KEY",
            Self::Compatible => "ATLAS_AI_API_KEY",
            Self::Ollama => "OLLAMA_API_KEY",
        }
    }
    pub fn configure(self, model: &str, endpoint: &str, key: &str, effort: Option<&str>) -> Config {
        let defaulted = |value: &str, fallback: &str| {
            if value.trim().is_empty() {
                fallback.to_owned()
            } else {
                value.trim().to_owned()
            }
        };
        Config {
            api: self.api(),
            base_url: defaulted(endpoint, self.default_url()),
            model: defaulted(model, self.default_model()),
            key: key.trim().to_owned(),
            effort: effort
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            max_turns: 24,
            timeout_s: 240,
        }
    }
}

/// Connection settings shared by the CLI and the desktop. Keys are only
/// accepted in memory, on stdin to the transport or from an environment variable.
#[derive(Clone)]
pub struct Config {
    pub api: Api,
    pub base_url: String,
    pub model: String,
    pub key: String,
    pub effort: Option<String>,
    pub max_turns: usize,
    pub timeout_s: u64,
}

impl Config {
    pub fn gemini(model: &str, key: &str, effort: Option<&str>) -> Self {
        Provider::Gemini.configure(model, "", key, effort)
    }
    pub fn deepseek(model: &str, key: &str, effort: Option<&str>) -> Self {
        Provider::DeepSeek.configure(model, "", key, effort)
    }
    pub fn label(&self) -> String {
        match &self.effort {
            Some(effort) => format!("{}（{effort}）", self.model),
            None => self.model.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let url = Url::parse(self.base_url.trim()).map_err(|_| "请填写有效的 API 地址。")?;
        if url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("API 地址需包含服务器，密钥请填在独立的密钥栏。".into());
        }
        let local = match url.host() {
            Some(Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
            Some(Host::Ipv4(ip)) => ip.is_loopback(),
            Some(Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        };
        if !(url.scheme() == "https" || url.scheme() == "http" && local) {
            return Err("请为远程服务填写 HTTPS 地址。本机服务可使用 HTTP。".into());
        }
        if self.key.trim().is_empty() && !local {
            return Err("没有 API 密钥。".into());
        }
        if self.model.trim().is_empty() {
            return Err("请填写服务端的模型名称。".into());
        }
        if self.api == Api::Gemini
            && !self
                .model
                .trim_start_matches("models/")
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-._".contains(c))
        {
            return Err("Gemini 模型名须为模型 ID。".into());
        }
        if self.key.chars().any(char::is_control) || self.base_url.chars().any(char::is_control) {
            return Err("地址及密钥中不能含有控制字符。".into());
        }
        if self.max_turns == 0 || self.timeout_s == 0 {
            return Err("轮数与超时时间应为正数。".into());
        }
        Ok(())
    }
    pub(crate) fn endpoint(&self, suffix: &str) -> String {
        let base = self.base_url.trim().trim_end_matches('/');
        if base.ends_with(suffix) {
            base.to_owned()
        } else {
            format!("{base}{suffix}")
        }
    }
    pub(crate) fn headers(&self) -> Vec<(String, String)> {
        let mut result = Vec::new();
        if self.api == Api::Anthropic {
            result.push(("anthropic-version".into(), "2023-06-01".into()));
        }
        if !self.key.is_empty() {
            let header = match self.api {
                Api::Gemini => ("x-goog-api-key".into(), self.key.clone()),
                Api::Anthropic => ("x-api-key".into(), self.key.clone()),
                Api::OpenAi | Api::Responses => {
                    ("Authorization".into(), format!("Bearer {}", self.key))
                }
            };
            result.push(header);
        }
        result
    }
}
