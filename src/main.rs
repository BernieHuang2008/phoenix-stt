mod audio;
mod clipboard;
mod config;
mod doctor;
mod init;
mod ipc;
mod llm;
mod tools;
mod transcribe;
mod ui;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use gtk::prelude::*;

use config::Config;

#[derive(Debug)]
pub enum AppEvent {
    Toggle,
    Done,
    Error(String),
    Notice(String),
    Quit,
}

#[derive(Parser)]
#[command(
    name = "phoenix-stt",
    version,
    about = "Phoenix 语音输入法：whisper 转写 + 本地/外部 LLM 润色",
    long_about = "Phoenix STT 是一个面向 NixOS + niri 的语音输入法。\n\n\
        按一次快捷键开始录音，再按一次停止。录音按块送 whisper，转写结果交给\n\
        本地或外部 OpenAI 兼容 chat/completions 接口润色，最后写入剪贴板。\n\n\
        默认配置文件：~/.config/phoenix-ai/config/stt.toml\n\
        默认密钥文件：~/.config/phoenix-ai/secrets/models.json\n\n\
        第一次使用建议依次运行：phoenix-stt init、phoenix-stt doctor、phoenix-stt daemon。",
    after_help = "示例：\n  \
        phoenix-stt init                 生成默认配置和密钥模板\n  \
        phoenix-stt doctor               检查模型、依赖和 LLM endpoint\n  \
        phoenix-stt models               列出 models.json 中的模型\n  \
        phoenix-stt daemon               启动常驻进程\n  \
        phoenix-stt toggle               开始/停止录音\n\n\
        也可以用 --config 和 --secrets 指定其他路径。"
)]
struct Cli {
    /// 配置文件路径（默认 ~/.config/phoenix-ai/config/stt.toml）
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,

    /// models.json 密钥文件路径（默认 ~/.config/phoenix-ai/secrets/models.json）
    #[arg(long, global = true, value_name = "PATH")]
    secrets: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 启动常驻进程（录音、UI、Unix socket）
    #[command(
        long_about = "启动常驻进程。它会监听 Unix socket、管理录音线程，并在\nWayland 下显示一个底部居中的小胶囊。建议由 niri 的 spawn-at-startup 启动。"
    )]
    Daemon,

    /// 通知常驻进程开始/停止录音
    #[command(
        long_about = "向 phoenix-stt daemon 发送 toggle 命令。第一次发送开始录音，\n第二次发送停止录音并进入 whisper + LLM 处理。"
    )]
    Toggle,

    /// 退出常驻进程
    #[command(long_about = "向 phoenix-stt daemon 发送 quit 命令并退出。")]
    Quit,

    /// 打印当前生效配置
    #[command(long_about = "打印实际读取到的配置（TOML）。密钥不会显示。")]
    Config,

    /// 列出 models.json 中的模型
    #[command(
        long_about = "列出 models.json 中定义的模型、endpoint、模型名，以及是否配置了 API key。\n不会打印 key 本身。"
    )]
    Models,

    /// 检查模型、依赖和 LLM endpoint
    #[command(
        long_about = "自动检查常见问题：whisper 模型/程序、录音命令、剪贴板工具、curl、\n配置文件、密钥权限、Wayland 会话以及 LLM endpoint 是否可访问，并给出修复建议。"
    )]
    Doctor,

    /// 生成默认配置和密钥模板
    #[command(
        long_about = "在默认位置生成 stt.toml 和 models.json 模板。已存在的文件默认跳过，\n使用 --force 覆盖。生成的 models.json 权限为 600。"
    )]
    Init {
        /// 覆盖已存在的文件
        #[arg(long)]
        force: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = config::Paths::resolve(cli.config.clone(), cli.secrets.clone());

    match &cli.command {
        Some(Command::Init { force }) => {
            return init::run(&paths.config, &paths.secrets, *force);
        }
        Some(Command::Doctor) => {
            let ok = doctor::run(&paths)?;
            if !ok {
                std::process::exit(1);
            }
            return Ok(());
        }
        _ => {}
    }

    let cfg = Config::load_paths(paths)?;

    match cli.command.unwrap_or(Command::Daemon) {
        Command::Daemon => run_daemon(cfg),
        Command::Toggle => ipc::send_toggle(&cfg.socket_path()),
        Command::Quit => ipc::send_command(&cfg.socket_path(), "quit"),
        Command::Config => {
            println!("# config:   {}", cfg.paths.config.display());
            println!("# secrets:  {}", cfg.paths.secrets.display());
            println!("# selected: {}", cfg.llm_connection.source);
            println!("{}", toml::to_string_pretty(&cfg)?);
            Ok(())
        }
        Command::Models => {
            let models = cfg.list_models()?;
            if models.is_empty() {
                println!("没有在 {} 中找到模型。", cfg.paths.secrets.display());
                println!("运行 `phoenix-stt init` 生成模板，或手动添加 models.json。");
            } else {
                for model in models {
                    println!(
                        "{} {:<20} endpoint={} model={} key={}",
                        if model.selected { "*" } else { " " },
                        model.name,
                        model.endpoint,
                        model.model,
                        if model.has_key { "yes" } else { "no" }
                    );
                }
            }
            Ok(())
        }
        Command::Doctor | Command::Init { .. } => unreachable!(),
    }
}

fn run_daemon(cfg: Config) -> Result<()> {
    if !cfg.paths.config.exists() {
        eprintln!(
            "phoenix-stt: 未找到配置文件 {}，使用默认值。运行 `phoenix-stt init` 生成模板。",
            cfg.paths.config.display()
        );
    }
    if !cfg.paths.secrets.exists() {
        eprintln!(
            "phoenix-stt: 未找到密钥文件 {}（使用本地默认 endpoint 时可以不需要）。",
            cfg.paths.secrets.display()
        );
    }
    if let Err(error) = cfg.check_whisper() {
        eprintln!("phoenix-stt: 警告: {error:#}");
    }
    if let Err(error) = cfg.check_audio() {
        eprintln!("phoenix-stt: 警告: {error:#}");
    }
    if let Err(error) = cfg.check_clipboard() {
        eprintln!("phoenix-stt: 警告: {error:#}");
    }
    if let Err(error) = cfg.check_curl() {
        eprintln!("phoenix-stt: 警告: {error:#}");
    }

    let (tx, rx) = mpsc::channel();

    ipc::spawn_listener(cfg.socket_path(), tx.clone())?;
    eprintln!("phoenix-stt: 监听 {}", cfg.socket_path().display());
    eprintln!("phoenix-stt: whisper 模型 {}", cfg.whisper.model.display());
    eprintln!(
        "phoenix-stt: LLM {}（{}，model={}）",
        cfg.llm_connection.endpoint, cfg.llm_connection.source, cfg.llm_connection.model
    );

    if cfg.ui.enabled {
        run_gtk(cfg, tx, rx)
    } else {
        run_headless(cfg, tx, rx)
    }
}

fn run_headless(cfg: Config, tx: Sender<AppEvent>, rx: Receiver<AppEvent>) -> Result<()> {
    let mut state = State::Idle;
    let mut stop: Option<Arc<AtomicBool>> = None;

    for event in rx {
        match event {
            AppEvent::Toggle => match state {
                State::Idle | State::Notice => {
                    stop = Some(audio::start(cfg.clone(), tx.clone()));
                    state = State::Recording;
                    eprintln!("phoenix-stt: 开始录音");
                }
                State::Recording => {
                    if let Some(stop) = stop.take() {
                        stop.store(true, Ordering::SeqCst);
                    }
                    state = State::Processing;
                    eprintln!("phoenix-stt: 停止录音，处理中");
                }
                State::Processing | State::Error => {}
            },
            AppEvent::Done => {
                stop = None;
                state = State::Idle;
                eprintln!("phoenix-stt: 完成");
            }
            AppEvent::Notice(text) => eprintln!("phoenix-stt: {text}"),
            AppEvent::Error(error) => {
                stop = None;
                state = State::Idle;
                eprintln!("phoenix-stt: 错误: {error}");
            }
            AppEvent::Quit => break,
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Recording,
    Processing,
    Notice,
    Error,
}

struct DaemonState {
    cfg: Config,
    ui: ui::Ui,
    state: State,
    stop: Option<Arc<AtomicBool>>,
}

fn run_gtk(cfg: Config, tx: Sender<AppEvent>, rx: Receiver<AppEvent>) -> Result<()> {
    let app = gtk::Application::builder()
        .application_id("dev.phoenix.stt")
        .build();

    let cfg = Rc::new(cfg);
    let rx_slot = Rc::new(RefCell::new(Some(rx)));
    app.connect_activate(move |app| {
        let Some(rx) = rx_slot.borrow_mut().take() else {
            return;
        };

        let ui = ui::Ui::build(app);
        let state = Rc::new(RefCell::new(DaemonState {
            cfg: (*cfg).clone(),
            ui,
            state: State::Idle,
            stop: None,
        }));

        let loop_state = state.clone();
        let loop_tx = tx.clone();
        let loop_app = app.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            while let Ok(event) = rx.try_recv() {
                match event {
                    AppEvent::Toggle => toggle(&loop_state, &loop_tx),
                    AppEvent::Done => done(&loop_state),
                    AppEvent::Notice(text) => show_notice(&loop_state, text),
                    AppEvent::Error(error) => show_error(&loop_state, error),
                    AppEvent::Quit => {
                        if let Ok(mut state) = loop_state.try_borrow_mut() {
                            if let Some(stop) = state.stop.take() {
                                stop.store(true, Ordering::SeqCst);
                            }
                            state.ui.hide();
                        }
                        loop_app.quit();
                        return glib::ControlFlow::Break;
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    });

    app.run_with_args::<&str>(&[]);
    Ok(())
}

fn toggle(state: &Rc<RefCell<DaemonState>>, tx: &Sender<AppEvent>) {
    let mut state = state.borrow_mut();
    match state.state {
        State::Idle | State::Notice => {
            let stop = audio::start(state.cfg.clone(), tx.clone());
            state.stop = Some(stop);
            state.state = State::Recording;
            state.ui.show("录音中");
        }
        State::Recording => {
            if let Some(stop) = state.stop.take() {
                stop.store(true, Ordering::SeqCst);
            }
            state.state = State::Processing;
            state.ui.show("处理中");
        }
        State::Processing | State::Error => {}
    }
}

fn done(state: &Rc<RefCell<DaemonState>>) {
    let mut state = state.borrow_mut();
    if matches!(state.state, State::Recording | State::Processing) {
        state.stop = None;
        state.state = State::Idle;
        state.ui.hide();
    }
}

fn show_notice(state: &Rc<RefCell<DaemonState>>, text: String) {
    {
        let mut state = state.borrow_mut();
        if !matches!(state.state, State::Idle | State::Notice) {
            return;
        }
        state.state = State::Notice;
        state.ui.show(&text);
    }

    let state = state.clone();
    glib::timeout_add_seconds_local(2, move || {
        let mut state = state.borrow_mut();
        if state.state == State::Notice {
            state.state = State::Idle;
            state.ui.hide();
        }
        glib::ControlFlow::Break
    });
}

fn show_error(state: &Rc<RefCell<DaemonState>>, error: String) {
    eprintln!("phoenix-stt: 错误: {error}");

    {
        let mut state = state.borrow_mut();
        state.stop = None;
        state.state = State::Error;
        state.ui.show(&short_error(&error));
    }

    let state = state.clone();
    glib::timeout_add_seconds_local(2, move || {
        let mut state = state.borrow_mut();
        if state.state == State::Error {
            state.state = State::Idle;
            state.ui.hide();
        }
        glib::ControlFlow::Break
    });
}

fn short_error(error: &str) -> String {
    let compact = error.split_whitespace().collect::<Vec<_>>().join(" ");
    let text: String = compact.chars().take(36).collect();
    if text.chars().count() < compact.chars().count() {
        format!("出错了：{text}…")
    } else {
        format!("出错了：{text}")
    }
}
