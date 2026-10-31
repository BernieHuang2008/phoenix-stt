# Phoenix STT

Phoenix STT 是 phoenix 的语音输入法：按一次快捷键开始录音，再按一次停止。录音按 10 秒切块送 whisper，转写结果拼接后交给本地或外部 OpenAI 兼容 LLM 润色，最后写入剪贴板。

## 功能

- 录音时屏幕中下部显示“录音中”胶囊；LLM 处理时显示“处理中”。
- 每 10 秒切一个音频块；从第二块开始额外带上上一块末尾 1 秒，避免切断字词。
- 每块立即送 `whisper-cli` 识别，不阻塞录音。
- 支持本地 npu-llm，也支持任何 OpenAI 兼容 `chat/completions` endpoint（OpenAI、DeepSeek、OpenRouter 等）。
- API key 只保存在 `~/.config/phoenix-ai/secrets/models.json`，不会出现在 `stt.toml` 或 `phoenix-stt config` 输出中。
- `phoenix-stt doctor` 自动检查模型、依赖、密钥权限和 endpoint 可达性，并给出修复建议。
- LLM 不可用时仍然复制原始 whisper 转写，并用胶囊提示。

## 构建

```bash
cd stt
nix build --offline
./result/bin/phoenix-stt --help
```

仓库内带了 `vendor/`（`cargo vendor` 生成），所以构建不依赖 crates.io。删除 `vendor/` 后 flake 会自动改用 `Cargo.lock` 模式。

开发：

```bash
nix develop
cargo check
cargo clippy
```

## 第一次使用

```bash
# 生成默认配置和密钥模板（不会覆盖已有文件）
phoenix-stt init

# 检查模型、whisper、PipeWire、剪贴板和 LLM endpoint
phoenix-stt doctor

# 启动常驻进程
phoenix-stt daemon
```

另开终端或绑定快捷键：

```bash
phoenix-stt toggle
```

退出 daemon：

```bash
phoenix-stt quit
```

## 配置文件

默认路径：

```text
~/.config/phoenix-ai/config/stt.toml
```

可以用 `--config PATH` 或 `PHOENIX_STT_CONFIG` 指定其他路径。

```toml
[whisper]
model = "/home/bernie/code/phoenix/aitest/models/ggml-small-q5_1.bin"
bin = "whisper-cli"
language = "zh"
threads = 4

[audio]
command = ["pw-record", "--rate", "16000", "--channels", "1", "--format", "s16", "--raw", "-"]
sample_rate = 16000
chunk_secs = 10
overlap_secs = 1

[llm]
# models.json 中的条目名。切换模型只改这里。
model = "local-qwen"

# 可选：直接覆盖 endpoint 和 API model 名（通常留空）
# endpoint = "http://127.0.0.1:4444/v1/chat/completions"
# model_name = "qwen3-4b-thinking"

prompt = "你是一个中文语音输入法的后处理器。请把下面的语音识别结果整理成通顺、自然、适合直接输入到编辑器里的文本：修正错别字和标点，去掉口头禅和重复词，不要添加原文没有的信息。只输出最终文本，不要解释，不要使用 Markdown，不要输出思考过程。"
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
```

## 模型与 API key

默认路径：

```text
~/.config/phoenix-ai/secrets/models.json
```

可以用 `--secrets PATH`、`PHOENIX_STT_SECRETS` 或 `PHOENIX_AI_SECRETS` 指定其他路径。文件权限建议 `600`。

```json
{
  "default": "local-qwen",
  "models": {
    "local-qwen": {
      "endpoint": "http://127.0.0.1:4444/v1/chat/completions",
      "model": "qwen3-4b-thinking"
    },
    "openai": {
      "base_url": "https://api.openai.com/v1",
      "api_key": "sk-...",
      "model": "gpt-4o-mini"
    },
    "deepseek": {
      "base_url": "https://api.deepseek.com/v1",
      "api_key": "sk-...",
      "model": "deepseek-chat",
      "timeout_secs": 120
    }
  }
}
```

说明：

- `endpoint`：完整的 `chat/completions` URL。
- `base_url`：会自动补 `/chat/completions`，例如 `https://api.openai.com/v1`。
- `api_key`：自动加 `Authorization: Bearer <key>`。如果 `headers` 里自己写了 `Authorization`，则不会重复添加。
- `model`：发送给 API 的 model 字段；不写时使用条目名。
- 可选字段：`headers`、`temperature`、`max_tokens`、`timeout_secs`。
- `stt.toml` 的 `[llm].model` 选择这里的一个条目；不写时使用 `default`。
- `PUT_YOUR_TOKEN_HERE` 会被视为未配置，`phoenix-stt doctor` 会提醒。
- 外部 HTTPS endpoint 通过 `curl` 调用，支持系统代理（`http_proxy` / `https_proxy`）。

查看当前配置和可用模型：

```bash
phoenix-stt config
phoenix-stt models
```

## niri 配置

```kdl
spawn-at-startup "/home/bernie/code/phoenix/stt/result/bin/phoenix-stt" "daemon"

binds {
    Mod+V { spawn "/home/bernie/code/phoenix/stt/result/bin/phoenix-stt" "toggle"; }
}
```

如果已经 `nix profile install /home/bernie/code/phoenix/stt`，可以直接写 `phoenix-stt`。

## 常见问题

先运行：

```bash
phoenix-stt doctor
```

### whisper 模型不存在

从 Hugging Face 下载 `ggml-small-q5_1.bin`：

<https://huggingface.co/ggerganov/whisper.cpp/tree/main>

```bash
mkdir -p ~/.local/share/phoenix-ai/models
curl -L -o ~/.local/share/phoenix-ai/models/ggml-small-q5_1.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin
```

然后在 `stt.toml` 中设置 `[whisper].model`，或设置：

```bash
export PHOENIX_STT_WHISPER_MODEL=~/.local/share/phoenix-ai/models/ggml-small-q5_1.bin
```

### 找不到 whisper-cli

NixOS 安装 `whisper-cpp`，或用 flake 构建的包（运行时 PATH 已包含）。

### 找不到 pw-record / 没有声音

确认 PipeWire 正在运行，并且 `pw-record` 在 PATH 中。NixOS 可启用：

```nix
services.pipewire.enable = true;
```

### LLM endpoint 无法访问

- 本地模型：确认 npu-llm 已启动，例如

  ```bash
  npu-llm --model qwen3-4b-thinking --serve 4444 --ctx 2048
  ```

- 外部模型：检查 `models.json` 里的 `endpoint` / `base_url`、`api_key`、网络和代理。
- 运行 `phoenix-stt doctor` 会显示 curl 错误分类（DNS、连接、超时、TLS、证书等）。

### 剪贴板没有内容

安装 `wl-clipboard`，确认 `wl-copy` 在 PATH 中，或在 `stt.toml` 设置 `[clipboard].bin`。

## 环境变量

`PHOENIX_STT_CONFIG`、`PHOENIX_STT_SECRETS`、`PHOENIX_AI_SECRETS`、
`PHOENIX_STT_WHISPER_MODEL`、`PHOENIX_STT_WHISPER_BIN`、`PHOENIX_STT_WHISPER_LANG`、
`PHOENIX_STT_AUDIO_CMD`、`PHOENIX_STT_SAMPLE_RATE`、`PHOENIX_STT_CHUNK_SECS`、
`PHOENIX_STT_OVERLAP_SECS`、`PHOENIX_STT_LLM_MODEL`、`PHOENIX_STT_LLM_MODEL_NAME`、
`PHOENIX_STT_LLM_ENDPOINT`、`PHOENIX_STT_LLM_URL`、`PHOENIX_STT_PROMPT`、
`PHOENIX_STT_CURL_BIN`、`PHOENIX_STT_SOCKET`、`PHOENIX_STT_CLIPBOARD_BIN`、`PHOENIX_STT_UI`。

注意：`PHOENIX_STT_LLM_MODEL` 现在表示 `models.json` 中的条目名；要改发送给 API 的 model 字段，请用 `PHOENIX_STT_LLM_MODEL_NAME`。

## 测试版限制

- whisper 分块有 1 秒重叠，边界处可能出现重复文字，交给 LLM 润色时一般会去掉。
- npu-llm 一次只处理一个请求，处理期间再按快捷键会被忽略。
- 目前只实现了 Wayland / `wl-copy`；X11 需要替换剪贴板命令。
- `[audio].command` 按空格切分，参数里不要带空格。
