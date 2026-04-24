use crossterm::{
    cursor::{self, Hide, MoveTo, Show},
    event::{Event, EventStream, KeyCode, KeyModifiers},
    execute, queue,
    style::{Print, Stylize},
    terminal::{self, Clear, ClearType, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use std::io::{self, Write};
use tokio::time::{Duration, Instant}; // Added for timing
use unicode_width::UnicodeWidthChar;

#[tokio::main]
async fn main() -> io::Result<()> {
    let mut start_row: usize = 0;
    workarea_render(&mut start_row).await
}

async fn workarea_render(start_row: &mut usize) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    let mut reader = EventStream::new();

    let mut input_chars: Vec<char> = Vec::new();
    let mut cursor_pos = 0;
    let mut scroll_offset = 0;

    // --- Timing State ---
    let mut last_interrupt: Option<Instant> = None;
    let interrupt_threshold = Duration::from_millis(1000);

    let workarea_height = 4;
    let (_, y) = cursor::position()?;
    *start_row = y as usize;

    loop {
        let (width, height) = terminal::size()?;
        let width = width as usize;
        let inner_width = width.saturating_sub(4);

        if *start_row + workarea_height > height as usize {
            let scroll_needed = (*start_row + workarea_height) - height as usize;
            queue!(stdout, MoveTo(0, (height - 1) as u16))?;
            for _ in 0..scroll_needed {
                execute!(stdout, Print("\n"))?;
            }
            *start_row -= scroll_needed;
        }

        let mut visual_cursor_col = 0;
        for i in 0..cursor_pos {
            visual_cursor_col += input_chars[i].width().unwrap_or(0);
        }

        if visual_cursor_col < scroll_offset {
            scroll_offset = visual_cursor_col;
        } else if visual_cursor_col >= scroll_offset + inner_width {
            scroll_offset = visual_cursor_col - inner_width + 1;
        }

        let mut visible_text = String::new();
        let mut current_visual_col = 0;
        for ch in &input_chars {
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
            // blank line
            MoveTo(0, *start_row as u16),
            Clear(ClearType::CurrentLine),
            // separator
            MoveTo(0, (*start_row + 1) as u16),
            Print("─".repeat(width).dark_grey()),
            // input
            MoveTo(0, (*start_row + 2) as u16),
            Clear(ClearType::CurrentLine),
            Print(format!("{} ", "❯".grey())),
            Print(&visible_text),
            // separator
            MoveTo(0, (*start_row + 3) as u16),
            Print("─".repeat(width).dark_grey()),
            // cursor
            MoveTo(
                (visual_cursor_col - scroll_offset + 2) as u16,
                (*start_row + 2) as u16
            ),
            Show,
        )?;
        stdout.flush()?;

        if let Some(Ok(event)) = reader.next().await {
            if let Event::Key(key_event) = event {
                match key_event.code {
                    KeyCode::Esc => break,
                    KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                        let now = Instant::now();

                        if let Some(last) = last_interrupt {
                            if now.duration_since(last) < interrupt_threshold {
                                // Double Ctrl+C detected within timeout
                                break;
                            }
                        }

                        // First Ctrl+C (or timing expired)
                        last_interrupt = Some(now);

                        let line = "interrupt (press again to exit)".yellow().italic();
                        workarea_print(&mut stdout, line, start_row)?;

                        let (_, new_y) = cursor::position()?;
                        *start_row = new_y as usize;
                    }
                    KeyCode::Left => {
                        last_interrupt = None;
                        cursor_pos = cursor_pos.saturating_sub(1);
                    }
                    KeyCode::Right => {
                        last_interrupt = None;
                        if cursor_pos < input_chars.len() {
                            cursor_pos += 1;
                        }
                    }
                    KeyCode::Backspace => {
                        last_interrupt = None;
                        if cursor_pos > 0 {
                            input_chars.remove(cursor_pos - 1);
                            cursor_pos -= 1;
                        }
                    }
                    KeyCode::Char(c) => {
                        last_interrupt = None;
                        input_chars.insert(cursor_pos, c);
                        cursor_pos += 1;
                    }
                    KeyCode::Enter => {
                        // user input
                        last_interrupt = None;
                        let submitted: String = input_chars.iter().collect();
                        let line = format!("{} {}\n", "❯".dark_grey(), submitted);
                        workarea_print(&mut stdout, line, start_row)?;

                        input_chars.clear();
                        cursor_pos = 0;
                        scroll_offset = 0;
                        let (_, new_y) = cursor::position()?;
                        *start_row = new_y as usize;
                    }
                    _ => {
                        last_interrupt = None;
                    }
                }
            }
        }
    }

    // --- 5. Graceful Exit Cleanup ---
    // Move cursor to the last line of the workarea and print a final newline
    // so the prompt doesn't overwrite your UI in the scrollback.
    execute!(
        stdout,
        MoveTo(0, (*start_row + workarea_height - 1) as u16),
        Print("\n"),
        Show
    )?;

    disable_raw_mode()?;

    Ok(())
}

fn workarea_print<T: std::fmt::Display>(
    stdout: &mut io::Stdout,
    line: T,
    start_row: &mut usize,
) -> io::Result<()> {
    let content = format!("{}", line);
    let lines: Vec<&str> = content.lines().collect();

    // Move to the first blank line of the workarea
    queue!(stdout, MoveTo(0, *start_row as u16))?;

    for (i, s) in lines.iter().enumerate() {
        let formatted = if i == 0 {
            format!("{}", s)
        } else {
            format!("  {}", s)
        };
        // Clear current line to prevent ghost characters, then print
        queue!(
            stdout,
            Clear(ClearType::CurrentLine),
            Print(formatted),
            Print("\r\n"),
        )?;
    }

    queue!(stdout, Clear(ClearType::CurrentLine), Print("\r\n"))?;

    stdout.flush()?;

    Ok(())
}
