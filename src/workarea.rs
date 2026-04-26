// src/workarea.ts
use crossterm::{
    cursor::{self, Hide, MoveTo, Show},
    event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers},
    execute, queue,
    style::{Print, Stylize},
    terminal::{self, Clear, ClearType, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use std::cell::RefCell;
use std::io::{self, Write};
use tokio::time::{Duration, Instant};
use unicode_width::UnicodeWidthChar;

const WORKAREA_HEIGHT: usize = 5;
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Current input phase of the WorkArea (used for dynamic status hints).
#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Phase {
    /// Waiting for user input at the prompt.
    Input,
    /// Agent is processing; WorkArea is disabled.
    Processing,
    /// Interrupt requested by user.
    Interrupted,
}

/// Action resulting from processing a keyboard event.
enum Action {
    None,
    Submit(String),
    Interrupt,
    Exit,
}

/// Events produced by the workarea event loop.
pub enum WorkAreaEvent {
    /// User submitted a line by pressing Enter.
    Submit(String),
    /// Interrup (ESC or single Ctrl+C).
    Interrupt,
    /// User signaled exit (double Ctrl+C).
    Exit,
}

/// Manages the TUI workarea: a prompt box with separator lines and output area.
///
/// Single-threaded — uses `RefCell` for interior mutability. All methods take `&self`.
///
/// `tick()` blocks until a meaningful keyboard event (Submit/Interrupt/Exit) occurs.
/// Call it in a `tokio::select!` alongside an agent event receiver.
pub struct WorkArea {
    /// Row index of the first (blank) line of the workarea
    start_row: RefCell<usize>,
    /// Current input buffer (characters)
    input_chars: RefCell<Vec<char>>,
    /// Current cursor position in character indices
    cursor_pos: RefCell<usize>,
    /// Horizontal scroll offset for long input lines
    scroll_offset: RefCell<usize>,
    /// Timestamp of last Ctrl+C press (for double-press exit)
    last_interrupt: RefCell<Option<Instant>>,
    /// Time threshold for recognizing a double Ctrl+C
    interrupt_threshold: Duration,
    /// Stdout handle for writing terminal commands
    stdout: RefCell<io::Stdout>,
    /// Custom status content (set via `set_status`)
    status: RefCell<String>,
    /// Current input phase for dynamic hints
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

    /// Block until a meaningful keyboard event occurs.
    ///
    /// Draws the workarea, awaits keyboard input, and returns a [`WorkAreaEvent`]
    /// for Submit, Interrupt, or Exit. Loops internally for other key events.
    pub async fn tick(&self) -> io::Result<WorkAreaEvent> {
        self.draw_frame()?;

        // Create once outside the loop — recreating on each iteration re-registers
        // the terminal epoll/kqueue watcher, causing missed wakeups in tokio::select!
        // and stalling the TUI while the agent streams output.
        let mut stream = EventStream::new();

        loop {
            if let Some(Ok(Event::Key(key_event))) = stream.next().await {
                let result = self.process_key_event(key_event)?;
                match result {
                    Action::Submit(line) => return Ok(WorkAreaEvent::Submit(line)),
                    Action::Interrupt => return Ok(WorkAreaEvent::Interrupt),
                    Action::Exit => return Ok(WorkAreaEvent::Exit),
                    Action::None => {
                        // Redraw after state change and keep waiting
                        self.draw_frame()?;
                    }
                }
            }
        }
    }

    /// Print multi-line content at the top of workarea.
    /// Returns the number of lines printed. Updates `start_row` accordingly.
    pub fn print<T: std::fmt::Display>(&self, line: T) -> io::Result<usize> {
        let content = format!("{}", line);
        let raw_lines: Vec<&str> = content.lines().collect();

        let (width, height) = terminal::size()?;
        let width = width as usize;
        let height = height as usize;

        // Word-wrap all lines to terminal width (minus 2 for continuation indentation)
        let wrap_width = width.saturating_sub(2);
        let lines: Vec<String> = raw_lines.iter().flat_map(|l| wrap_line(l, wrap_width)).collect();

        let mut stdout = self.stdout.borrow_mut();
        let start_row = *self.start_row.borrow();

        queue!(stdout, MoveTo(0, start_row as u16))?;

        for (i, s) in lines.iter().enumerate() {
            let formatted = if i == 0 {
                s.clone()
            } else {
                format!("  {}", s)
            };
            queue!(
                stdout,
                Clear(ClearType::CurrentLine),
                Print(formatted),
                Print("\r\n"),
            )?;
        }

        // Position cursor at the last printed line's end — no extra blank line
        // between consecutive print() calls (that would add gaps in streaming output).
        // draw_frame_inner() redraws the workarea below, so no scrollback sentinel needed.
        queue!(stdout, MoveTo(0, (start_row + lines.len()) as u16))?;

        let printed = lines.len();
        let new_start = start_row + printed;

        // Push newlines to scroll the terminal when workarea would go off-screen,
        // then clamp start_row and redraw the workarea.
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

        // Redraw workarea after printing so it's always visible
        self.draw_frame_inner(&mut stdout)?;
        stdout.flush()?;
        Ok(printed)
    }

    /// Update the custom status text shown in the status bar.
    pub fn set_status(&self, status: String) {
        *self.status.borrow_mut() = status;
    }

    /// Update the current input phase (affects dynamic hints).
    #[allow(dead_code)]
    pub fn set_phase(&self, phase: Phase) {
        *self.phase.borrow_mut() = phase;
    }

    /// Redraw the workarea UI (useful after updating status/phase).
    #[allow(dead_code)]
    pub fn redraw(&self) {
        let _ = self.draw_frame();
    }

    /// Internal: draw a single frame of the workarea UI.
    /// Pre-computes all strings outside the stdout lock to minimize contention.
    fn draw_frame(&self) -> io::Result<()> {
        // Pre-compute everything that needs other locks / heavy work
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

            // Keep workarea within terminal bounds
            let start_row = if start_row_base + WORKAREA_HEIGHT > height {
                let scroll_needed = (start_row_base + WORKAREA_HEIGHT) - height;
                *self.start_row.borrow_mut() = start_row_base - scroll_needed;
                start_row_base - scroll_needed
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

            // Build visible text with scroll offset
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

        // Now acquire stdout lock only for fast queue operations
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
        stdout.flush()?;

        Ok(())
    }

    /// Internal: draw workarea frame (stdout lock must already be held).
    /// Used by `print()` to redraw the workarea after streaming content.
    fn draw_frame_inner(&self, stdout: &mut io::Stdout) -> io::Result<()> {
        let start_row = *self.start_row.borrow();
        let cursor_pos = *self.cursor_pos.borrow();
        let scroll_offset = *self.scroll_offset.borrow();

        let input_chars = self.input_chars.borrow();

        let (width, height) = terminal::size()?;
        let width = width as usize;
        let height = height as usize;
        let inner_width = width.saturating_sub(4);

        // Keep workarea within terminal bounds
        let mut start_row = start_row;
        if start_row + WORKAREA_HEIGHT > height {
            let scroll_needed = (start_row + WORKAREA_HEIGHT) - height;
            queue!(stdout, MoveTo(0, height as u16 - 1))?;
            for _ in 0..scroll_needed {
                execute!(stdout, Print("\n"))?;
            }
            *self.start_row.borrow_mut() = start_row - scroll_needed;
            start_row -= scroll_needed;
        }

        // Calculate visual cursor position with scrolling
        let mut visual_cursor_col = 0;
        for i in 0..cursor_pos {
            visual_cursor_col += input_chars[i].width().unwrap_or(0);
        }

        let scroll_offset = if visual_cursor_col < scroll_offset {
            visual_cursor_col
        } else if visual_cursor_col >= scroll_offset + inner_width {
            visual_cursor_col - inner_width + 1
        } else {
            scroll_offset
        };

        // Build visible text with scroll offset
        let mut visible_text = String::new();
        let mut current_visual_col = 0;
        for ch in &*input_chars {
            let ch_width = ch.width().unwrap_or(0);
            if current_visual_col >= scroll_offset
                && current_visual_col + ch_width <= scroll_offset + inner_width
            {
                visible_text.push(*ch);
            }
            current_visual_col += ch_width;
        }

        // Build dynamic status line
        let status_text = self.status.borrow().clone();
        let phase = *self.phase.borrow();
        let status_line = Self::build_status_line(phase, input_chars.len(), &status_text, width);

        let sep = "─".repeat(width);
        queue!(
            stdout,
            Hide,
            MoveTo(0, start_row as u16),
            Clear(ClearType::CurrentLine),
            MoveTo(0, (start_row + 1) as u16),
            Print(sep.clone().dark_grey()),
            MoveTo(0, (start_row + 2) as u16),
            Clear(ClearType::CurrentLine),
            Print(format!("{} ", "❯".grey())),
            Print(&visible_text),
            MoveTo(0, (start_row + 3) as u16),
            Print(sep.dark_grey()),
            MoveTo(0, (start_row + 4) as u16),
            Clear(ClearType::CurrentLine),
            Print(status_line.dark_grey()),
            MoveTo(
                (visual_cursor_col - scroll_offset + 2) as u16,
                (start_row + 2) as u16
            ),
            Show,
        )?;

        Ok(())
    }

    // --- Private helpers ---

    /// Build the status bar content: contextual hints on the left, status + version on the right.
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
        let padding = if pad_len > 1 {
            " ".repeat(pad_len)
        } else {
            String::from(" ")
        };
        format!("{}{}{}", hint, padding, right)
    }

    /// Process a keyboard event, updating internal state.
    fn process_key_event(&self, key_event: KeyEvent) -> io::Result<Action> {
        match key_event.code {
            KeyCode::Esc => {
                self.graceful_exit()?;
                return Ok(Action::Exit);
            }
            KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                let now = Instant::now();

                let should_exit = {
                    let mut last = self.last_interrupt.borrow_mut();
                    if let Some(last_instant) = *last {
                        if now.duration_since(last_instant) < self.interrupt_threshold {
                            true
                        } else {
                            *last = Some(now);
                            false
                        }
                    } else {
                        *last = Some(now);
                        false
                    }
                };

                if should_exit {
                    self.graceful_exit()?;
                    return Ok(Action::Exit);
                }

                let line = format!("{} interrupt (press again to exit)", "⛉".yellow());
                self.print(line)?;

                return Ok(Action::Interrupt);
            }
            KeyCode::Left => {
                *self.last_interrupt.borrow_mut() = None;
                *self.cursor_pos.borrow_mut() = self.cursor_pos.borrow().saturating_sub(1);
            }
            KeyCode::Right => {
                *self.last_interrupt.borrow_mut() = None;
                let cursor_pos = *self.cursor_pos.borrow();
                let input_len = self.input_chars.borrow().len();
                if cursor_pos < input_len {
                    *self.cursor_pos.borrow_mut() = cursor_pos + 1;
                }
            }
            KeyCode::Backspace => {
                *self.last_interrupt.borrow_mut() = None;
                let cursor_pos = *self.cursor_pos.borrow();
                if cursor_pos > 0 {
                    self.input_chars.borrow_mut().remove(cursor_pos - 1);
                    *self.cursor_pos.borrow_mut() = cursor_pos - 1;
                }
            }
            KeyCode::Char(c) => {
                *self.last_interrupt.borrow_mut() = None;
                let cursor_pos = *self.cursor_pos.borrow();
                self.input_chars.borrow_mut().insert(cursor_pos, c);
                *self.cursor_pos.borrow_mut() = cursor_pos + 1;
            }
            KeyCode::Enter => {
                let submitted: String = self.input_chars.borrow().iter().collect();
                let submitted = submitted.trim();
                if !submitted.is_empty() {
                    let line = format!("{} {}\n\n", "❯".dark_grey(), submitted);
                    self.print(line)?;

                    self.input_chars.borrow_mut().clear();
                    *self.cursor_pos.borrow_mut() = 0;
                    *self.scroll_offset.borrow_mut() = 0;

                    return Ok(Action::Submit(submitted.into()));
                }
            }
            _ => {
                *self.last_interrupt.borrow_mut() = None;
            }
        }

        Ok(Action::None)
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

/// Word-wrap a single line to fit within `max_width` character columns
/// using textwrap (breaks on whitespace, handles unicode widths).
fn wrap_line(line: &str, max_width: usize) -> Vec<String> {
    textwrap::wrap(line, textwrap::Options::new(max_width))
        .into_iter()
        .map(|s| s.to_string())
        .collect()
}
