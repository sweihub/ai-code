// src/workarea.ts
use crossterm::{
    cursor::{self, Hide, MoveTo, Show},
    event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers},
    execute, queue,
    style::{Print, Stylize},
    terminal::{self, Clear, ClearType, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use std::cell::Cell;
use std::io::{self, Write};
use std::sync::Mutex;
use tokio::time::{Duration, Instant};
use unicode_width::UnicodeWidthChar;

const WORKAREA_HEIGHT: usize = 5;

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
/// `tick()` drives the async event loop: draws the UI, awaits the next keyboard
/// event, processes it, and returns a [`WorkAreaEvent`]. Call it in a loop.
pub struct WorkArea {
    /// Row index of the first (blank) line of the workarea
    start_row: Cell<usize>,
    /// Current input buffer (characters)
    input_chars: Mutex<Vec<char>>,
    /// Current cursor position in character indices
    cursor_pos: Cell<usize>,
    /// Horizontal scroll offset for long input lines
    scroll_offset: Cell<usize>,
    /// Timestamp of last Ctrl+C press (for double-press exit)
    last_interrupt: Mutex<Option<Instant>>,
    /// Time threshold for recognizing a double Ctrl+C
    interrupt_threshold: Duration,
    /// Stdout handle for writing terminal commands
    stdout: Mutex<io::Stdout>,
    /// Event stream from crossterm for reading keyboard input
    reader: Mutex<EventStream>,
}

impl WorkArea {
    pub fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        let stdout = Mutex::new(io::stdout());
        let reader = Mutex::new(EventStream::new());
        let (_, y) = cursor::position()?;
        Ok(Self {
            start_row: Cell::new(y as usize),
            input_chars: Mutex::new(Vec::new()),
            cursor_pos: Cell::new(0),
            scroll_offset: Cell::new(0),
            last_interrupt: Mutex::new(None),
            interrupt_threshold: Duration::from_millis(1000),
            stdout,
            reader,
        })
    }

    /// Draw the workarea UI and run the async event loop.
    ///
    /// Awaits keyboard events, processes them, and returns a [`WorkAreaEvent`]
    /// only for meaningful actions (Submit or Exit). Call it in a loop.
    pub async fn tick(&self) -> io::Result<WorkAreaEvent> {
        self.draw_frame()?;

        loop {
            let event = self.reader.lock().unwrap().next().await;

            if let Some(Ok(Event::Key(key_event))) = event {
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

    /// Print multi-line content at the top of workarea
    pub fn print<T: std::fmt::Display>(&self, line: T) -> io::Result<()> {
        let content = format!("{}", line);
        let lines: Vec<&str> = content.lines().collect();

        let start_row = self.start_row.get();
        let mut stdout = self.stdout.lock().unwrap();
        queue!(stdout, MoveTo(0, start_row as u16))?;

        for (i, s) in lines.iter().enumerate() {
            let formatted = if i == 0 {
                format!("{}", s)
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

        // maintain the top blank line for input to scrollback
        queue!(stdout, Clear(ClearType::CurrentLine), Print("\r\n"))?;
        stdout.flush()?;

        Ok(())
    }

    /// Internal: draw a single frame of the workarea UI.
    fn draw_frame(&self) -> io::Result<()> {
        let mut stdout = self.stdout.lock().unwrap();
        let start_row = self.start_row.get();
        let cursor_pos = self.cursor_pos.get();
        let scroll_offset = self.scroll_offset.get();

        let input_chars = self.input_chars.lock().unwrap();

        let (width, height) = terminal::size()?;
        let width = width as usize;
        let inner_width = width.saturating_sub(4);

        // Keep workarea within terminal bounds
        let mut start_row = start_row;
        if start_row + WORKAREA_HEIGHT > height as usize {
            let scroll_needed = (start_row + WORKAREA_HEIGHT) - height as usize;
            queue!(stdout, MoveTo(0, (height - 1) as u16))?;
            for _ in 0..scroll_needed {
                execute!(stdout, Print("\n"))?;
            }
            self.start_row.set(start_row - scroll_needed);
            start_row = start_row - scroll_needed;
        }

        // Calculate visual cursor position with scrolling
        let mut visual_cursor_col = 0;
        for i in 0..cursor_pos {
            visual_cursor_col += input_chars[i].width().unwrap_or(0);
        }

        let mut new_scroll = scroll_offset;
        if visual_cursor_col < scroll_offset {
            new_scroll = visual_cursor_col;
        } else if visual_cursor_col >= scroll_offset + inner_width {
            new_scroll = visual_cursor_col - inner_width + 1;
        }
        if new_scroll != scroll_offset {
            self.scroll_offset.set(new_scroll);
        }

        let scroll_offset = new_scroll;

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

        queue!(
            stdout,
            Hide,
            // top blank line for input to scrollback
            MoveTo(0, start_row as u16),
            Clear(ClearType::CurrentLine),
            // separator
            MoveTo(0, (start_row + 1) as u16),
            Print("─".repeat(width).dark_grey()),
            // >
            MoveTo(0, (start_row + 2) as u16),
            Clear(ClearType::CurrentLine),
            Print(format!("{} ", "❯".grey())),
            Print(&visible_text),
            // separator
            MoveTo(0, (start_row + 3) as u16),
            Print("─".repeat(width).dark_grey()),
            // status bar
            MoveTo(0, (start_row + 4) as u16),
            Clear(ClearType::CurrentLine),
            Print("status content".dark_grey()),
            // cursor
            MoveTo(
                (visual_cursor_col - scroll_offset + 2) as u16,
                (start_row + 2) as u16
            ),
            Show,
        )?;
        stdout.flush()?;

        Ok(())
    }

    // --- Private helpers ---

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
                    let mut last = self.last_interrupt.lock().unwrap();
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

                let (_, new_y) = cursor::position()?;
                self.start_row.set(new_y as usize);
                return Ok(Action::Interrupt);
            }
            KeyCode::Left => {
                let mut last = self.last_interrupt.lock().unwrap();
                *last = None;
                self.cursor_pos.set(self.cursor_pos.get().saturating_sub(1));
            }
            KeyCode::Right => {
                let mut last = self.last_interrupt.lock().unwrap();
                *last = None;
                let cursor_pos = self.cursor_pos.get();
                let input_len = self.input_chars.lock().unwrap().len();
                if cursor_pos < input_len {
                    self.cursor_pos.set(cursor_pos + 1);
                }
            }
            KeyCode::Backspace => {
                let mut last = self.last_interrupt.lock().unwrap();
                *last = None;
                let cursor_pos = self.cursor_pos.get();
                if cursor_pos > 0 {
                    self.input_chars.lock().unwrap().remove(cursor_pos - 1);
                    self.cursor_pos.set(cursor_pos - 1);
                }
            }
            KeyCode::Char(c) => {
                let mut last = self.last_interrupt.lock().unwrap();
                *last = None;
                let cursor_pos = self.cursor_pos.get();
                self.input_chars.lock().unwrap().insert(cursor_pos, c);
                self.cursor_pos.set(cursor_pos + 1);
            }
            KeyCode::Enter => {
                let submitted: String = self.input_chars.lock().unwrap().iter().collect();
                let submitted = submitted.trim();
                if submitted.len() > 0 {
                    let line = format!("{} {}\n", "❯".dark_grey(), submitted);
                    self.print(line)?;

                    self.input_chars.lock().unwrap().clear();
                    self.cursor_pos.set(0);
                    self.scroll_offset.set(0);
                    let (_, new_y) = cursor::position()?;
                    self.start_row.set(new_y as usize);

                    return Ok(Action::Submit(submitted.into()));
                }
            }
            _ => {
                let mut last = self.last_interrupt.lock().unwrap();
                *last = None;
            }
        }

        Ok(Action::None)
    }

    fn graceful_exit(&self) -> io::Result<()> {
        let mut stdout = self.stdout.lock().unwrap();
        let start_row = self.start_row.get();
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
