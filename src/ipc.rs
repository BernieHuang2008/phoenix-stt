use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;

use anyhow::{bail, Context, Result};

use crate::AppEvent;

pub fn send_toggle(path: &Path) -> Result<()> {
    send_command(path, "toggle")
}

pub fn send_command(path: &Path, command: &str) -> Result<()> {
    let mut stream = UnixStream::connect(path).with_context(|| {
        format!(
            "无法连接 {}，phoenix-stt daemon 是否在运行？",
            path.display()
        )
    })?;
    stream
        .write_all(format!("{command}\n").as_bytes())
        .with_context(|| format!("发送 {command} 失败"))?;
    Ok(())
}

pub fn spawn_listener(path: PathBuf, events: Sender<AppEvent>) -> Result<()> {
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            bail!("phoenix-stt daemon 已经在运行（{}）", path.display());
        }
        fs::remove_file(&path).with_context(|| format!("删除旧 socket {} 失败", path.display()))?;
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建 socket 目录 {} 失败", parent.display()))?;
    }

    let listener = UnixListener::bind(&path)
        .with_context(|| format!("监听 socket {} 失败", path.display()))?;
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));

    let cleanup_path = path.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    let mut command = String::new();
                    if stream.read_to_string(&mut command).is_err() {
                        continue;
                    }
                    match command.trim() {
                        "toggle" => {
                            let _ = events.send(AppEvent::Toggle);
                        }
                        "quit" => {
                            let _ = events.send(AppEvent::Quit);
                            break;
                        }
                        _ => {}
                    }
                }
                Err(error) => eprintln!("phoenix-stt: IPC 错误: {error}"),
            }
        }
        let _ = fs::remove_file(cleanup_path);
    });

    Ok(())
}
