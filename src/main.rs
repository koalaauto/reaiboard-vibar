//! Vibar: a small macOS client for the ReAI Vibe Board keyboard.
//! It connects over Bluetooth, maps the knob, keys and lever to system shortcuts and lives in the menu bar.
//! It does not need the vendor app. The only thing it ever writes to the keyboard is the sleep time, on request.
//!
//! Usage: vibar [--headless] [--dry-run] [--secs N]
//!   --headless  no menu bar icon, run in the terminal (for debugging)
//!   --dry-run   print the actions instead of pressing keys
//!   --secs N    with --headless only: exit after N seconds
//!   --set-sleep-connected <seconds>     write the sleep time while connected, then exit (implies --headless)
//!   --set-sleep-disconnected <seconds>  write the sleep time after disconnecting, then exit (implies --headless)

mod actions;
mod autostart;
mod config;
mod coreaudio;
mod daemon;
mod i18n;
mod logfile;
mod mic;
mod settings;
mod setup;
mod ui;

use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use daemon::{Bindings, Command, Status};
use i18n::Msg;
use ui::{Handlers, UiEvent};

fn main() {
    // First of all, so that nothing printed later is lost when the app is started from Finder.
    let to_file = logfile::init();
    let mut logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"));
    if to_file {
        logger.format_timestamp(None); // the log file stamps every line itself
    } else {
        logger.format_timestamp_millis();
    }
    logger.init();

    if let Err(e) = run() {
        eprintln!("Error: {e:#}");
        logfile::drain();
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
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

    // Diagnostics: ask CoreAudio about the virtual microphone, read only. No keyboard, no single-instance lock.
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

    let migrated = config::migrate_legacy();
    // Before anything creates the default config: an existing file means this is not a first run.
    let first_run_state = setup::at_startup(config::config_path().exists());
    // Language first, so that everything reported below is in the right language.
    i18n::set(i18n::resolve(config::peek_language().as_deref()));
    let _lock = single_instance()?;
    // Only now, so that a second copy of the app, which exits at once, leaves the log alone.
    logfile::announce();
    if let Some((old, new)) = migrated {
        println!(
            "Migrated the old config from {} to {}",
            old.display(),
            new.display()
        );
    }

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
        // One-shot setting: send the change when the current value is first read, exit once the device reports the new one.
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
                    logfile::drain();
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
                let _ = proxy
                    .lock()
                    .unwrap()
                    .send_event(UiEvent::ConfigProblem(Some(n)));
            }
            let p = proxy.clone();
            let exec = actions::spawn_executor(dry_run, move |ok| {
                let _ = p.lock().unwrap().send_event(UiEvent::Trusted(ok));
            })
            .expect("failed to start the actions thread");

            let status: Arc<dyn Fn(Status) + Send + Sync> = Arc::new(move |s| {
                let _ = proxy.lock().unwrap().send_event(UiEvent::Status(s));
            });
            // UI demo: no keyboard, cycle through the icon states so each one can be checked by eye or in a screenshot.
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
        first_run_state,
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
            // A reload that worked is not worth a word in the menu. Only a problem is reported, and
            // the next good reload clears it.
            let problem = match daemon::load_bindings() {
                Ok((_, map, bad)) => {
                    println!(
                        "[config] reloaded, {} bindings, {} invalid",
                        map.count(),
                        bad.len()
                    );
                    *bindings.write().unwrap() = map;
                    bad.first().map(|first| {
                        Msg::ConfigUpdatedWithInvalid {
                            count: bad.len(),
                            first: first.clone(),
                        }
                        .text()
                    })
                }
                Err(e) => {
                    println!("[config] {e:#}");
                    Some(Msg::KeepingPrevious(e.to_string()).text())
                }
            };
            let proxy = proxy.lock().unwrap();
            if language_changed {
                let _ = proxy.send_event(UiEvent::LanguageChanged);
            }
            let _ = proxy.send_event(UiEvent::ConfigProblem(problem));
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
