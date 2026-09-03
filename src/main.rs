mod normalize;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::process::ExitCode;

struct Args {
    check: bool,
    paths: Vec<String>,
}

/// The only flag so far is `--check`; anything else is a file path.
/// There's no `--` escape yet because there's nothing to escape: routefmt
/// doesn't take any option that looks like a path.
fn parse_args(raw: &[String]) -> Args {
    let mut check = false;
    let mut paths = Vec::new();
    for arg in raw {
        if arg == "--check" {
            check = true;
        } else {
            paths.push(arg.clone());
        }
    }
    Args { check, paths }
}

fn main() -> ExitCode {
    let raw_args: Vec<String> = env::args().skip(1).collect();
    let args = parse_args(&raw_args);

    let input = if args.paths.is_empty() {
        match read_stdin() {
            Ok(text) => text,
            Err(e) => {
                eprintln!("routefmt: failed to read stdin: {}", e);
                return ExitCode::FAILURE;
            }
        }
    } else {
        match read_files(&args.paths) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("routefmt: {}", e);
                return ExitCode::FAILURE;
            }
        }
    };

    let had_error = if args.check {
        run_check(&input)
    } else {
        match run_format(&input) {
            Ok(had_error) => had_error,
            Err(e) => {
                eprintln!("routefmt: failed to write output: {}", e);
                return ExitCode::FAILURE;
            }
        }
    };

    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn run_format(input: &str) -> io::Result<bool> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut had_error = false;
    let mut seen: HashMap<String, usize> = HashMap::new();

    for (i, line) in input.lines().enumerate() {
        match normalize::normalize_route(line) {
            Ok(Some(normalized)) => {
                if record_collision(&mut seen, &normalized, i + 1) {
                    had_error = true;
                }
                writeln!(out, "{}", normalized)?;
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("routefmt: line {}: {}", i + 1, e);
                had_error = true;
            }
        }
    }

    Ok(had_error)
}

/// Same checks as `run_format`, but it never writes a rewrite to stdout:
/// it only reports whether the input is already canonical, so a CI step
/// can fail the build without a formatted copy showing up anywhere.
fn run_check(input: &str) -> bool {
    let mut had_error = false;
    let mut seen: HashMap<String, usize> = HashMap::new();

    for (i, line) in input.lines().enumerate() {
        match normalize::normalize_route(line) {
            Ok(Some(normalized)) => {
                if record_collision(&mut seen, &normalized, i + 1) {
                    had_error = true;
                }

                if !is_normalized(line, &normalized) {
                    eprintln!(
                        "routefmt: line {}: not normalized, expected '{}'",
                        i + 1,
                        normalized
                    );
                    had_error = true;
                }
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("routefmt: line {}: {}", i + 1, e);
                had_error = true;
            }
        }
    }

    had_error
}

/// Records `normalized`'s collision key as having first appeared on
/// `line_no`, or reports and returns true if that shape was already seen.
fn record_collision(seen: &mut HashMap<String, usize>, normalized: &str, line_no: usize) -> bool {
    let key = normalize::collision_key(normalized);
    if let Some(&first_line) = seen.get(&key) {
        eprintln!(
            "routefmt: line {}: route collides with line {} after normalization",
            line_no, first_line
        );
        true
    } else {
        seen.insert(key, line_no);
        false
    }
}

/// A line counts as already normalized when the only difference between
/// it and what normalize_route produced is surrounding whitespace.
fn is_normalized(original_line: &str, normalized: &str) -> bool {
    original_line.trim() == normalized
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_separates_check_flag_from_paths() {
        let raw = vec!["--check".to_string(), "routes.txt".to_string()];
        let args = parse_args(&raw);
        assert!(args.check);
        assert_eq!(args.paths, vec!["routes.txt".to_string()]);
    }

    #[test]
    fn parse_args_check_flag_can_come_after_paths() {
        let raw = vec!["routes.txt".to_string(), "--check".to_string()];
        let args = parse_args(&raw);
        assert!(args.check);
        assert_eq!(args.paths, vec!["routes.txt".to_string()]);
    }

    #[test]
    fn parse_args_defaults_to_no_check_and_no_paths() {
        let args = parse_args(&[]);
        assert!(!args.check);
        assert!(args.paths.is_empty());
    }

    #[test]
    fn is_normalized_accepts_matching_line_modulo_whitespace() {
        assert!(is_normalized("  GET /users/:id  ", "GET /users/:id"));
    }

    #[test]
    fn is_normalized_rejects_lowercase_method() {
        assert!(!is_normalized("get /users/:id", "GET /users/:id"));
    }

    #[test]
    fn is_normalized_rejects_unnormalized_path() {
        assert!(!is_normalized("GET /users//1/", "GET /users/1"));
    }

    #[test]
    fn record_collision_flags_repeated_shape_and_keeps_first_line() {
        let mut seen = HashMap::new();
        assert!(!record_collision(&mut seen, "GET /users/:id", 1));
        assert!(record_collision(&mut seen, "GET /users/:name", 4));
        assert_eq!(seen.get("GET /users/:"), Some(&1));
    }

    #[test]
    fn run_check_passes_already_normalized_input() {
        assert!(!run_check("GET /users/:id\n"));
    }

    #[test]
    fn run_check_fails_on_unnormalized_input() {
        assert!(run_check("get /Users/{id}\n"));
    }

    #[test]
    fn run_check_fails_on_collision() {
        assert!(run_check("GET /users/:id\nGET /users/:name\n"));
    }
}
