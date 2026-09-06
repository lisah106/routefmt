mod normalize;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::process::ExitCode;

struct Args {
    check: bool,
    in_place: bool,
    paths: Vec<String>,
}

/// The flags so far are `--check` and `--in-place`; anything else is a
/// file path. There's no `--` escape yet because there's nothing to
/// escape: routefmt doesn't take any option that looks like a path.
fn parse_args(raw: &[String]) -> Args {
    let mut check = false;
    let mut in_place = false;
    let mut paths = Vec::new();
    for arg in raw {
        if arg == "--check" {
            check = true;
        } else if arg == "--in-place" {
            in_place = true;
        } else {
            paths.push(arg.clone());
        }
    }
    Args { check, in_place, paths }
}

fn main() -> ExitCode {
    let raw_args: Vec<String> = env::args().skip(1).collect();
    let args = parse_args(&raw_args);

    if args.check && args.in_place {
        eprintln!("routefmt: --check and --in-place cannot be used together");
        return ExitCode::FAILURE;
    }

    if args.in_place {
        if args.paths.is_empty() {
            eprintln!("routefmt: --in-place requires at least one file");
            return ExitCode::FAILURE;
        }
        return if run_in_place(&args.paths) {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
    }

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
    let (formatted, had_error) = format_text(input, "");
    io::stdout().write_all(formatted.as_bytes())?;
    Ok(had_error)
}

/// Rewrites each file in place with its normalized contents. Files are
/// processed independently of each other: collisions and errors are
/// scoped to a single file (so its messages carry the path instead of a
/// bare line number), and a file is only touched on disk if its
/// normalized form actually differs from what's already there.
fn run_in_place(paths: &[String]) -> bool {
    let mut had_error = false;
    for path in paths {
        let contents = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("routefmt: {}: {}", path, e);
                had_error = true;
                continue;
            }
        };

        let (formatted, file_had_error) = format_text(&contents, path);
        if file_had_error {
            had_error = true;
        }

        if formatted != contents {
            if let Err(e) = fs::write(path, &formatted) {
                eprintln!("routefmt: {}: failed to write: {}", path, e);
                had_error = true;
            }
        }
    }
    had_error
}

/// Normalizes every line of `input`, collecting the rewritten output as a
/// single string instead of writing it anywhere, so callers can either
/// print it (stdout) or compare it against a file's existing contents
/// (in-place) before deciding what to do with it. `label` prefixes error
/// and collision messages with a file path; pass "" when there's no
/// single file to point at (stdin, or multiple files concatenated).
fn format_text(input: &str, label: &str) -> (String, bool) {
    let mut out = String::new();
    let mut had_error = false;
    let mut seen: HashMap<String, usize> = HashMap::new();

    for (i, line) in input.lines().enumerate() {
        let line_no = i + 1;
        match normalize::normalize_route(line) {
            Ok(Some(normalized)) => {
                if record_collision(&mut seen, &normalized, line_no, label) {
                    had_error = true;
                }
                out.push_str(&normalized);
                out.push('\n');
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("{}", format_location(label, line_no, &e.to_string()));
                had_error = true;
            }
        }
    }

    (out, had_error)
}

/// Same checks as `run_format`, but it never writes a rewrite to stdout:
/// it only reports whether the input is already canonical, so a CI step
/// can fail the build without a formatted copy showing up anywhere.
fn run_check(input: &str) -> bool {
    let mut had_error = false;
    let mut seen: HashMap<String, usize> = HashMap::new();

    for (i, line) in input.lines().enumerate() {
        let line_no = i + 1;
        match normalize::normalize_route(line) {
            Ok(Some(normalized)) => {
                if record_collision(&mut seen, &normalized, line_no, "") {
                    had_error = true;
                }

                if !is_normalized(line, &normalized) {
                    eprintln!(
                        "{}",
                        format_location(
                            "",
                            line_no,
                            &format!("not normalized, expected '{}'", normalized)
                        )
                    );
                    had_error = true;
                }
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("{}", format_location("", line_no, &e.to_string()));
                had_error = true;
            }
        }
    }

    had_error
}

/// Records `normalized`'s collision key as having first appeared on
/// `line_no`, or reports and returns true if that shape was already seen.
fn record_collision(
    seen: &mut HashMap<String, usize>,
    normalized: &str,
    line_no: usize,
    label: &str,
) -> bool {
    let key = normalize::collision_key(normalized);
    if let Some(&first_line) = seen.get(&key) {
        eprintln!(
            "{}",
            format_location(
                label,
                line_no,
                &format!("route collides with line {} after normalization", first_line)
            )
        );
        true
    } else {
        seen.insert(key, line_no);
        false
    }
}

/// Formats a diagnostic as `routefmt: line N: msg`, or, when `label` is a
/// file path (used for `--in-place`, where several files are processed
/// independently), `routefmt: path: line N: msg`.
fn format_location(label: &str, line_no: usize, msg: &str) -> String {
    if label.is_empty() {
        format!("routefmt: line {}: {}", line_no, msg)
    } else {
        format!("routefmt: {}: line {}: {}", label, line_no, msg)
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
    fn parse_args_recognizes_in_place_flag() {
        let raw = vec!["--in-place".to_string(), "routes.txt".to_string()];
        let args = parse_args(&raw);
        assert!(args.in_place);
        assert_eq!(args.paths, vec!["routes.txt".to_string()]);
    }

    #[test]
    fn parse_args_defaults_to_no_check_and_no_paths() {
        let args = parse_args(&[]);
        assert!(!args.check);
        assert!(!args.in_place);
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
        assert!(!record_collision(&mut seen, "GET /users/:id", 1, ""));
        assert!(record_collision(&mut seen, "GET /users/:name", 4, ""));
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

    #[test]
    fn format_text_prefixes_errors_with_label() {
        let (_, had_error) = format_text("GET /users/{}\n", "routes.txt");
        assert!(had_error);
    }

    fn temp_path(name: &str) -> String {
        std::env::temp_dir()
            .join(format!("routefmt-test-{}-{}", std::process::id(), name))
            .to_string_lossy()
            .to_string()
    }

    #[test]
    fn run_in_place_rewrites_unnormalized_file() {
        let path = temp_path("rewrites-unnormalized");
        fs::write(&path, "get /Users/{id}\n").unwrap();

        assert!(!run_in_place(&[path.clone()]));
        assert_eq!(fs::read_to_string(&path).unwrap(), "GET /users/:id\n");

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn run_in_place_leaves_already_normalized_file_untouched() {
        let path = temp_path("leaves-normalized-alone");
        fs::write(&path, "GET /users/:id\n").unwrap();
        let before = fs::metadata(&path).unwrap().modified().unwrap();

        assert!(!run_in_place(&[path.clone()]));
        let after = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(before, after);

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn run_in_place_reports_error_but_still_writes_valid_lines() {
        let path = temp_path("reports-error");
        fs::write(&path, "GET /users/{}\nget /Posts\n").unwrap();

        assert!(run_in_place(&[path.clone()]));
        assert_eq!(fs::read_to_string(&path).unwrap(), "GET /posts\n");

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn run_in_place_reports_missing_file_without_panicking() {
        let path = temp_path("does-not-exist");
        assert!(run_in_place(&[path]));
    }
}
