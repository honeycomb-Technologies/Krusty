//! Outer terminal lifecycle guard for TUI v2.

use std::io::{self, Stdout, Write};

use anyhow::Result;
use crossterm::{
    cursor::{Hide, Show},
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

type AppTerminal = Terminal<CrosstermBackend<Stdout>>;

pub struct TerminalSession {
    terminal: AppTerminal,
}

impl TerminalSession {
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;

        let mut stdout = io::stdout();
        if let Err(error) = enter_terminal_modes(&mut stdout) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }

        let backend = CrosstermBackend::new(stdout);
        let terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(error) => {
                restore_terminal_modes();
                return Err(error.into());
            }
        };

        Ok(Self { terminal })
    }

    pub fn draw<F>(&mut self, render: F) -> io::Result<()>
    where
        F: FnOnce(&mut ratatui::Frame),
    {
        self.terminal.draw(render).map(|_| ())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = leave_terminal_modes(self.terminal.backend_mut());
        let _ = disable_raw_mode();
    }
}

fn restore_terminal_modes() {
    let _ = disable_raw_mode();
    let _ = leave_terminal_modes(&mut io::stdout());
}

fn enter_terminal_modes(writer: &mut impl Write) -> io::Result<()> {
    execute!(
        writer,
        Hide,
        EnterAlternateScreen,
        EnableFocusChange,
        EnableMouseCapture,
        EnableBracketedPaste
    )
}

fn leave_terminal_modes(writer: &mut impl Write) -> io::Result<()> {
    execute!(
        writer,
        Show,
        LeaveAlternateScreen,
        DisableFocusChange,
        DisableMouseCapture,
        DisableBracketedPaste
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_sequences_restore_every_enabled_terminal_mode() {
        let mut enter = Vec::new();
        enter_terminal_modes(&mut enter).expect("enter modes");
        let enter = String::from_utf8(enter).expect("terminal escapes are UTF-8");
        assert!(enter.contains("\x1b[?25l"), "cursor was not hidden");
        assert!(
            enter.contains("\x1b[?1049h"),
            "alternate screen was not entered"
        );
        assert!(
            enter.contains("\x1b[?1004h"),
            "focus reporting was not enabled"
        );
        assert!(
            enter.contains("\x1b[?1000h"),
            "mouse capture was not enabled"
        );
        assert!(
            enter.contains("\x1b[?2004h"),
            "bracketed paste was not enabled"
        );

        let mut leave = Vec::new();
        leave_terminal_modes(&mut leave).expect("leave modes");
        let leave = String::from_utf8(leave).expect("terminal escapes are UTF-8");
        assert!(leave.contains("\x1b[?25h"), "cursor was not restored");
        assert!(
            leave.contains("\x1b[?1049l"),
            "alternate screen was not left"
        );
        assert!(
            leave.contains("\x1b[?1004l"),
            "focus reporting was not disabled"
        );
        assert!(
            leave.contains("\x1b[?1000l"),
            "mouse capture was not disabled"
        );
        assert!(
            leave.contains("\x1b[?2004l"),
            "bracketed paste was not disabled"
        );
    }
}
