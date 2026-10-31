use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

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
