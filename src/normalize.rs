use std::fmt;

const KNOWN_METHODS: [&str; 7] =
    ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

#[derive(Debug)]
pub enum NormalizeError {
    EmptyParamName(String),
    InvalidParamName(String),
}

impl fmt::Display for NormalizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NormalizeError::EmptyParamName(seg) => {
                write!(f, "empty parameter name in segment '{}'", seg)
            }
            NormalizeError::InvalidParamName(seg) => {
                write!(
                    f,
                    "parameter name in segment '{}' must be alphanumeric or underscore",
                    seg
                )
            }
        }
    }
}

/// Normalizes one line of route table input.
///
/// Returns `Ok(None)` for blank lines and comments (lines starting with `#`),
/// so callers can drop them from the output without treating that as an error.
pub fn normalize_route(line: &str) -> Result<Option<String>, NormalizeError> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Ok(None);
    }

    let (method, path) = split_method(trimmed);
    let normalized_path = normalize_path(path)?;
    Ok(Some(format!("{} {}", method, normalized_path)))
}

/// Pulls a leading HTTP method token off the line, if there is one.
/// A route with no method (just a path) defaults to GET, since that's
/// what most route tables mean when they omit it.
fn split_method(line: &str) -> (String, &str) {
    if let Some(idx) = line.find(char::is_whitespace) {
        let (first, rest) = line.split_at(idx);
        let candidate = first.to_uppercase();
        if KNOWN_METHODS.contains(&candidate.as_str()) {
            return (candidate, rest.trim_start());
        }
    }
    ("GET".to_string(), line)
}

fn normalize_path(path: &str) -> Result<String, NormalizeError> {
    // Splitting on '/' and dropping empty pieces is what collapses
    // repeated slashes and strips leading/trailing ones in one pass.
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    let mut normalized = Vec::with_capacity(segments.len());
    for seg in segments {
        normalized.push(normalize_segment(seg)?);
    }

    if normalized.is_empty() {
        Ok("/".to_string())
    } else {
        Ok(format!("/{}", normalized.join("/")))
    }
}

fn normalize_segment(segment: &str) -> Result<String, NormalizeError> {
    if segment == "*" {
        return Ok("*".to_string());
    }

    if let Some(name) = param_name(segment) {
        if name.is_empty() {
            return Err(NormalizeError::EmptyParamName(segment.to_string()));
        }
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(NormalizeError::InvalidParamName(segment.to_string()));
        }
        return Ok(format!(":{}", name));
    }

    // Static segments are case-insensitive on every router I've had to
    // deal with, so folding them to lowercase makes diffs and dedup work.
    Ok(segment.to_lowercase())
}

/// Recognizes the three param spellings people actually use in the wild:
/// `:id`, `{id}`, and `<id>`. Returns the bare name in all three cases.
fn param_name(segment: &str) -> Option<&str> {
    if let Some(rest) = segment.strip_prefix(':') {
        return Some(rest);
    }
    if segment.len() >= 2 && segment.starts_with('{') && segment.ends_with('}') {
        return Some(&segment[1..segment.len() - 1]);
    }
    if segment.len() >= 2 && segment.starts_with('<') && segment.ends_with('>') {
        return Some(&segment[1..segment.len() - 1]);
    }
    None
}
