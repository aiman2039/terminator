use super::*;

fn plain(snippet: &str) -> Snippet {
    parse_snippet(snippet, &SnippetVars::default())
}

/// The ranges of stop `index`, in text order.
fn ranges(snippet: &Snippet, index: u32) -> Vec<(usize, usize)> {
    let mut found: Vec<(usize, usize)> = snippet
        .stops
        .iter()
        .filter(|stop| stop.index == index)
        .map(|stop| (stop.start, stop.end))
        .collect();
    found.sort_unstable();
    found
}

#[test]
fn placeholders_nest_and_every_stop_keeps_its_range() {
    let snippet = plain("foo(${1:a, ${2:b}})$0");
    assert_eq!(snippet.text, "foo(a, b)");
    assert_eq!(ranges(&snippet, 1), [(4, 8)]);
    assert_eq!(ranges(&snippet, 2), [(7, 8)]);
    assert_eq!(ranges(&snippet, 0), [(9, 9)]);
    // Bare and braced stops are empty ranges.
    let snippet = plain("if $1 {\n    ${2}\n}");
    assert_eq!(snippet.text, "if  {\n    \n}");
    assert_eq!(ranges(&snippet, 1), [(3, 3)]);
    assert_eq!(ranges(&snippet, 2), [(10, 10)]);
    assert!(ranges(&snippet, 0).is_empty());
}

#[test]
fn choices_insert_their_first_option_as_a_stop() {
    let snippet = plain("let ${1|one,two,three|} = ${2|a\\,b,c|};");
    assert_eq!(snippet.text, "let one = a,b;");
    assert_eq!(ranges(&snippet, 1), [(4, 7)]);
    assert_eq!(ranges(&snippet, 2), [(10, 13)]);
    // A choice without its closing `|}` is text.
    assert_eq!(plain("${1|a,b}").text, "${1|a,b}");
}

#[test]
fn escapes_keep_dollar_brace_and_backslash_literal() {
    let snippet = plain("\\$1 \\} \\\\ ${1:a\\}b} \\n");
    assert_eq!(snippet.text, "$1 } \\ a}b \\n");
    assert_eq!(ranges(&snippet, 1), [(7, 10)]);
    // A `$` that starts nothing, and a stray `}`, stay text.
    let snippet = plain("cost: $ 5 } ${");
    assert_eq!(snippet.text, "cost: $ 5 } ${");
    assert!(snippet.stops.is_empty());
}

#[test]
fn variables_resolve_known_names_and_fall_back_to_defaults() {
    let vars = SnippetVars {
        path: Some(PathBuf::from("/src/app/main.rs")),
        line: 4,
        line_text: "    let x".into(),
        word: "x".into(),
        selected: String::new(),
    };
    let snippet = parse_snippet(
        "$TM_FILENAME ${TM_FILENAME_BASE} ${TM_DIRECTORY} $TM_LINE_NUMBER \
         ${TM_SELECTED_TEXT:none} ${UNKNOWN:dflt} [$UNKNOWN] ${TM_CURRENT_WORD}",
        &vars,
    );
    assert_eq!(snippet.text, "main.rs main /src/app 5 none dflt [] x");
    // A default can hold a stop; a value replaces it.
    let snippet = parse_snippet("${TM_SELECTED_TEXT:${1:arg}}", &vars);
    assert_eq!(
        (snippet.text.as_str(), ranges(&snippet, 1)),
        ("arg", vec![(0, 3)])
    );
    let snippet = parse_snippet("${TM_CURRENT_WORD:${1:arg}}", &vars);
    assert_eq!(snippet.text, "x");
    assert!(snippet.stops.is_empty());
    // A transform is read but not applied.
    let snippet = parse_snippet("${TM_FILENAME/(.*)\\.rs/${1:/upcase}/g}!", &vars);
    assert_eq!(snippet.text, "main.rs!");
}

#[test]
fn copies_of_a_stop_show_its_first_placeholder() {
    let snippet = plain("${1:x} = $1 + ${1:y}; $2");
    assert_eq!(snippet.text, "x = x + x; ");
    assert_eq!(ranges(&snippet, 1), [(0, 1), (4, 5), (8, 9)]);
    // A choice fills its copies too.
    assert_eq!(plain("$1 ${1|on,off|}").text, "on on");
}

#[test]
fn malformed_constructs_stay_text() {
    assert_eq!(plain("${1:open").text, "${1:open");
    assert_eq!(plain("${1:a ${2:b}").text, "${1:a b");
    assert_eq!(plain("${1:a ${2:b}").stops.len(), 1);
    assert_eq!(plain("$").text, "$");
    assert_eq!(plain("${x").text, "${x");
    assert_eq!(plain("${1x}").text, "${1x}");
}

#[test]
fn deep_or_self_copying_snippets_end_quickly() {
    let open = "${1:".repeat(500);
    let snippet = plain(&format!("{open}x"));
    assert!(snippet.text.ends_with('x'));
    // A placeholder that holds its own copy is written once.
    let snippet = plain("${1:a$1}");
    assert_eq!(snippet.text, "a");
    // Copies that double at every level stop at the node budget.
    let mut doubling = String::new();
    for index in 1u32..40 {
        let next = index.saturating_add(1);
        doubling.push_str(&format!("${{{index}:${next}${next}}}"));
    }
    let snippet = plain(&doubling);
    assert!(snippet.stops.len() <= 100_000);
}

#[test]
fn a_long_chain_of_shared_placeholders_expands_on_a_small_stack() {
    // `${1:$2}${2:$3}…${N:$1}`: each copy writes the next placeholder.
    let count = 50_000u32;
    let mut chain = String::new();
    for index in 1..=count {
        let next = index % count + 1;
        chain.push_str(&format!("${{{index}:${next}}}"));
    }
    let expanded = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || plain(&chain))
        .expect("thread")
        .join()
        .expect("no stack overflow");
    assert!(expanded.stops.len() <= 100_000);
}

#[test]
fn many_copies_of_a_long_placeholder_stop_at_the_size_limit() {
    let long = "x".repeat(10_000);
    let copies = "$1".repeat(10_000);
    // Written out, this would be 100 million chars.
    let snippet = plain(&format!("${{1:{long}}}{copies}"));
    let len = snippet.text.chars().count();
    assert!(len <= 1 << 20, "{len}");
    assert!(snippet.stops.iter().all(|stop| stop.end <= len));
}

#[test]
fn the_caret_without_a_session_goes_to_the_first_stop() {
    assert_eq!(plain("push(${1:value})$0").caret(), 5);
    assert_eq!(plain("$2 ${1:a}").caret(), 1);
    assert_eq!(plain("foo($0)").caret(), 4);
    assert_eq!(Snippet::plain("plain").caret(), 5);
    assert_eq!(
        plain("a${0:b}c").final_stop(),
        Some(&SnippetStop {
            index: 0,
            start: 1,
            end: 2
        })
    );
}
