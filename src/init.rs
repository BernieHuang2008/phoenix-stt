use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::{Config, WHISPER_MODELS_URL};

const SECRETS_TEMPLATE: &str = r#"{
  "default": "local-qwen",
  "models": {
    "local-qwen": {
      "endpoint": "http://127.0.0.1:4444/v1/chat/completions",
      "model": "qwen3-4b-thinking"
    },
    "openai": {
      "base_url": "https://api.openai.com/v1",
      "api_key": "PUT_YOUR_TOKEN_HERE",
      "model": "gpt-4o-mini"
    },
    "deepseek": {
      "base_url": "https://api.deepseek.com/v1",
      "api_key": "PUT_YOUR_TOKEN_HERE",
      "model": "deepseek-chat"
    }
  }
}
"#;

pub fn run(config_path: &Path, secrets_path: &Path, force: bool) -> Result<()> {
    let defaults = Config::default();
    let model = defaults.whisper.model.display().to_string();
    let prompt = format!("{:?}", defaults.llm.prompt);
    let model_quoted = format!("{:?}", model);

    let config_template = format!(
        r#"# Phoenix STT 配置
# 文档：phoenix-stt --help / phoenix-stt doctor
# 密钥：{secrets}

[whisper]
model = {model}
bin = "whisper-cli"
language = "zh"
threads = 4

[audio]
command = ["pw-record", "--rate", "16000", "--channels", "1", "--format", "s16", "--raw", "-"]
sample_rate = 16000
chunk_secs = 10
overlap_secs = 1

[llm]
# models.json 中的条目名。切换外部模型时改这里。
model = "local-qwen"
# 可选：直接覆盖 endpoint / API model 名（通常留空）
# endpoint = "http://127.0.0.1:4444/v1/chat/completions"
# model_name = "qwen3-4b-thinking"
prompt = {prompt}
timeout_secs = 180
max_tokens = 1024
temperature = 0.2
curl_bin = "curl"

[ui]
enabled = true

[ipc]
# socket = "/run/user/1000/phoenix-stt.sock"

[clipboard]
bin = "wl-copy"
"#,
        secrets = secrets_path.display(),
        model = model_quoted,
        prompt = prompt,
    );

    let wrote_config = write_file(config_path, &config_template, force, false)?;
    let wrote_secrets = write_file(secrets_path, SECRETS_TEMPLATE, force, true)?;

    println!(
        "配置：{}（{}）",
        config_path.display(),
        if wrote_config {
            "已写入"
        } else {
            "已存在，跳过"
        }
    );
    println!(
        "密钥：{}（{}）",
        secrets_path.display(),
        if wrote_secrets {
            "已写入"
        } else {
            "已存在，跳过"
        }
    );

    if !defaults.whisper.model.exists() {
        println!(
            "\n警告：whisper 模型还不存在：{}",
            defaults.whisper.model.display()
        );
        println!("从 {WHISPER_MODELS_URL} 下载，例如：");
        println!(
            "  mkdir -p \"$(dirname \"{model}\")\"\n  curl -L -o \"{model}\" \\\n    https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin"
        );
    }

    println!("\n下一步：");
    println!("  phoenix-stt doctor      # 检查配置、模型和 LLM endpoint");
    println!("  phoenix-stt daemon      # 启动常驻进程");
    println!("  phoenix-stt toggle      # 开始/停止录音");

    Ok(())
}

fn write_file(path: &Path, content: &str, force: bool, secret: bool) -> Result<bool> {
    if path.exists() && !force {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建目录 {} 失败", parent.display()))?;
        if secret {
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }
    fs::write(path, content).with_context(|| format!("写入 {} 失败", path.display()))?;
    if secret {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("设置 {} 权限失败", path.display()))?;
    }
    Ok(true)
}
