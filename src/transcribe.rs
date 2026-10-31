use std::process::Command;
use std::sync::mpsc::{Receiver, Sender};

use anyhow::{bail, Context, Result};
use hound::{SampleFormat, WavSpec, WavWriter};
use tempfile::Builder;

use crate::audio::Chunk;
use crate::config::Config;
use crate::{clipboard, llm, AppEvent};

pub fn worker(cfg: Config, chunks: Receiver<Chunk>, events: Sender<AppEvent>) {
    let mut parts = Vec::new();

    for chunk in chunks {
        match transcribe_chunk(&cfg, &chunk.samples) {
            Ok(text) if !text.is_empty() => parts.push(text),
            Ok(_) => {}
            Err(error) => eprintln!("phoenix-stt: 第 {} 段转写失败: {error:#}", chunk.index + 1),
        }
    }

    let transcript = parts.join("\n");
    if transcript.trim().is_empty() {
        let _ = events.send(AppEvent::Error(
            "没有识别到语音。请确认麦克风正常、whisper 模型和 whisper-cli 可用，运行 `phoenix-stt doctor` 检查。"
                .to_string(),
        ));
        return;
    }

    let mut notice = None;
    let final_text = match llm::polish(&cfg, &transcript) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("phoenix-stt: LLM 润色失败，已使用原始转写\n{error:#}");
            notice = Some("LLM 不可用，已复制原始转写".to_string());
            transcript
        }
    };

    if let Err(error) = clipboard::copy(&cfg, &final_text) {
        let _ = events.send(AppEvent::Error(format!(
            "复制到剪贴板失败: {error:#}\n运行 `phoenix-stt doctor` 检查剪贴板工具。"
        )));
        return;
    }

    let _ = events.send(AppEvent::Done);
    if let Some(notice) = notice {
        let _ = events.send(AppEvent::Notice(notice));
    }
}

fn transcribe_chunk(cfg: &Config, samples: &[i16]) -> Result<String> {
    let temp = Builder::new()
        .prefix("phoenix-stt-")
        .suffix(".wav")
        .tempfile()
        .context("创建临时 WAV 失败")?;

    let spec = WavSpec {
        channels: 1,
        sample_rate: cfg.audio.sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };

    {
        let mut writer = WavWriter::create(temp.path(), spec).context("写入临时 WAV 失败")?;
        for sample in samples {
            writer.write_sample(*sample).context("写入音频采样失败")?;
        }
        writer.finalize().context("完成临时 WAV 失败")?;
    }

    let output = Command::new(&cfg.whisper.bin)
        .arg("-m")
        .arg(&cfg.whisper.model)
        .arg("-f")
        .arg(temp.path())
        .arg("-l")
        .arg(&cfg.whisper.language)
        .arg("-t")
        .arg(cfg.whisper.threads.to_string())
        .arg("-nt")
        .arg("-np")
        .output()
        .with_context(|| format!("启动 {} 失败", cfg.whisper.bin))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("whisper 退出码 {}: {}", output.status, stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(clean_transcript(&stdout))
}

fn clean_transcript(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| {
            !(line.starts_with('[') && line.ends_with(']'))
                && !(line.starts_with('(') && line.ends_with(')'))
        })
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}
