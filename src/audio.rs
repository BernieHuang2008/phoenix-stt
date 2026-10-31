use std::io::{ErrorKind, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::config::Config;
use crate::transcribe;
use crate::AppEvent;

pub struct Chunk {
    pub index: usize,
    pub samples: Vec<i16>,
}

pub fn start(cfg: Config, events: Sender<AppEvent>) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = stop.clone();

    thread::spawn(move || {
        if let Err(error) = run(cfg, stop_for_thread, events.clone()) {
            let _ = events.send(AppEvent::Error(format!("{error:#}")));
        }
    });

    stop
}

fn run(cfg: Config, stop: Arc<AtomicBool>, events: Sender<AppEvent>) -> Result<()> {
    cfg.check_whisper()?;
    cfg.check_audio()?;

    let program = cfg.audio.command.first().context("[audio].command 为空")?;
    let mut child = Command::new(program)
        .args(&cfg.audio.command[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| {
            format!(
                "启动录音命令 {} 失败（NixOS 下请确认 pipewire 正在运行）",
                program
            )
        })?;

    let pid = child.id() as libc::pid_t;
    let finished = Arc::new(AtomicBool::new(false));

    let watcher = {
        let stop = stop.clone();
        let finished = finished.clone();
        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) && !finished.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(50));
            }
            if stop.load(Ordering::SeqCst) && !finished.load(Ordering::SeqCst) {
                unsafe {
                    libc::kill(pid, libc::SIGINT);
                }
            }
        })
    };

    let mut stdout = child.stdout.take().context("打开录音进程 stdout 失败")?;
    let (chunk_tx, chunk_rx) = mpsc::channel();
    let stt_cfg = cfg.clone();
    let stt_events = events.clone();
    let transcriber = thread::spawn(move || transcribe::worker(stt_cfg, chunk_rx, stt_events));

    let chunk_len = cfg.audio.sample_rate as usize * cfg.audio.chunk_secs as usize;
    let overlap = cfg.audio.sample_rate as usize * cfg.audio.overlap_secs as usize;
    let min_final_samples = (cfg.audio.sample_rate as usize / 2).max(1);

    let mut samples: Vec<i16> = Vec::new();
    let mut pending_byte: Option<u8> = None;
    let mut chunk_index = 0usize;
    let mut aborted = false;
    let mut buffer = [0u8; 8192];

    loop {
        match stdout.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                let mut start = 0;
                if let Some(low) = pending_byte.take() {
                    samples.push(i16::from_le_bytes([low, buffer[0]]));
                    start = 1;
                }

                let usable = (n - start) / 2 * 2;
                for pair in buffer[start..start + usable].as_chunks::<2>().0 {
                    samples.push(i16::from_le_bytes(*pair));
                }
                if (n - start) % 2 == 1 {
                    pending_byte = Some(buffer[n - 1]);
                }

                if !emit_ready(&samples, &mut chunk_index, chunk_len, overlap, &chunk_tx) {
                    aborted = true;
                    break;
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => {
                eprintln!("phoenix-stt: 读取录音数据失败: {error}");
                aborted = true;
                break;
            }
        }
    }

    if aborted {
        unsafe {
            libc::kill(pid, libc::SIGINT);
        }
    }

    let final_start = if chunk_index == 0 {
        0
    } else {
        chunk_index * chunk_len - overlap
    };
    let remaining = samples.len().saturating_sub(final_start);
    if remaining >= min_final_samples {
        let _ = chunk_tx.send(Chunk {
            index: chunk_index,
            samples: samples[final_start..].to_vec(),
        });
    }

    drop(chunk_tx);
    finished.store(true, Ordering::SeqCst);

    let status = child.wait().context("等待录音进程退出失败")?;
    let _ = watcher.join();
    let _ = transcriber.join();

    if !stop.load(Ordering::SeqCst)
        && !status.success()
        && remaining < min_final_samples
        && samples.is_empty()
    {
        bail!("录音进程提前退出（{status}），没有采集到音频");
    }

    Ok(())
}

fn emit_ready(
    samples: &[i16],
    chunk_index: &mut usize,
    chunk_len: usize,
    overlap: usize,
    tx: &Sender<Chunk>,
) -> bool {
    while samples.len() >= (*chunk_index + 1) * chunk_len {
        let start = if *chunk_index == 0 {
            0
        } else {
            *chunk_index * chunk_len - overlap
        };
        let end = (*chunk_index + 1) * chunk_len;
        if tx
            .send(Chunk {
                index: *chunk_index,
                samples: samples[start..end].to_vec(),
            })
            .is_err()
        {
            return false;
        }
        *chunk_index += 1;
    }
    true
}
