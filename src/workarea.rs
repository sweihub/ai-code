// src/workarea.rs
use crossterm::{
    cursor::{self, Hide, MoveTo, Show},
    event::{KeyCode, KeyEvent, KeyModifiers},
    execute, queue,
    style::{Print, Stylize},
    terminal::{self, Clear, ClearType, disable_raw_mode, enable_raw_mode},
};
use std::cell::RefCell;
use std::io::{self, Write};
use std::time::Duration;
use tokio::time::Instant;
use unicode_width::UnicodeWidthChar;

const WORKAREA_HEIGHT: usize = 5;
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Current input phase of the WorkArea (used for dynamic status hints).
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Phase {
    Input,
    Processing,
    Interrupted,
}

/// Events produced by processing a keyboard event.
pub enum WorkAreaEvent {
    Submit(String),
    Interrupt,
    Exit,
}

/// Manages the TUI workarea.
///
/// All terminal writes are queued — the caller owns `flush()`.
/// `process_key()` is pure state mutation, no terminal I/O.
pub struct WorkArea {
    start_row: RefCell<usize>,
    input_chars: RefCell<Vec<char>>,
    cursor_pos: RefCell<usize>,
    scroll_offset: RefCell<usize>,
    last_interrupt: RefCell<Option<Instant>>,
    interrupt_threshold: Duration,
    stdout: RefCell<io::Stdout>,
    status: RefCell<String>,
    phase: RefCell<Phase>,
}

impl WorkArea {
    pub fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        let (_, y) = cursor::position()?;
        Ok(Self {
            start_row: RefCell::new(y as usize),
            input_chars: RefCell::new(Vec::new()),
            cursor_pos: RefCell::new(0),
            scroll_offset: RefCell::new(0),
            last_interrupt: RefCell::new(None),
            interrupt_threshold: Duration::from_millis(1000),
            stdout: RefCell::new(io::stdout()),
            status: RefCell::new(String::new()),
            phase: RefCell::new(Phase::Input),
        })
    }

    /// Flush queued terminal writes.
    pub fn flush(&self) -> io::Result<()> {
        self.stdout.borrow_mut().flush()
    }

    /// Process a keyboard event — pure state mutation, no terminal I/O.
    pub fn process_key(&self, key_event: KeyEvent) -> io::Result<Option<WorkAreaEvent>> {
        match key_event.code {
            KeyCode::Esc => {
                self.graceful_exit()?;
                return Ok(Some(WorkAreaEvent::Exit));
            }
            KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                let now = Instant::now();
                let should_exit = {
                    let mut last = self.last_interrupt.borrow_mut();
                    match *last {
                        Some(t) if now.duration_since(t) < self.interrupt_threshold => true,
                        _ => {
                            *last = Some(now);
                            false
                        }
                    }
                };
                if should_exit {
                    self.graceful_exit()?;
                    return Ok(Some(WorkAreaEvent::Exit));
                }
                let line = format!("{} interrupt (press again to exit)", "⛉".yellow());
                self.print_content(&line)?;
                return Ok(Some(WorkAreaEvent::Interrupt));
            }
            KeyCode::Left => {
                *self.last_interrupt.borrow_mut() = None;
                *self.cursor_pos.borrow_mut() = self.cursor_pos.borrow().saturating_sub(1);
            }
            KeyCode::Right => {
                *self.last_interrupt.borrow_mut() = None;
                let cp = *self.cursor_pos.borrow();
                let len = self.input_chars.borrow().len();
                if cp < len {
                    *self.cursor_pos.borrow_mut() = cp + 1;
                }
            }
            KeyCode::Backspace => {
                *self.last_interrupt.borrow_mut() = None;
                let cp = *self.cursor_pos.borrow();
                if cp > 0 {
                    self.input_chars.borrow_mut().remove(cp - 1);
                    *self.cursor_pos.borrow_mut() = cp - 1;
                }
            }
            KeyCode::Char(c) => {
                *self.last_interrupt.borrow_mut() = None;
                let cp = *self.cursor_pos.borrow();
                self.input_chars.borrow_mut().insert(cp, c);
                *self.cursor_pos.borrow_mut() = cp + 1;
            }
            KeyCode::Enter => {
                let submitted: String = self.input_chars.borrow().iter().collect();
                let submitted = submitted.trim();
                if !submitted.is_empty() {
                    let line = format!("{} {}\n\n", "❯".dark_grey(), submitted);
                    self.print_content(&line)?;
                    self.input_chars.borrow_mut().clear();
                    *self.cursor_pos.borrow_mut() = 0;
                    *self.scroll_offset.borrow_mut() = 0;
                    return Ok(Some(WorkAreaEvent::Submit(submitted.into())));
                }
            }
            _ => {
                *self.last_interrupt.borrow_mut() = None;
            }
        }
        Ok(None)
    }

    /// Print content at the top of the workarea. Queues writes, no flush.
    pub fn print<T: std::fmt::Display>(&self, line: T) -> io::Result<usize> {
        self.print_content(&format!("{}", line))
    }

    /// Draw the workarea frame. Queues writes, no flush.
    pub fn draw(&self) -> io::Result<()> {
        self.draw_frame()
    }

    /// Update the status bar text.
    pub fn set_status(&self, status: String) {
        *self.status.borrow_mut() = status;
    }

    /// Update the current input phase.
    #[allow(dead_code)]
    pub fn set_phase(&self, phase: Phase) {
        *self.phase.borrow_mut() = phase;
    }

    // --- Private ---

    fn print_content(&self, content: &str) -> io::Result<usize> {
        let raw_lines: Vec<&str> = content.lines().collect();
        let (width, height) = terminal::size()?;
        let width = width as usize;
        let height = height as usize;
        let wrap_width = width.saturating_sub(2);
        let lines: Vec<String> = raw_lines.iter().flat_map(|l| wrap_line(l, wrap_width)).collect();

        let mut stdout = self.stdout.borrow_mut();
        let start_row = *self.start_row.borrow();

        queue!(stdout, MoveTo(0, start_row as u16))?;
        for (i, s) in lines.iter().enumerate() {
            let formatted = if i == 0 { s.clone() } else { format!("  {}", s) };
            queue!(
                stdout,
                Clear(ClearType::CurrentLine),
                Print(formatted),
                Print("\r\n"),
            )?;
        }
        queue!(stdout, MoveTo(0, (start_row + lines.len()) as u16))?;

        let printed = lines.len();
        let new_start = start_row + printed;

        // Scroll the terminal when workarea would go off-screen
        if new_start + WORKAREA_HEIGHT > height {
            let scroll = new_start + WORKAREA_HEIGHT - height;
            queue!(stdout, MoveTo(0, height as u16 - 1))?;
            for _ in 0..scroll {
                execute!(stdout, Print("\n"))?;
            }
            *self.start_row.borrow_mut() = height - WORKAREA_HEIGHT;
        } else {
            *self.start_row.borrow_mut() = new_start;
        }

        Ok(printed)
    }

    fn draw_frame(&self) -> io::Result<()> {
        let (visible_text, separator, status_line, start_row, cursor_col, need_scroll) = {
            let start_row_base = *self.start_row.borrow();
            let cursor_pos = *self.cursor_pos.borrow();
            let scroll_offset_base = *self.scroll_offset.borrow();

            let input_chars = self.input_chars.borrow();
            let status_text = self.status.borrow().clone();
            let phase = *self.phase.borrow();

            let (width, height) = terminal::size()?;
            let width = width as usize;
            let height = height as usize;
            let inner_width = width.saturating_sub(4);

            let start_row = if start_row_base + WORKAREA_HEIGHT > height {
                let s = (start_row_base + WORKAREA_HEIGHT) - height;
                *self.start_row.borrow_mut() = start_row_base - s;
                start_row_base - s
            } else {
                start_row_base
            };
            let need_scroll = if start_row_base + WORKAREA_HEIGHT > height {
                (start_row_base + WORKAREA_HEIGHT) - height
            } else {
                0
            };

            let mut visual_cursor_col = 0;
            for i in 0..cursor_pos {
                visual_cursor_col += input_chars[i].width().unwrap_or(0);
            }

            let mut new_scroll = scroll_offset_base;
            if visual_cursor_col < scroll_offset_base {
                new_scroll = visual_cursor_col;
            } else if visual_cursor_col >= scroll_offset_base + inner_width {
                new_scroll = visual_cursor_col - inner_width + 1;
            }
            if new_scroll != scroll_offset_base {
                *self.scroll_offset.borrow_mut() = new_scroll;
            }

            let mut visible_text = String::new();
            let mut current_visual_col = 0;
            for ch in &*input_chars {
                let ch_width = ch.width().unwrap_or(0);
                if current_visual_col >= new_scroll
                    && current_visual_col + ch_width <= new_scroll + inner_width
                {
                    visible_text.push(*ch);
                }
                current_visual_col += ch_width;
            }

            let status_line = Self::build_status_line(phase, input_chars.len(), &status_text, width);
            let cursor_col = (visual_cursor_col - new_scroll + 2) as u16;

            (visible_text, "─".repeat(width), status_line, start_row, cursor_col, need_scroll)
        };

        let mut stdout = self.stdout.borrow_mut();
        let row = start_row as u16;

        if need_scroll > 0 {
            let (_, height) = terminal::size()?;
            queue!(stdout, MoveTo(0, (height - 1) as u16))?;
            for _ in 0..need_scroll {
                execute!(stdout, Print("\n"))?;
            }
        }

        let sep = separator.dark_grey();
        let prompt_prefix = format!("{} ", "❯".grey());
        let status = status_line.dark_grey();

        queue!(
            stdout,
            Hide,
            MoveTo(0, row),
            Clear(ClearType::CurrentLine),
            MoveTo(0, row + 1),
            Print(&sep),
            MoveTo(0, row + 2),
            Clear(ClearType::CurrentLine),
            Print(prompt_prefix),
            Print(visible_text),
            MoveTo(0, row + 3),
            Print(&sep),
            MoveTo(0, row + 4),
            Clear(ClearType::CurrentLine),
            Print(status),
            MoveTo(cursor_col, row + 2),
            Show,
        )?;

        Ok(())
    }

    fn build_status_line(
        phase: Phase,
        input_len: usize,
        status: &str,
        term_width: usize,
    ) -> String {
        let hint = match phase {
            Phase::Input if input_len == 0 => "? for shortcuts",
            Phase::Input => "Ctrl+C interrupt  Esc exit",
            Phase::Processing => "waiting for response",
            Phase::Interrupted => "interrupted",
        };
        let right = if status.is_empty() {
            format!("ai v{VERSION}")
        } else {
            format!("{}  ai v{VERSION}", status)
        };
        let pad_len = term_width.saturating_sub(hint.len() + right.len());
        let padding = if pad_len > 1 { " ".repeat(pad_len) } else { " ".into() };
        format!("{}{}{}", hint, padding, right)
    }

    fn graceful_exit(&self) -> io::Result<()> {
        let mut stdout = self.stdout.borrow_mut();
        let start_row = *self.start_row.borrow();
        execute!(
            stdout,
            MoveTo(0, (start_row + WORKAREA_HEIGHT - 1) as u16),
            Print("\n"),
            Show
        )?;
        disable_raw_mode()?;
        Ok(())
    }
}

fn wrap_line(line: &str, max_width: usize) -> Vec<String> {
    textwrap::wrap(line, textwrap::Options::new(max_width))
        .into_iter()
        .map(|s| s.to_string())
        .collect()
}
