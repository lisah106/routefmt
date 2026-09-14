use std::fmt;

const KNOWN_METHODS: [&str; 7] =
    ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

#[derive(Debug)]
pub enum NormalizeError {
    EmptyParamName(String),
    InvalidParamName(String),
    UnknownMethod(String),
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
            NormalizeError::UnknownMethod(method) => {
                write!(f, "unknown HTTP method '{}'", method)
            }
        }
    }
}

/// Normalizes one line of route table input.
///
/// Returns `Ok(None)` for blank lines and comments (lines starting with `#`),
/// so callers can drop them from the output without treating that as an error.
///
/// Besides the plain `METHOD /path` form, this also picks a route out of a
/// line lifted straight from framework source, like `app.get('/users/:id')`
/// or `router.post("/users", create)` or Rails-style `delete '/users/<id>'`:
/// anything with a quoted string that looks like a path.
pub fn normalize_route(line: &str) -> Result<Option<String>, NormalizeError> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Ok(None);
    }

    let (method, path) = match extract_embedded_route(trimmed) {
        Some((m, p)) => (m, p),
        None => split_method(trimmed),
    };
    let normalized_path = normalize_path(path)?;
    Ok(Some(format!("{} {}", method, normalized_path)))
}

/// Which half of a `METHOD /path` pair a table line supplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableField {
    Method,
    Path,
}

/// Recognizes one `method` or `path` key from a route table where the two
/// live on separate lines, as in a YAML list entry or a pretty-printed JSON
/// object with the braces on their own lines:
///   - method: GET
///     path: /users/:id
///   {
///     "method": "GET",
///     "path": "/users/:id"
///   }
/// Any other key (name, handler, description, ...) is ignored, so unrelated
/// fields in the same entry don't get mistaken for a route.
///
/// This is a line scan, not a parser, so it only handles one key per line,
/// and a `{`/`}` isn't stripped from a value the way it would be by a real
/// JSON parser (deliberately - a path can legitimately end in a `{id}`
/// placeholder, and there'd be no reliable way to tell that apart from an
/// object's closing brace tacked onto the same line). A single-line object
/// with both keys crammed together (`{"method": "GET", "path": "/users"}`)
/// is rejected outright for the same reason: telling the value's closing
/// quote from the next key's opening one needs real JSON parsing.
pub fn extract_table_field(line: &str) -> Option<(TableField, String)> {
    let trimmed = line.trim().trim_start_matches('-').trim();

    let (key, rest) = trimmed.split_once(':')?;
    let field = match unquote(key.trim()).to_lowercase().as_str() {
        "method" => TableField::Method,
        "path" => TableField::Path,
        _ => return None,
    };

    let rest = rest.trim();
    if let Some(comma_idx) = rest.find(',') {
        let after = rest[comma_idx + 1..].trim();
        let after = after.strip_prefix('"').or_else(|| after.strip_prefix('\'')).unwrap_or(after);
        let next_key = after
            .split(|c| c == ':' || c == '"' || c == '\'')
            .next()
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if next_key == "method" || next_key == "path" {
            return None;
        }
    }

    let value = unquote(rest.trim_end_matches(',').trim());
    if value.is_empty() {
        None
    } else {
        Some((field, value.to_string()))
    }
}

/// Strips one layer of matching double or single quotes, if present.
fn unquote(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// Combines an explicit method and path pulled from separate table keys
/// into the same canonical form `normalize_route` produces. Unlike a bare
/// path line, the method here isn't optional or inferred, so an unknown
/// one is an error instead of silently defaulting to GET.
pub fn normalize_table_route(method: &str, path: &str) -> Result<String, NormalizeError> {
    let upper = method.to_uppercase();
    if !KNOWN_METHODS.contains(&upper.as_str()) {
        return Err(NormalizeError::UnknownMethod(method.to_string()));
    }
    let normalized_path = normalize_path(path)?;
    Ok(format!("{} {}", upper, normalized_path))
}

/// Looks for a quoted path inside a line of code, since that's how routes
/// show up in most frameworks' own config: `app.get('/users/:id', handler)`,
/// `router.post("/users")`, `get '/health'`. A quote is only treated as the
/// start of a path if it's immediately followed by `/`, which is what tells
/// it apart from, say, the key of a JSON object or a handler function name.
fn extract_embedded_route(line: &str) -> Option<(String, &str)> {
    for (i, c) in line.char_indices() {
        if c != '\'' && c != '"' {
            continue;
        }
        let start = i + c.len_utf8();
        if !line[start..].starts_with('/') {
            continue;
        }
        if let Some(end) = line[start..].find(c) {
            let path = &line[start..start + end];
            return Some((method_before(&line[..i]), path));
        }
    }
    None
}

/// Reads the word immediately before a quoted path, e.g. the `get` in
/// `app.get(` or `router.post(`. Anything that isn't a known method
/// (including no word at all, as in a bare JSON `"/users/:id"`) defaults
/// to GET, same as a plain path line with no method column.
fn method_before(prefix: &str) -> String {
    let word: String = prefix
        .trim_end()
        .trim_end_matches('(')
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    let word: String = word.chars().rev().collect();

    let upper = word.to_uppercase();
    if KNOWN_METHODS.contains(&upper.as_str()) {
        upper
    } else {
        "GET".to_string()
    }
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

/// Reduces an already-normalized "METHOD /path" line to a key that two
/// routes share exactly when they'd collide at runtime: param names don't
/// matter to a router, only where the params sit in the path, so `:id` and
/// `:name` at the same position are folded to the same placeholder.
pub fn collision_key(normalized_route: &str) -> String {
    let mut parts = normalized_route.splitn(2, ' ');
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    let generic_path: Vec<&str> = path
        .split('/')
        .map(|seg| if seg.starts_with(':') { ":" } else { seg })
        .collect();

    format!("{} {}", method, generic_path.join("/"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_passes_through_wildcard() {
        assert_eq!(normalize_segment("*").unwrap(), "*");
    }

    #[test]
    fn segment_lowercases_static_text() {
        assert_eq!(normalize_segment("Users").unwrap(), "users");
        assert_eq!(normalize_segment("POSTS").unwrap(), "posts");
    }

    #[test]
    fn segment_normalizes_colon_param() {
        assert_eq!(normalize_segment(":id").unwrap(), ":id");
    }

    #[test]
    fn segment_normalizes_brace_param() {
        assert_eq!(normalize_segment("{id}").unwrap(), ":id");
    }

    #[test]
    fn segment_normalizes_angle_param() {
        assert_eq!(normalize_segment("<id>").unwrap(), ":id");
    }

    #[test]
    fn segment_rejects_empty_param_name() {
        assert!(matches!(
            normalize_segment(":"),
            Err(NormalizeError::EmptyParamName(_))
        ));
        assert!(matches!(
            normalize_segment("{}"),
            Err(NormalizeError::EmptyParamName(_))
        ));
    }

    #[test]
    fn segment_rejects_punctuation_in_param_name() {
        assert!(matches!(
            normalize_segment(":user-id"),
            Err(NormalizeError::InvalidParamName(_))
        ));
        assert!(matches!(
            normalize_segment("{user.id}"),
            Err(NormalizeError::InvalidParamName(_))
        ));
    }

    #[test]
    fn segment_allows_underscore_in_param_name() {
        assert_eq!(normalize_segment(":user_id").unwrap(), ":user_id");
    }

    #[test]
    fn path_collapses_repeated_slashes() {
        assert_eq!(normalize_path("/users//1//posts").unwrap(), "/users/1/posts");
    }

    #[test]
    fn path_strips_leading_and_trailing_slashes() {
        assert_eq!(normalize_path("users/1/").unwrap(), "/users/1");
        assert_eq!(normalize_path("/users/1/").unwrap(), "/users/1");
    }

    #[test]
    fn path_of_just_slashes_is_root() {
        assert_eq!(normalize_path("/").unwrap(), "/");
        assert_eq!(normalize_path("").unwrap(), "/");
        assert_eq!(normalize_path("///").unwrap(), "/");
    }

    #[test]
    fn path_propagates_segment_errors() {
        assert!(normalize_path("/users/{}/posts").is_err());
    }

    #[test]
    fn route_defaults_missing_method_to_get() {
        assert_eq!(
            normalize_route("/health").unwrap(),
            Some("GET /health".to_string())
        );
    }

    #[test]
    fn route_skips_blank_and_comment_lines() {
        assert_eq!(normalize_route("").unwrap(), None);
        assert_eq!(normalize_route("   ").unwrap(), None);
        assert_eq!(normalize_route("# a comment").unwrap(), None);
    }

    #[test]
    fn route_uppercases_known_method() {
        assert_eq!(
            normalize_route("get /Users/{id}/Posts//").unwrap(),
            Some("GET /users/:id/posts".to_string())
        );
    }

    #[test]
    fn route_treats_unknown_leading_token_as_path() {
        // "foo" isn't a known method, so with no whitespace to split on
        // the whole thing is the path, not a method that got dropped.
        assert_eq!(
            normalize_route("foo/bar").unwrap(),
            Some("GET /foo/bar".to_string())
        );
    }

    #[test]
    fn collision_key_folds_different_param_names_together() {
        assert_eq!(
            collision_key("GET /users/:id"),
            collision_key("GET /users/:name")
        );
    }

    #[test]
    fn collision_key_distinguishes_static_segments() {
        assert_ne!(
            collision_key("GET /users/:id"),
            collision_key("GET /users/active")
        );
    }

    #[test]
    fn collision_key_distinguishes_methods() {
        assert_ne!(
            collision_key("GET /users/:id"),
            collision_key("POST /users/:id")
        );
    }

    #[test]
    fn collision_key_distinguishes_path_length() {
        assert_ne!(
            collision_key("GET /users/:id"),
            collision_key("GET /users/:id/posts")
        );
    }

    #[test]
    fn method_before_reads_word_before_call_paren() {
        assert_eq!(method_before("app.get("), "GET");
        assert_eq!(method_before("router.post("), "POST");
    }

    #[test]
    fn method_before_reads_bare_word_with_no_call() {
        assert_eq!(method_before("delete "), "DELETE");
    }

    #[test]
    fn method_before_defaults_to_get_when_word_is_unknown() {
        assert_eq!(method_before("handler("), "GET");
        assert_eq!(method_before(""), "GET");
    }

    #[test]
    fn extract_embedded_route_reads_express_style_call() {
        assert_eq!(
            extract_embedded_route("app.get('/users/:id', handler)"),
            Some(("GET".to_string(), "/users/:id"))
        );
    }

    #[test]
    fn extract_embedded_route_reads_double_quoted_call() {
        assert_eq!(
            extract_embedded_route("router.post(\"/users\", create)"),
            Some(("POST".to_string(), "/users"))
        );
    }

    #[test]
    fn extract_embedded_route_reads_rails_style_call() {
        assert_eq!(
            extract_embedded_route("delete '/users/<id>'"),
            Some(("DELETE".to_string(), "/users/<id>"))
        );
    }

    #[test]
    fn extract_embedded_route_defaults_to_get_for_bare_quoted_path() {
        assert_eq!(
            extract_embedded_route("\"/users/:id\","),
            Some(("GET".to_string(), "/users/:id"))
        );
    }

    #[test]
    fn extract_embedded_route_skips_quotes_not_followed_by_slash() {
        assert_eq!(extract_embedded_route("\"path\": \"/users/:id\""), Some(("GET".to_string(), "/users/:id")));
        assert_eq!(extract_embedded_route("name: \"users_index\""), None);
    }

    #[test]
    fn route_normalizes_express_style_call() {
        assert_eq!(
            normalize_route("app.get('/Users/{id}', handler)").unwrap(),
            Some("GET /users/:id".to_string())
        );
    }

    #[test]
    fn route_normalizes_rails_style_call() {
        assert_eq!(
            normalize_route("delete '/users/<id>'").unwrap(),
            Some("DELETE /users/:id".to_string())
        );
    }

    #[test]
    fn route_still_normalizes_plain_lines_with_no_quotes() {
        assert_eq!(
            normalize_route("get /Users/{id}/Posts//").unwrap(),
            Some("GET /users/:id/posts".to_string())
        );
    }

    #[test]
    fn extract_table_field_reads_yaml_list_entry() {
        assert_eq!(
            extract_table_field("- method: GET"),
            Some((TableField::Method, "GET".to_string()))
        );
        assert_eq!(
            extract_table_field("  path: /users/:id"),
            Some((TableField::Path, "/users/:id".to_string()))
        );
    }

    #[test]
    fn extract_table_field_reads_quoted_json_key() {
        assert_eq!(
            extract_table_field("\"method\": \"GET\","),
            Some((TableField::Method, "GET".to_string()))
        );
        assert_eq!(
            extract_table_field("  \"path\": \"/users/:id\""),
            Some((TableField::Path, "/users/:id".to_string()))
        );
    }

    #[test]
    fn extract_table_field_strips_a_lone_brace_line() {
        assert_eq!(extract_table_field("{"), None);
        assert_eq!(extract_table_field("},"), None);
    }

    #[test]
    fn extract_table_field_ignores_unrelated_keys() {
        assert_eq!(extract_table_field("name: list users"), None);
        assert_eq!(extract_table_field("handler: listUsers"), None);
    }

    #[test]
    fn extract_table_field_rejects_two_keys_crammed_on_one_line() {
        assert_eq!(
            extract_table_field("\"method\": \"GET\", \"path\": \"/users\""),
            None
        );
    }

    #[test]
    fn extract_table_field_keeps_brace_param_in_path_value() {
        assert_eq!(
            extract_table_field("  path: /users/{id}"),
            Some((TableField::Path, "/users/{id}".to_string()))
        );
    }

    #[test]
    fn extract_table_field_rejects_empty_value() {
        assert_eq!(extract_table_field("method:"), None);
    }

    #[test]
    fn extract_table_field_ignores_lines_with_no_colon() {
        assert_eq!(extract_table_field("GET /users/:id"), None);
    }

    #[test]
    fn normalize_table_route_combines_method_and_path() {
        assert_eq!(
            normalize_table_route("get", "/Users/{id}").unwrap(),
            "GET /users/:id".to_string()
        );
    }

    #[test]
    fn normalize_table_route_rejects_unknown_method() {
        assert!(matches!(
            normalize_table_route("FETCH", "/users"),
            Err(NormalizeError::UnknownMethod(_))
        ));
    }

    #[test]
    fn normalize_table_route_propagates_path_errors() {
        assert!(normalize_table_route("GET", "/users/{}").is_err());
    }
}
