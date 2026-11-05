//! Link filters for terminal tokens. Each filter reads one token and
//! proposes a file location; the resolver checks the candidates on the
//! filesystem worker, in order, and takes the first existing file. No
//! filter touches the filesystem, so they are cheap to test and to add.

/// A proposed file location: path text plus optional 1-based line/column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub path: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

impl Candidate {
    fn new(path: &str, line: Option<u32>, column: Option<u32>) -> Option<Self> {
        (!path.is_empty()).then(|| Self {
            path: path.to_owned(),
            line,
            column,
        })
    }
}

type Filter = fn(&str) -> Option<Candidate>;

/// Filters in priority order. The whole token is tried first, so a file
/// whose name really ends in `:12` still opens.
const FILTERS: [Filter; 4] = [whole, colon_location, paren_location, file_url];

fn number(text: &str) -> Option<u32> {
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn whole(token: &str) -> Option<Candidate> {
    Candidate::new(token, None, None)
}

/// `path:line`, `path:line:col`, with an optional trailing `:` (rustc,
/// gcc, grep -n, ripgrep, eslint, pytest). Go to File reads typed
/// `main.rs:42` the same way.
pub(crate) fn colon_location(token: &str) -> Option<Candidate> {
    let token = token.strip_suffix(':').unwrap_or(token);
    let (rest, last) = token.rsplit_once(':')?;
    let last = number(last)?;
    if let Some((path, line)) = rest.rsplit_once(':')
        && let Some(line) = number(line)
    {
        return Candidate::new(path, Some(line), Some(last));
    }
    Candidate::new(rest, Some(last), None)
}

/// `path(line)` and `path(line,col)` (MSVC, TypeScript, C#).
fn paren_location(token: &str) -> Option<Candidate> {
    let token = token.strip_suffix(':').unwrap_or(token);
    let inner = token.strip_suffix(')')?;
    let (path, numbers) = inner.rsplit_once('(')?;
    match numbers.split_once(',') {
        Some((line, column)) => Candidate::new(
            path,
            Some(number(line.trim())?),
            Some(number(column.trim())?),
        ),
        None => Candidate::new(path, Some(number(numbers)?), None),
    }
}

/// `file:///abs/path` with percent-encoded bytes, plus an optional
/// `#L12` or `:12` line.
fn file_url(token: &str) -> Option<Candidate> {
    let rest = token.strip_prefix("file://")?;
    // `file://host/path`: only the local host is a local file.
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let (path, line) = match rest.rsplit_once("#L") {
        Some((path, line)) => (path, number(line)),
        None => (rest, None),
    };
    let decoded = percent_decode(path)?;
    if line.is_some() {
        return Candidate::new(&decoded, line, None);
    }
    colon_location(&decoded).or_else(|| Candidate::new(&decoded, None, None))
}

fn percent_decode(text: &str) -> Option<String> {
    let mut out = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%' {
            let [high, low, after @ ..] = tail else {
                return None;
            };
            let digit = |c: u8| {
                char::from(c)
                    .to_digit(16)
                    .and_then(|d| u8::try_from(d).ok())
            };
            out.push(digit(*high)?.checked_mul(16)?.checked_add(digit(*low)?)?);
            rest = after;
        } else {
            out.push(byte);
            rest = tail;
        }
    }
    String::from_utf8(out).ok()
}

/// Strip quotes and sentence punctuation around a token.
fn trim(token: &str) -> &str {
    const QUOTES: [char; 3] = ['\'', '"', '`'];
    const PUNCTUATION: [char; 3] = [',', ';', '.'];
    token
        .trim()
        .trim_end_matches(PUNCTUATION)
        .trim_matches(QUOTES)
        .trim_end_matches(PUNCTUATION)
}

/// Candidates for `token`, most specific last; the resolver tries them in
/// this order.
pub(crate) fn candidates(token: &str) -> Vec<Candidate> {
    let token = trim(token);
    let mut found: Vec<Candidate> = Vec::new();
    for filter in FILTERS {
        if let Some(candidate) = filter(token)
            && !found.contains(&candidate)
        {
            found.push(candidate);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(path: &str, line: Option<u32>, column: Option<u32>) -> Candidate {
        Candidate {
            path: path.into(),
            line,
            column,
        }
    }

    #[test]
    fn compiler_locations_give_line_and_column() {
        let found = candidates("src/main.rs:12:5:");
        assert_eq!(found[0], at("src/main.rs:12:5:", None, None));
        assert!(found.contains(&at("src/main.rs", Some(12), Some(5))));
        assert!(candidates("a.py:7").contains(&at("a.py", Some(7), None)));
    }

    #[test]
    fn parenthesized_locations_are_understood() {
        assert!(candidates("src/app.ts(10,3):").contains(&at("src/app.ts", Some(10), Some(3))));
        assert!(candidates("Program.cs(42)").contains(&at("Program.cs", Some(42), None)));
        assert!(!candidates("fn(x)").iter().any(|c| c.line.is_some()));
    }

    #[test]
    fn file_urls_are_decoded() {
        assert!(candidates("file:///tmp/my%20file.rs#L9").contains(&at(
            "/tmp/my file.rs",
            Some(9),
            None
        )));
        assert!(candidates("file://localhost/etc/hosts:3").contains(&at(
            "/etc/hosts",
            Some(3),
            None
        )));
        assert!(
            candidates("file:///bad%zz")
                .iter()
                .all(|c| !c.path.starts_with('/'))
        );
    }

    #[test]
    fn surrounding_quotes_and_punctuation_are_ignored() {
        assert!(candidates("'lib.rs:3',").contains(&at("lib.rs", Some(3), None)));
        assert_eq!(candidates("\"notes.md\".")[0], at("notes.md", None, None));
    }

    #[test]
    fn non_numeric_suffixes_are_not_locations() {
        let found = candidates("host:port");
        assert_eq!(found, [at("host:port", None, None)]);
    }
}
