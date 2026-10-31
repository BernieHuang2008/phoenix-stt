use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;

use anyhow::Result;

use crate::config::{self, Config, Paths, WHISPER_MODELS_URL};
use crate::llm;
use crate::tools;

#[derive(Default)]
struct Report {
    warnings: u32,
    failures: u32,
}

impl Report {
    fn ok(&self, label: &str, detail: &str) {
        println!("[ok]   {label}: {detail}");
    }

    fn warn(&mut self, label: &str, detail: &str) {
        println!("[warn] {label}: {detail}");
        self.warnings += 1;
    }

    fn fail(&mut self, label: &str, detail: &str) {
        println!("[fail] {label}: {detail}");
        self.failures += 1;
    }
}

pub fn run(paths: &Paths) -> Result<bool> {
    let mut report = Report::default();
    println!("Phoenix STT doctor\n");

    if paths.config.exists() {
        report.ok("配置文件", &paths.config.display().to_string());
    } else {
        report.warn(
            "配置文件",
            &format!(
                "不存在：{}（运行 `phoenix-stt init` 生成）",
                paths.config.display()
            ),
        );
    }

    if paths.secrets.exists() {
        report.ok("密钥文件", &paths.secrets.display().to_string());
        check_secrets_permissions(&mut report, paths);
    } else {
        report.warn(
            "密钥文件",
            &format!(
                "不存在：{}（使用本地默认 endpoint 可不需要；运行 `phoenix-stt init` 生成模板）",
                paths.secrets.display()
            ),
        );
    }

    let cfg = match Config::load_paths(paths.clone()) {
        Ok(cfg) => {
            report.ok("配置加载", "OK");
            Some(cfg)
        }
        Err(error) => {
            report.fail("配置加载", &format!("{error:#}"));
            None
        }
    };

    if let Some(cfg) = &cfg {
        check_model(&mut report, cfg);
        check_whisper(&mut report, cfg);
        check_audio(&mut report, cfg);
        check_clipboard(&mut report, cfg);
        check_curl(&mut report, cfg);
        check_ui(&mut report, cfg);
        check_endpoint(&mut report, cfg);
    }

    println!(
        "\n结果：{} 项警告，{} 项失败",
        report.warnings, report.failures
    );
    Ok(report.failures == 0)
}

fn check_secrets_permissions(report: &mut Report, paths: &Paths) {
    let Ok(metadata) = fs::metadata(&paths.secrets) else {
        return;
    };
    let mode = metadata.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        report.warn(
            "密钥权限",
            &format!(
                "权限为 {mode:o}，建议执行 `chmod 600 {}`",
                paths.secrets.display()
            ),
        );
    } else {
        report.ok("密钥权限", &format!("{mode:o}"));
    }
}

fn check_model(report: &mut Report, cfg: &Config) {
    let connection = &cfg.llm_connection;
    report.ok(
        "LLM 模型",
        &format!(
            "{} -> {}（model={}，key={}）",
            connection.source,
            connection.endpoint,
            connection.model,
            if connection.api_key.is_some() {
                "已配置"
            } else {
                "无"
            }
        ),
    );
}

fn check_whisper(report: &mut Report, cfg: &Config) {
    match fs::metadata(&cfg.whisper.model) {
        Ok(metadata) => {
            let size = metadata.len() as f64 / 1024.0 / 1024.0;
            if size < 1.0 {
                report.warn(
                    "whisper 模型",
                    &format!(
                        "{} 只有 {size:.1} MiB，可能不是完整模型。下载地址：{WHISPER_MODELS_URL}",
                        cfg.whisper.model.display()
                    ),
                );
            } else {
                report.ok(
                    "whisper 模型",
                    &format!("{}（{size:.1} MiB）", cfg.whisper.model.display()),
                );
            }
        }
        Err(_) => report.fail(
            "whisper 模型",
            &format!(
                "找不到 {}。\n       请从 {WHISPER_MODELS_URL} 下载 {}，例如：\n       mkdir -p ~/.local/share/phoenix-ai/models\n       curl -L -o ~/.local/share/phoenix-ai/models/{} https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}\n       然后在 {} 中设置 [whisper].model。",
                cfg.whisper.model.display(),
                config::DEFAULT_WHISPER_FILE,
                config::DEFAULT_WHISPER_FILE,
                config::DEFAULT_WHISPER_FILE,
                cfg.paths.config.display()
            ),
        ),
    }

    match tools::find_in_path(&cfg.whisper.bin) {
        Some(path) => report.ok("whisper 程序", &path.display().to_string()),
        None => report.fail(
            "whisper 程序",
            &format!(
                "找不到 `{}`。NixOS 可安装 whisper-cpp，或运行 `nix shell nixpkgs#whisper-cpp`；也可以在 {} 中设置 [whisper].bin。",
                cfg.whisper.bin,
                cfg.paths.config.display()
            ),
        ),
    }
}

fn check_audio(report: &mut Report, cfg: &Config) {
    let Some(program) = cfg.audio.command.first() else {
        report.fail("录音命令", "[audio].command 为空");
        return;
    };
    match tools::find_in_path(program) {
        Some(path) => report.ok("录音命令", &path.display().to_string()),
        None => report.fail(
            "录音命令",
            &format!(
                "找不到 `{program}`。请确认 PipeWire 正在运行；NixOS 可启用 services.pipewire.enable，或在 {} 中修改 [audio].command。",
                cfg.paths.config.display()
            ),
        ),
    }
}

fn check_clipboard(report: &mut Report, cfg: &Config) {
    match tools::find_in_path(&cfg.clipboard.bin) {
        Some(path) => report.ok("剪贴板", &path.display().to_string()),
        None => report.fail(
            "剪贴板",
            &format!(
                "找不到 `{}`。安装 wl-clipboard，或在 {} 中设置 [clipboard].bin。",
                cfg.clipboard.bin,
                cfg.paths.config.display()
            ),
        ),
    }
}

fn check_curl(report: &mut Report, cfg: &Config) {
    match tools::find_in_path(&cfg.llm.curl_bin) {
        Some(path) => report.ok("curl", &path.display().to_string()),
        None => report.fail(
            "curl",
            &format!(
                "找不到 `{}`。安装 curl，或在 {} 中设置 [llm].curl_bin。",
                cfg.llm.curl_bin,
                cfg.paths.config.display()
            ),
        ),
    }
}

fn check_ui(report: &mut Report, cfg: &Config) {
    if !cfg.ui.enabled {
        report.ok("胶囊 UI", "已禁用（[ui].enabled = false）");
        return;
    }
    if env::var_os("WAYLAND_DISPLAY").is_some() {
        report.ok("Wayland 会话", "已检测到 WAYLAND_DISPLAY");
    } else {
        report.warn(
            "Wayland 会话",
            "未检测到 WAYLAND_DISPLAY；layer-shell 胶囊可能无法显示（[ui].enabled 可关闭）",
        );
    }
}

fn check_endpoint(report: &mut Report, cfg: &Config) {
    let connection = &cfg.llm_connection;
    if connection.api_key.is_none() && !is_local_endpoint(&connection.endpoint) {
        report.warn(
            "API key",
            &format!(
                "endpoint 不是本机地址，但 models.json 条目没有 api_key：{}",
                connection.endpoint
            ),
        );
    }

    match llm::probe(cfg) {
        Ok(code) => report.ok("LLM endpoint", &format!("可访问（HTTP {code}）")),
        Err(error) => report.fail("LLM endpoint", &format!("{error:#}")),
    }
}

fn is_local_endpoint(endpoint: &str) -> bool {
    endpoint.starts_with("http://127.0.0.1")
        || endpoint.starts_with("http://localhost")
        || endpoint.starts_with("http://[::1]")
        || endpoint.starts_with("http://0.0.0.0")
}
