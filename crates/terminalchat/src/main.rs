mod app;
mod bbcode;
mod commands;
mod config;
mod events;
mod images;
mod input;
mod pager;
mod paths;
mod presence;
mod rows;
mod sanitize;
mod screens;
mod store;
mod stores;
mod style;
mod ui;

use std::io::{stdout, Write};
use std::time::Duration;

use ratatui::crossterm::event::{self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture};
use ratatui::crossterm::execute;
use tokio::sync::mpsc;

use app::{App, AppEvent};
use config::Config;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_help() {
    println!("terminalchat {VERSION} - terminal client for F-Chat (f-list.net)");
    println!();
    println!("Usage: terminalchat [--version | --help]");
    println!();
    println!("Environment:");
    println!("  TERMINALCHAT_CONFIG  path of config.json");
    println!("  TERMINALCHAT_DATA    data directory (logs, pins, ...)");
}

fn restore_terminal() {
    let _ = execute!(stdout(), DisableBracketedPaste, DisableMouseCapture);
    ratatui::restore();
}

fn spawn_input(tx: mpsc::UnboundedSender<AppEvent>) {
    std::thread::spawn(move || {
        while let Ok(event) = event::read() {
            if tx.send(AppEvent::Term(event)).is_err() {
                break;
            }
        }
    });
}

fn spawn_ticker(tx: mpsc::UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            if tx.send(AppEvent::Tick).is_err() {
                break;
            }
        }
    });
}

fn minimize(app: &mut App) {
    if std::env::var_os("TMUX").is_none() {
        app.warn("/minimize needs the 'terminalchat' launcher (runs inside tmux)");
        return;
    }
    let status = std::process::Command::new("tmux")
        .arg("detach-client")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if !status.is_ok_and(|status| status.success()) {
        app.notify("Could not detach from tmux", app::Severity::Error);
    }
}

fn handle(app: &mut App, event: AppEvent) {
    match event {
        AppEvent::Term(event) => events::handle(app, event),
        AppEvent::Tick => app.on_tick(),
        AppEvent::Server(generation, event) => app.on_server(generation, event),
        AppEvent::Ticket(purpose, result) => app.on_ticket(purpose, result),
        AppEvent::Connected(generation, result) => app.on_connected(generation, result),
        AppEvent::Profile(name, result) => app.on_profile(&name, result),
        AppEvent::Image(key, frames) => app.images.on_loaded(key, frames),
        AppEvent::Attached(attached) => app.images.attached = attached,
    }
}

async fn run(config: Config) -> std::io::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut app = App::new(config, tx.clone());
    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    let picker = images::detect_picker(&app.config);
    app.images.configure(picker, &app.config);
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));
    spawn_input(tx.clone());
    spawn_ticker(tx);
    app.start();

    loop {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;
        for sequence in app.osc_queue.drain(..) {
            let mut out = stdout();
            out.write_all(sequence.as_bytes())?;
            out.flush()?;
        }
        let event = match app.images.next_wake() {
            Some(due) => tokio::select! {
                event = rx.recv() => event,
                () = tokio::time::sleep_until(tokio::time::Instant::from_std(due)) => continue,
            },
            None => rx.recv().await,
        };
        let Some(event) = event else { break };
        handle(&mut app, event);
        while let Ok(event) = rx.try_recv() {
            handle(&mut app, event);
        }
        if app.minimize_requested {
            app.minimize_requested = false;
            minimize(&mut app);
        }
        if app.should_quit {
            break;
        }
    }
    app.save_files();
    restore_terminal();
    Ok(())
}

#[tokio::main]
async fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => {
            println!("terminalchat {VERSION}");
            return;
        }
        Some("--help" | "-h") => {
            print_help();
            return;
        }
        Some(other) => {
            eprintln!("unknown option: {other}");
            print_help();
            std::process::exit(2);
        }
        None => {}
    }
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let config = match Config::load(&paths::config_file()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = run(config).await {
        restore_terminal();
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
