use crossterm::{
    cursor::{self, Hide, MoveTo, Show},
    event::{self, Event, KeyCode, KeyModifiers},
    execute, queue,
    style::{Print, Stylize},
    terminal::{self, Clear, ClearType},
};
use std::io::{self, Write};
use unicode_width::UnicodeWidthChar;

fn main() -> io::Result<()> {
    let mut start_row: usize = 0;
    render_workarea(&mut start_row)
}

fn render_workarea(start_row: &mut usize) -> io::Result<()> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();

    let mut input_chars: Vec<char> = Vec::new();
    let mut cursor_pos = 0;
    let mut scroll_offset = 0;
    let workarea_height = 4; // Height of the UI (blank + separator + input + separator)

    // Initial anchor point for the UI
    let (_, y) = cursor::position()?;
    *start_row = y as usize;

    loop {
        let (width, height) = terminal::size()?;
        let width = width as usize;
        let height = height as usize;
        let inner_width = width.saturating_sub(4);

        // --- 1. Terminal Boundary Management ---
        // If the UI would go off the bottom of the screen, scroll the terminal
        if *start_row + workarea_height > height {
            let scroll_needed = (*start_row + workarea_height) - height;
            queue!(stdout, MoveTo(0, (height - 1) as u16))?;
            for _ in 0..scroll_needed {
                execute!(stdout, Print("\n"))?;
            }
            *start_row -= scroll_needed;
        }

        // --- 2. CJK & Scrolling Logic ---
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

        // --- 3. Render UI ---
        queue!(
            stdout,
            Hide,
            // Clear current UI area
            MoveTo(0, *start_row as u16),
            Clear(ClearType::CurrentLine),
            // Top Border
            MoveTo(0, (*start_row + 1) as u16),
            Print("─".repeat(width).dark_grey()),
            // Prompt and Input
            MoveTo(0, (*start_row + 2) as u16),
            Clear(ClearType::CurrentLine),
            Print(format!("{} ", "❯".grey())),
            Print(&visible_text),
            // Bottom Border
            MoveTo(0, (*start_row + 3) as u16),
            Print("─".repeat(width).dark_grey()),
            // Position Cursor
            MoveTo(
                (visual_cursor_col - scroll_offset + 2) as u16,
                (*start_row + 2) as u16
            ),
            Show
        )?;
        stdout.flush()?;

        // --- 4. Input Handling ---
        if let Event::Key(key_event) = event::read()? {
            match key_event.code {
                KeyCode::Esc => break,
                KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => break,
                KeyCode::Left => cursor_pos = cursor_pos.saturating_sub(1),
                KeyCode::Right => {
                    if cursor_pos < input_chars.len() {
                        cursor_pos += 1;
                    }
                }
                KeyCode::Backspace => {
                    if cursor_pos > 0 {
                        input_chars.remove(cursor_pos - 1);
                        cursor_pos -= 1;
                    }
                }
                KeyCode::Char(c) => {
                    input_chars.insert(cursor_pos, c);
                    cursor_pos += 1;
                }
                KeyCode::Enter => {
                    let submitted: String = input_chars.iter().collect();

                    // Clear the UI box and print the finalized text
                    queue!(
                        stdout,
                        Hide,
                        MoveTo(0, (*start_row) as u16),
                        // The '\n' will move cursor to the top of workarea
                        Print(format!("{} {}\n", "You:".cyan(), submitted)),
                    )?;
                    stdout.flush()?;

                    // Reset state
                    input_chars.clear();
                    cursor_pos = 0;
                    scroll_offset = 0;

                    // Re-anchor start_row to exactly where the cursor is now
                    let (_, new_y) = cursor::position()?;
                    *start_row = new_y as usize;
                }
                _ => {}
            }
        }
    }

    terminal::disable_raw_mode()?;
    print!("\n\n");
    Ok(())
}
