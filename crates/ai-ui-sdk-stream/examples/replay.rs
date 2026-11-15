//! JSONL replay tool for deterministic oracles and separately scoped CPU timing.

mod support;

use std::io::{self, BufRead, Read, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = io::stdin().lock();
    let mut output = io::BufWriter::new(io::stdout().lock());
    loop {
        let mut line = String::new();
        let read = input
            .by_ref()
            .take(128 * 1024 * 1024)
            .read_line(&mut line)?;
        if read == 0 {
            break;
        }
        if read >= 128 * 1024 * 1024 {
            return Err("replay request exceeds 128 MiB".into());
        }
        if line.trim().is_empty() {
            continue;
        }
        let request = serde_json::from_str(&line)?;
        let replay = support::replay(&request)?;
        serde_json::to_writer(&mut output, &replay)?;
        writeln!(&mut output)?;
        output.flush()?;
    }
    Ok(())
}
