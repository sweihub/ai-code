// src/main.rs
mod workarea;

use std::io;
use tokio::task::LocalSet;
use workarea::{WorkArea, WorkAreaEvent};

fn main() -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local_set = LocalSet::new();
    local_set.block_on(&runtime, async {
        let area = WorkArea::new()?;

        loop {
            match area.tick().await? {
                WorkAreaEvent::Submit(line) => {
                    // submit to agent
                    // agent.query(line)
                    println!("You entered: {line}");
                }
                WorkAreaEvent::Interrupt => {
                    // agent.interrupt();
                }
                WorkAreaEvent::Exit => break,
            }
        }

        Ok(())
    })
}
