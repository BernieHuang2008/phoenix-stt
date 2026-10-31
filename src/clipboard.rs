use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::config::Config;

pub fn copy(cfg: &Config, text: &str) -> Result<()> {
    let mut child = Command::new(&cfg.clipboard.bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| {
            format!(
                "启动 {} 失败。请安装 wl-clipboard，或在配置中设置 [clipboard].bin。",
                cfg.clipboard.bin
            )
        })?;

    {
        let mut stdin = child.stdin.take().context("无法写入剪贴板进程")?;
        stdin.write_all(text.as_bytes()).context("写入剪贴板失败")?;
    }

    let status = child.wait().context("等待剪贴板进程失败")?;
    if !status.success() {
        bail!("{} 退出码 {status}", cfg.clipboard.bin);
    }
    Ok(())
}

pub fn paste(cfg: &Config) -> Result<()> {
    if !cfg.paste.enabled {
        return Ok(());
    }

    if cfg.paste.delay_ms > 0 {
        thread::sleep(Duration::from_millis(cfg.paste.delay_ms));
    }

    let app_id = focused_app_id(&cfg.paste.niri_bin);
    let shortcut = cfg.paste.shortcut_for(app_id.as_deref());
    let args = shortcut_args(shortcut)?;

    let output = Command::new(&cfg.paste.bin)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| {
            format!(
                "启动 {} 失败。请安装 wtype，或在配置中设置 [paste].bin。",
                cfg.paste.bin
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "{} 退出码 {}: {}",
            cfg.paste.bin,
            output.status,
            stderr.trim()
        );
    }
    Ok(())
}

fn focused_app_id(niri_bin: &str) -> Option<String> {
    if niri_bin.trim().is_empty() {
        return None;
    }
    let output = Command::new(niri_bin)
        .args(["msg", "--json", "focused-window"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value: Value = serde_json::from_slice(&output.stdout).ok()?;
    value
        .get("app_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn shortcut_args(shortcut: &str) -> Result<Vec<String>> {
    let parts = shortcut
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let (key, modifiers) = parts
        .split_last()
        .context("粘贴快捷键为空，请检查 [paste].shortcut")?;

    let mut args = Vec::with_capacity(modifiers.len() * 2 + 2);
    for modifier in modifiers {
        args.push("-M".to_string());
        args.push(normalize_modifier(modifier)?);
    }
    args.push("-k".to_string());
    args.push((*key).to_string());
    Ok(args)
}

fn normalize_modifier(modifier: &str) -> Result<String> {
    let normalized = match modifier.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => "ctrl",
        "shift" => "shift",
        "alt" => "alt",
        "super" | "logo" | "win" | "meta" => "logo",
        "altgr" => "altgr",
        _ => bail!("不支持的修饰键 `{modifier}`。可用：ctrl、shift、alt、super/logo/win、altgr。"),
    };
    Ok(normalized.to_string())
}

#[cfg(test)]
mod tests {
    use super::shortcut_args;

    #[test]
    fn parses_modifiers_and_key() {
        assert_eq!(
            shortcut_args("ctrl+shift+v").unwrap(),
            vec!["-M", "ctrl", "-M", "shift", "-k", "v"]
        );
        assert_eq!(shortcut_args("v").unwrap(), vec!["-k", "v"]);
        assert_eq!(
            shortcut_args("Super+Insert").unwrap(),
            vec!["-M", "logo", "-k", "Insert"]
        );
    }
}
