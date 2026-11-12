//! `terminator-hook inbox`: the attention list in any terminal, for example
//! over SSH from a phone. It reads the daemon snapshot (hook state and
//! notices), shows a session's screen read-only, and sends a reply only
//! after an explicit confirm key. It never launches an agent.
mod keys;
mod rows;
#[cfg(test)]
mod tests;
mod view;

use super::{SendArgs, lookup::Ctl};
use anyhow::Result;
use clap::Parser;
use keys::{Effect, Inbox, Key, Mode};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::{path::PathBuf, time::Duration};
use terminator_core::*;

/// How often the list reloads the daemon snapshot.
const REFRESH: Duration = Duration::from_secs(2);

#[derive(Parser)]
#[command(about = "Attention list of agent sessions; reply only after a confirm key")]
struct InboxCli {
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    runtime_dir: Option<PathBuf>,
}

pub fn run(args: &[String]) -> Result<()> {
    let cli = InboxCli::parse_from(
        std::iter::once("terminator-hook inbox".to_owned()).chain(args.iter().cloned()),
    );
    let mut ctl = Ctl::connect(&super::cli_paths(cli.data_dir, cli.runtime_dir)?)?;
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut ctl);
    ratatui::restore();
    result
}

fn key_of(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        _ => return None,
    })
}

enum Input {
    Quit,
    Key(Key),
    Other,
}

/// Reads one terminal event. Ctrl-C quits from every mode.
fn read_key() -> Result<Input> {
    let Event::Key(press) = event::read()? else {
        return Ok(Input::Other);
    };
    if press.kind != KeyEventKind::Press {
        return Ok(Input::Other);
    }
    if press.modifiers.contains(KeyModifiers::CONTROL) && press.code == KeyCode::Char('c') {
        return Ok(Input::Quit);
    }
    Ok(key_of(press.code).map_or(Input::Other, Input::Key))
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, ctl: &mut Ctl) -> Result<()> {
    let mut inbox = Inbox::new();
    let mut screen = String::new();
    let mut list = rows::rows(&ctl.state, now());
    loop {
        terminal.draw(|frame| view::draw(frame, &inbox, &list, &screen))?;
        if event::poll(REFRESH)? {
            match read_key()? {
                Input::Quit => return Ok(()),
                Input::Key(key) => {
                    inbox.status.clear();
                    let effect = inbox.key(key, &list);
                    if effect == Effect::Quit {
                        return Ok(());
                    }
                    apply(ctl, effect, &mut inbox, &mut screen);
                }
                Input::Other => {}
            }
        } else {
            match Ctl::snapshot(&ctl.paths) {
                Ok(state) => ctl.state = state,
                Err(error) => inbox.status = format!("Daemon unavailable: {error:#}"),
            }
            if let Mode::Screen { session } = &inbox.mode
                && let Ok(text) =
                    ctl.read(session.clone(), ctl.capability(SCREEN_CAPABILITY).is_ok())
            {
                screen = text;
            }
        }
        if inbox.mode == Mode::List {
            screen.clear();
        }
        list = rows::rows(&ctl.state, now());
        inbox.clamp(list.len());
    }
}

/// Runs one effect. A failure shows in the status line; the inbox stays open.
fn apply(ctl: &mut Ctl, effect: Effect, inbox: &mut Inbox, screen: &mut String) {
    let done = match effect {
        Effect::None | Effect::Quit => return,
        Effect::OpenScreen(session) => {
            let full = ctl.capability(SCREEN_CAPABILITY).is_ok();
            let text = ctl.read(session.clone(), full);
            inbox.mode = Mode::Screen { session };
            text.map(|text| {
                *screen = text;
                String::new()
            })
        }
        Effect::Send {
            session,
            text,
            enter,
        } => ctl
            .send(SendArgs {
                session,
                text: Some(text),
                stdin: false,
                enter,
            })
            .map(|_| {
                if enter {
                    "Sent with Enter"
                } else {
                    "Typed without Enter"
                }
                .to_owned()
            }),
        Effect::DismissAgent(id) => notice(ctl, id, "dismiss").map(|()| "Dismissed".to_owned()),
        Effect::Snooze(id) => notice(ctl, id, "snooze").map(|()| "Snoozed".to_owned()),
        Effect::DismissTerminal(id) => ctl.dismiss_notice(id).map(|_| "Dismissed".to_owned()),
    };
    inbox.status = done.unwrap_or_else(|error| format!("{error:#}"));
    if let Ok(state) = Ctl::snapshot(&ctl.paths) {
        ctl.state = state;
    }
}

fn notice(ctl: &Ctl, id: String, action: &str) -> Result<()> {
    rpc(
        &ctl.paths,
        Request::Notice {
            id,
            action: action.to_owned(),
        },
    )?
    .checked()?;
    Ok(())
}
