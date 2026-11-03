//! Vibar:ReAI Vibe Board 的精简 macOS 客户端。
//! 蓝牙直连键盘,把旋钮 / 按键 / 拨杆映射成系统快捷键,菜单栏常驻。
//! 不依赖官方 App。只有在你主动修改「休眠时间」时才会向键盘写入设置,其余全部只读。
//!
//! 用法:vibar [--headless] [--dry-run] [--secs N]
//!   --headless 不显示菜单栏图标,在终端里跑(调试用)
//!   --dry-run  只打印将要执行的动作,不真的按键
//!   --secs N   仅 --headless:运行 N 秒后退出
//!   --set-sleep-connected <秒>     一次性写入「已连接时休眠」后退出(隐含 --headless)
//!   --set-sleep-disconnected <秒>  一次性写入「断开后休眠」后退出(隐含 --headless)

mod actions;
mod autostart;
mod config;
mod coreaudio;
mod daemon;
mod i18n;
mod mic;
mod settings;
mod ui;

use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use daemon::{Bindings, Command, Status};
use i18n::Msg;
use ui::{Handlers, UiEvent};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .format_timestamp_millis()
        .init();

    let args: Vec<String> = std::env::args().collect();
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let arg_u16 = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .and_then(|s| s.parse::<u16>().ok())
    };
    let set_conn = arg_u16("--set-sleep-connected");
    let set_disc = arg_u16("--set-sleep-disconnected");
    let one_shot = set_conn.is_some() || set_disc.is_some();
    let headless = one_shot || args.iter().any(|a| a == "--headless");
    let demo_icons = args.iter().any(|a| a == "--demo-icons");
    let secs = args
        .iter()
        .position(|a| a == "--secs")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u64>().ok());

    // 诊断:只读地查询虚拟麦克风在 CoreAudio 里的状态,不连键盘、不占单实例锁
    if args.iter().any(|a| a == "--probe-audio") {
        let secs = secs.unwrap_or(5);
        for i in 0..secs {
            let id = coreaudio::find_device(reai_board_sdk::virtual_mic::DEVICE_NAME);
            let running = id.and_then(coreaudio::is_running_somewhere);
            println!("[{i}s] device id={id:?} running={running:?}");
            std::thread::sleep(Duration::from_secs(1));
        }
        return Ok(());
    }

    config::migrate_legacy();
    // Language first, so that everything reported below is in the right language.
    i18n::set(i18n::resolve(config::peek_language().as_deref()));
    let _lock = single_instance()?;

    // A broken config at startup must not stop the app: fall back to the built-in defaults and say so in the menu.
    let (cfg, map, startup_notice) = match daemon::load_bindings() {
        Ok((cfg, map, bad)) => {
            let note = bad.first().map(|first| {
                Msg::ConfigInvalid {
                    count: bad.len(),
                    first: first.clone(),
                }
                .text()
            });
            (cfg, map, note)
        }
        Err(e) => {
            eprintln!("[config] {e:#}");
            let default = config::parse(config::DEFAULT_CONFIG)
                .expect("the built-in default config must be valid");
            let (cfg, map, _) = daemon::resolve_all(default);
            (cfg, map, Some(Msg::UsingDefaults(e.to_string()).text()))
        }
    };
    println!(
        "Loaded {} bindings{}",
        map.count(),
        if dry_run { " (dry-run)" } else { "" }
    );
    let bindings: Bindings = Arc::new(RwLock::new(map));
    let device_name = cfg.device.clone();

    if headless {
        let exec = actions::spawn_executor(dry_run, |_| {})?;
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        // 一次性设置:连上后第一次读到当前值时下发修改,等设备回报生效值后退出
        let sent = std::sync::atomic::AtomicBool::new(false);
        let status: Arc<dyn Fn(Status) + Send + Sync> = Arc::new(move |s| {
            if let Status::Error(m) = &s {
                eprintln!("[error] {m}");
            }
            if let (
                true,
                Status::Sleep {
                    connected,
                    disconnected,
                },
            ) = (one_shot, &s)
            {
                if !sent.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    println!("Current: connected {connected}s, disconnected {disconnected}s, writing the new value...");
                    let _ = cmd_tx.send(Command::SetSleep {
                        connected: set_conn,
                        disconnected: set_disc,
                    });
                } else {
                    println!("Effective: connected {connected}s, disconnected {disconnected}s");
                    std::process::exit(0);
                }
            }
        });
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async {
            let run = daemon::run(device_name, bindings, exec, status, cmd_rx);
            match secs {
                Some(s) => {
                    let _ = tokio::time::timeout(Duration::from_secs(s), run).await;
                }
                None => {
                    tokio::select! { _ = run => {}, _ = tokio::signal::ctrl_c() => {} }
                }
            }
        });
        return Ok(());
    }

    let watch_bindings = bindings.clone();
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
    let (tx_mic, tx_remove, tx_rel) = (cmd_tx.clone(), cmd_tx.clone(), cmd_tx.clone());
    let handlers = Handlers {
        set_sleep: Box::new(move |connected, disconnected| {
            let _ = cmd_tx.send(Command::SetSleep {
                connected,
                disconnected,
            });
        }),
        set_mic: Box::new(move |on| {
            let _ = tx_mic.send(Command::SetMic(on));
        }),
        remove_mic: Box::new(move || {
            let _ = tx_remove.send(Command::RemoveMicDriver);
        }),
        set_released: Box::new(move |on| {
            let _ = tx_rel.send(Command::SetReleased(on));
        }),
    };

    ui::run(
        move |proxy| {
            let proxy = Arc::new(Mutex::new(proxy));
            watch_config(watch_bindings, proxy.clone());
            if let Some(n) = startup_notice {
                let _ = proxy.lock().unwrap().send_event(UiEvent::Notice(n));
            }
            let p = proxy.clone();
            let exec = actions::spawn_executor(dry_run, move |ok| {
                let _ = p.lock().unwrap().send_event(UiEvent::Trusted(ok));
            })
            .expect("failed to start the actions thread");

            let status: Arc<dyn Fn(Status) + Send + Sync> = Arc::new(move |s| {
                let _ = proxy.lock().unwrap().send_event(UiEvent::Status(s));
            });
            // 界面演示:不连键盘,循环切换图标状态,用于肉眼 / 截图检查各状态的渲染
            if demo_icons {
                std::thread::spawn(move || loop {
                    for (s, secs) in [
                        (Status::Searching, 3),
                        (Status::Connected, 3),
                        (Status::Voice(true), 4),
                        (Status::Voice(false), 3),
                    ] {
                        status(s);
                        std::thread::sleep(Duration::from_secs(secs));
                    }
                });
                return;
            }
            std::thread::Builder::new()
                .name("daemon".into())
                .spawn(move || {
                    let rt = tokio::runtime::Runtime::new().expect("tokio");
                    if let Err(e) = rt.block_on(daemon::run(
                        device_name,
                        bindings,
                        exec,
                        status.clone(),
                        cmd_rx,
                    )) {
                        eprintln!("[fatal] {e:#}");
                        status(Status::Error(e.to_string()));
                    }
                })
                .expect("failed to start the daemon thread");
        },
        handlers,
    )
}

/// A saved config file takes effect right away: check its modification time once a second.
/// A broken file is only reported and the previous settings stay in force.
fn watch_config(bindings: Bindings, proxy: Arc<Mutex<ui::Proxy>>) {
    let path = config::config_path();
    let mtime = move || std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let mut last = mtime();
    std::thread::Builder::new()
        .name("config-watch".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_secs(1));
            let now = mtime();
            if now == last {
                continue;
            }
            last = now;
            // Language first, so the notice below is in the new language.
            let language_changed = i18n::set(i18n::resolve(config::peek_language().as_deref()));
            let msg = match daemon::load_bindings() {
                Ok((_, map, bad)) => {
                    let n = map.count();
                    *bindings.write().unwrap() = map;
                    match bad.first() {
                        None => Msg::ConfigUpdated(n).text(),
                        Some(first) => Msg::ConfigUpdatedWithInvalid {
                            count: bad.len(),
                            first: first.clone(),
                        }
                        .text(),
                    }
                }
                Err(e) => Msg::KeepingPrevious(e.to_string()).text(),
            };
            println!("[config] {msg}");
            let proxy = proxy.lock().unwrap();
            if language_changed {
                let _ = proxy.send_event(UiEvent::LanguageChanged);
            }
            let _ = proxy.send_event(UiEvent::Notice(msg));
        })
        .expect("failed to start the config watcher thread");
}

/// Single-instance lock: two instances would fight over the same keyboard.
/// The returned handle must be kept alive.
fn single_instance() -> anyhow::Result<std::fs::File> {
    use std::os::fd::AsRawFd;
    let path = config::config_path().with_file_name("vibar.lock");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)?;
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        anyhow::bail!("Vibar is already running, two instances would fight over the keyboard");
    }
    Ok(f)
}
