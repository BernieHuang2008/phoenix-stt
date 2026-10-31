use std::io::Write;
use std::process::{Command, ExitStatus};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use tempfile::Builder;

use crate::config::Config;

pub fn polish(cfg: &Config, transcript: &str) -> Result<String> {
    let connection = &cfg.llm_connection;
    let body = json!({
        "model": connection.model,
        "messages": [
            { "role": "system", "content": cfg.llm.prompt },
            { "role": "user", "content": transcript }
        ],
        "temperature": connection.temperature.unwrap_or(cfg.llm.temperature),
        "max_tokens": connection.max_tokens.unwrap_or(cfg.llm.max_tokens),
        "stream": false
    });

    let body_bytes = serde_json::to_vec(&body).context("序列化 LLM 请求失败")?;
    let mut body_file = Builder::new()
        .prefix("phoenix-stt-body-")
        .suffix(".json")
        .tempfile()
        .context("创建临时请求文件失败")?;
    body_file
        .write_all(&body_bytes)
        .context("写入临时请求失败")?;
    body_file.flush().context("刷新临时请求失败")?;

    let mut headers = vec![
        ("Content-Type".to_string(), "application/json".to_string()),
        (
            "User-Agent".to_string(),
            format!("phoenix-stt/{}", env!("CARGO_PKG_VERSION")),
        ),
    ];
    let has_authorization = connection
        .headers
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case("authorization"));
    if let Some(api_key) = &connection.api_key {
        if !has_authorization {
            headers.push(("Authorization".to_string(), format!("Bearer {api_key}")));
        }
    }
    headers.extend(connection.headers.iter().cloned());

    let mut curl_config = Builder::new()
        .prefix("phoenix-stt-curl-")
        .suffix(".conf")
        .tempfile()
        .context("创建临时 curl 配置失败")?;
    for (key, value) in &headers {
        writeln!(
            curl_config,
            "header = \"{}\"",
            curl_escape(&format!("{key}: {value}"))
        )
        .context("写入临时 curl 配置失败")?;
    }
    curl_config.flush().context("刷新临时 curl 配置失败")?;

    let timeout = connection
        .timeout_secs
        .unwrap_or(cfg.llm.timeout_secs)
        .max(1);

    let output = Command::new(&cfg.llm.curl_bin)
        .arg("--silent")
        .arg("--show-error")
        .arg("--fail-with-body")
        .arg("--proto")
        .arg("=http,https")
        .arg("--max-time")
        .arg(timeout.to_string())
        .arg("--config")
        .arg(curl_config.path())
        .arg("--data-binary")
        .arg(format!("@{}", body_file.path().display()))
        .arg(&connection.endpoint)
        .output()
        .with_context(|| {
            format!(
                "启动 curl (`{}`) 失败。请安装 curl 或设置 [llm].curl_bin。",
                cfg.llm.curl_bin
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let body = String::from_utf8_lossy(&output.stdout);
        bail!(
            "{}",
            request_error(cfg, &output.status, stderr.trim(), body.trim())
        );
    }

    let value: Value = serde_json::from_slice(&output.stdout).with_context(|| {
        format!(
            "解析 LLM 响应 JSON 失败（endpoint: {}）。响应片段：{}",
            connection.endpoint,
            truncate(&String::from_utf8_lossy(&output.stdout), 400)
        )
    })?;

    let content = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/choices/0/text").and_then(Value::as_str))
        .unwrap_or_default();

    let cleaned = clean_output(content);
    if cleaned.is_empty() {
        bail!(
            "LLM 返回了空文本（model: {}，endpoint: {}）",
            connection.model,
            connection.endpoint
        );
    }
    Ok(cleaned)
}

pub fn probe(cfg: &Config) -> Result<String> {
    let connection = &cfg.llm_connection;
    let output = Command::new(&cfg.llm.curl_bin)
        .arg("--silent")
        .arg("--show-error")
        .arg("--max-time")
        .arg("5")
        .arg("--output")
        .arg("/dev/null")
        .arg("--write-out")
        .arg("%{http_code}")
        .arg("--proto")
        .arg("=http,https")
        .arg(&connection.endpoint)
        .output()
        .with_context(|| format!("启动 curl (`{}`) 失败", cfg.llm.curl_bin))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "{}",
            probe_error(&output.status, stderr.trim(), &connection.endpoint)
        );
    }

    let code = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if code.is_empty() || code == "000" {
        bail!("endpoint 无法访问：{}", connection.endpoint);
    }
    Ok(code)
}

fn request_error(cfg: &Config, status: &ExitStatus, stderr: &str, body: &str) -> String {
    let connection = &cfg.llm_connection;
    let code = status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_string());
    let reason = match status.code() {
        Some(6) => "无法解析域名",
        Some(7) => "无法连接到服务器",
        Some(22) => "HTTP 错误",
        Some(28) => "请求超时",
        Some(35) => "TLS 握手失败",
        Some(52) => "服务器没有返回内容",
        Some(60) => "证书校验失败",
        _ => "curl 调用失败",
    };

    let mut message = format!(
        "LLM 请求失败（{reason}）\nendpoint: {}\nmodel: {}\n来源: {}\ncurl 退出码: {code}\n{stderr}",
        connection.endpoint, connection.model, connection.source
    );
    if !body.is_empty() {
        message.push_str(&format!("\nHTTP 响应片段：{}", truncate(body, 600)));
    }
    message.push_str(
        "\n\n建议：\n- 本地模型：确认 npu-llm 已启动，例如 `npu-llm --model qwen3-4b-thinking --serve 4444`\n- 外部模型：检查 endpoint、网络/代理，以及 secrets/models.json 中的 api_key\n- 运行 `phoenix-stt doctor` 查看完整检查",
    );
    message
}

fn probe_error(status: &ExitStatus, stderr: &str, endpoint: &str) -> String {
    let reason = match status.code() {
        Some(6) => "无法解析域名",
        Some(7) => "无法连接到服务器",
        Some(28) => "请求超时",
        Some(35) => "TLS 握手失败",
        Some(60) => "证书校验失败",
        _ => "curl 调用失败",
    };
    let hint = if endpoint.starts_with("http://127.0.0.1")
        || endpoint.starts_with("http://localhost")
        || endpoint.starts_with("http://[::1]")
    {
        "这是本地 endpoint。确认 npu-llm 已启动，例如：\n  npu-llm --model qwen3-4b-thinking --serve 4444 --ctx 2048"
    } else {
        "这是外部 endpoint。检查 base_url/endpoint、api_key、网络或代理设置。"
    };
    format!("endpoint 无法访问（{reason}）\nendpoint: {endpoint}\n{stderr}\n\n建议：{hint}")
}

fn curl_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\n', '\r'], " ")
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn clean_output(raw: &str) -> String {
    let mut text = raw.trim().to_string();

    if let Some(index) = text.rfind(" response") {
        text = text[index + " response".len()..].trim().to_string();
    } else if let Some(index) = text.rfind("<｜end▁of▁thinking｜>") {
        text = text[index + "<｜end▁of▁thinking｜>".len()..]
            .trim()
            .to_string();
    }

    if text.starts_with("```") {
        let mut lines = text.lines();
        lines.next();
        text = lines.collect::<Vec<_>>().join("\n");
        if let Some(index) = text.rfind("```") {
            text.truncate(index);
        }
    }

    text.trim()
        .trim_matches(|c| {
            matches!(
                c,
                '"' | '\'' | '“' | '”' | '‘' | '’' | '「' | '」' | '『' | '』'
            )
        })
        .trim()
        .to_string()
}
