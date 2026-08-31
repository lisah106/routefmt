mod normalize;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();

    let input = if args.is_empty() {
        match read_stdin() {
            Ok(text) => text,
            Err(e) => {
                eprintln!("routefmt: failed to read stdin: {}", e);
                return ExitCode::FAILURE;
            }
        }
    } else {
        match read_files(&args) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("routefmt: {}", e);
                return ExitCode::FAILURE;
            }
        }
    };

    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut had_error = false;
    let mut seen: HashMap<String, usize> = HashMap::new();

    for (i, line) in input.lines().enumerate() {
        match normalize::normalize_route(line) {
            Ok(Some(normalized)) => {
                let key = normalize::collision_key(&normalized);
                if let Some(&first_line) = seen.get(&key) {
                    eprintln!(
                        "routefmt: line {}: route collides with line {} after normalization",
                        i + 1,
                        first_line
                    );
                    had_error = true;
                } else {
                    seen.insert(key, i + 1);
                }

                if let Err(e) = writeln!(out, "{}", normalized) {
                    eprintln!("routefmt: failed to write output: {}", e);
                    return ExitCode::FAILURE;
                }
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("routefmt: line {}: {}", i + 1, e);
                had_error = true;
            }
        }
    }

    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn read_stdin() -> io::Result<String> {
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}

/// Reads every file in order and concatenates them, as if the user had
/// `cat`-ed them together, so line numbers in error messages stay simple.
fn read_files(paths: &[String]) -> io::Result<String> {
    let mut combined = String::new();
    for path in paths {
        let contents = fs::read_to_string(path)
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {}", path, e)))?;
        combined.push_str(&contents);
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
    }
    Ok(combined)
}
