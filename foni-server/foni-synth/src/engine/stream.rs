use once_cell::sync::Lazy;
use regex::Regex;

/// Compile once, reuse forever. All patterns are correct by construction —
/// if a pattern is malformed the binary will not start (caught at first call).
macro_rules! re {
    ($pat:expr) => {{
        static RE: Lazy<Regex> = Lazy::new(|| Regex::new($pat).expect("regex compile"));
        &*RE
    }};
}

#[derive(Debug, Clone, Default)]
pub struct StreamState {
    pub buffer: String,
    pub code_depth: usize,
    pub in_inline_code: bool,
    pub backtick_run: usize,
}

pub fn fresh_state() -> StreamState {
    StreamState::default()
}

pub fn resolve_backtick_run(state: &mut StreamState) {
    let run = state.backtick_run;
    state.backtick_run = 0;
    if run == 0 {
        return;
    }
    if run >= 3 {
        state.code_depth = if state.code_depth > 0 { 0 } else { 1 };
    } else if state.code_depth == 0 {
        state.in_inline_code = !state.in_inline_code;
    }
}

pub struct DrainResult {
    pub chunks: Vec<String>,
    pub remainder: String,
}

pub fn drain_chunks(text: &str) -> DrainResult {
    let para_re = re!(r"\n\n+");
    let sent_re = re!(r"[.!?!?]\s+");

    let mut chunks = Vec::new();
    let mut remaining = text.to_string();

    for _ in 0..50 {
        let para_idx = para_re.find(&remaining).map(|m| m.start());
        let sent_match = sent_re.find(&remaining);
        let sent_idx = sent_match.map(|m| m.start() + 1);

        let pi = para_idx.unwrap_or(usize::MAX);
        let si = sent_idx.unwrap_or(usize::MAX);

        if pi == usize::MAX && si == usize::MAX {
            break;
        }

        if pi <= si {
            let chunk = remaining[..pi].trim().to_string();
            if chunk.len() > 2 {
                chunks.push(chunk);
            }
            remaining = remaining[pi..].trim_start_matches('\n').to_string();
        } else if let Some(sm) = sent_match {
            let cut = sm.start() + 1;
            let chunk = remaining[..cut].trim().to_string();
            if chunk.len() > 2 {
                chunks.push(chunk);
            }
            remaining = remaining[cut..].trim_start().to_string();
        } else {
            break;
        }
    }

    DrainResult {
        chunks,
        remainder: remaining,
    }
}

pub fn strip_markdown(text: &str) -> String {
    let mut s = text.to_string();

    // Fenced code blocks (closed)
    s = re!(r"(?s)\n?```.*?```\n?")
        .replace_all(&s, "\n")
        .to_string();

    // Unclosed fenced code block
    s = re!(r"(?s)```.*").replace_all(&s, "").to_string();

    // Inline code
    s = re!(r"`[^`]+`").replace_all(&s, "").to_string();

    // Images
    s = re!(r"!\[[^\]]*\]\([^)]*\)").replace_all(&s, "").to_string();

    // Links — keep text, drop URL
    s = re!(r"\[([^\]]+)\]\([^)]*\)")
        .replace_all(&s, "$1")
        .to_string();

    // Headers
    s = re!(r"(?m)^#{1,6}\s+").replace_all(&s, "").to_string();

    // Bold/italic — no backreferences in Rust regex, handle each variant
    s = re!(r"\*{3}(.+?)\*{3}").replace_all(&s, "$1").to_string();
    s = re!(r"\*{2}(.+?)\*{2}").replace_all(&s, "$1").to_string();
    s = re!(r"\*(.+?)\*").replace_all(&s, "$1").to_string();
    s = re!(r"_{3}(.+?)_{3}").replace_all(&s, "$1").to_string();
    s = re!(r"_{2}(.+?)_{2}").replace_all(&s, "$1").to_string();
    s = re!(r"_(.+?)_").replace_all(&s, "$1").to_string();

    // Blockquote
    s = re!(r"(?m)^>\s*").replace_all(&s, "").to_string();

    // ASCII horizontal rules (--- / *** / ___)
    s = re!(r"(?m)^[-*_]{3,}\s*$").replace_all(&s, "").to_string();

    // Unicode visual-formatting noise — lines made purely of Box Drawing (U+2500–U+257F)
    // or Block Elements (U+2580–U+259F): ─ │ ┌ ┐ █ ▄ ▀ ▌ ░ etc.
    // One contiguous range covers both complete Unicode blocks.
    s = re!(r"(?m)^[\u{2500}-\u{259F}\s]{2,}$")
        .replace_all(&s, "")
        .to_string();

    // Same chars inline flanking text ('─── Section ────') — strip chars, keep text.
    s = re!(r"[\u{2500}-\u{259F}]+")
        .replace_all(&s, " ")
        .to_string();

    // Markdown table separator rows (|---|---|, :---:|, etc.) — drop entirely.
    s = re!(r"(?m)^\|[-:\s|]+\|\s*$")
        .replace_all(&s, "")
        .to_string();

    // Markdown table data rows — strip the pipes, keep cell text.
    // '| foo | bar |' → 'foo  bar'
    s = re!(r"(?m)^\|(.+)\|\s*$")
        .replace_all(&s, |caps: &regex::Captures| {
            caps[1].replace('|', " ").trim().to_string()
        })
        .to_string();

    // Lines that are pure non-alphabetic noise after all passes — drop.
    // Matches lines with no Unicode letter or digit at all.
    s = re!(r"(?m)^[^\p{L}\p{N}\n]*$")
        .replace_all(&s, "")
        .to_string();

    // Unordered list bullets
    s = re!(r"(?m)^[\s]*[-*+]\s+").replace_all(&s, "").to_string();

    // Ordered list numbers
    s = re!(r"(?m)^[\s]*\d+\.\s+").replace_all(&s, "").to_string();

    // Shell/regex escape sequences
    s = re!(r"\\[|ntrfv\\]").replace_all(&s, " ").to_string();

    // Trailing backslash
    s = re!(r"(?m)\\\s*$").replace_all(&s, "").to_string();

    // Path-like tokens (simplified — no lookbehind in Rust regex)
    s = re!(r"(?i)(?:^|\s)/[a-z][a-z0-9_-]*")
        .replace_all(&s, "")
        .to_string();

    // Collapse multiple blank lines
    s = re!(r"\n{3,}").replace_all(&s, "\n\n").to_string();

    s = normalise_numbers(&s);
    s.trim().to_string()
}

// ── Russian number normalisation ───────────────────────────────────────────────

/// Convert a non-negative integer (up to 999 999 999) to Russian nominative cardinal words.
///
/// Gender agreement: masculine for standalone numbers and millions;
/// feminine for the thousands multiplier (одна тысяча, две тысячи).
pub fn num_to_words_ru(n: i64) -> String {
    if n < 0 {
        return format!("минус {}", num_to_words_ru(-n));
    }
    if n == 0 {
        return "ноль".to_string();
    }
    num_below_billion(n as u64)
}

fn num_below_billion(n: u64) -> String {
    let mut parts: Vec<String> = Vec::new();

    if n >= 1_000_000 {
        let m = n / 1_000_000;
        let form = match m % 100 {
            11..=19 => "миллионов",
            _ => match m % 10 {
                1 => "миллион",
                2..=4 => "миллиона",
                _ => "миллионов",
            },
        };
        parts.push(format!("{} {form}", num_below_thousand(m, false)));
    }

    let sub_m = n % 1_000_000;
    if sub_m >= 1_000 {
        let t = sub_m / 1_000;
        let form = match t % 100 {
            11..=19 => "тысяч",
            _ => match t % 10 {
                1 => "тысяча",
                2..=4 => "тысячи",
                _ => "тысяч",
            },
        };
        // Thousands multiplier uses feminine (одна/две вместо один/два).
        parts.push(format!("{} {form}", num_below_thousand(t, true)));
    }

    let sub_t = n % 1_000;
    if sub_t > 0 {
        parts.push(num_below_thousand(sub_t, false));
    }

    parts.join(" ")
}

/// Render 1–999 in Russian. `feminine` flips один→одна and два→две.
fn num_below_thousand(n: u64, feminine: bool) -> String {
    const HUNDREDS: &[&str] = &[
        "",
        "сто",
        "двести",
        "триста",
        "четыреста",
        "пятьсот",
        "шестьсот",
        "семьсот",
        "восемьсот",
        "девятьсот",
    ];
    const TEENS: &[&str] = &[
        "десять",
        "одиннадцать",
        "двенадцать",
        "тринадцать",
        "четырнадцать",
        "пятнадцать",
        "шестнадцать",
        "семнадцать",
        "восемнадцать",
        "девятнадцать",
    ];
    const TENS: &[&str] = &[
        "",
        "",
        "двадцать",
        "тридцать",
        "сорок",
        "пятьдесят",
        "шестьдесят",
        "семьдесят",
        "восемьдесят",
        "девяносто",
    ];
    const ONES_M: &[&str] = &[
        "",
        "один",
        "два",
        "три",
        "четыре",
        "пять",
        "шесть",
        "семь",
        "восемь",
        "девять",
    ];
    const ONES_F: &[&str] = &[
        "",
        "одна",
        "две",
        "три",
        "четыре",
        "пять",
        "шесть",
        "семь",
        "восемь",
        "девять",
    ];

    let mut parts: Vec<&str> = Vec::new();
    let h = (n / 100) as usize;
    let rem = n % 100;

    if h > 0 {
        parts.push(HUNDREDS[h]);
    }

    if (10..=19).contains(&rem) {
        parts.push(TEENS[(rem - 10) as usize]);
    } else {
        let t = (rem / 10) as usize;
        let o = (rem % 10) as usize;
        if t > 1 {
            parts.push(TENS[t]);
        }
        if o > 0 {
            parts.push(if feminine { ONES_F[o] } else { ONES_M[o] });
        }
    }

    parts.join(" ")
}

/// Replace standalone Arabic integers in `s` with Russian word forms.
///
/// Skips numbers that are part of:
///   - version strings: letter immediately before (`v2`, `Python3`)
///   - decimals / IPs: digit or dot immediately after (`3.14`, `192.168`)
///   - unit suffixes: letter immediately after (`3D`, `50px`, `mp4`)
///   - percentage: digit followed by `%` → appends «процентов»
pub fn normalise_numbers(s: &str) -> String {
    let re = re!(r"-?\d+");
    let mut out = String::with_capacity(s.len());
    let mut cursor = 0usize;

    for mat in re.find_iter(s) {
        let start = mat.start();
        let end = mat.end();

        // Emit everything up to this match unchanged.
        out.push_str(&s[cursor..start]);
        cursor = end;

        let raw = mat.as_str();

        // Char immediately before the match.
        let prev = if start > 0 {
            s[..start].chars().last()
        } else {
            None
        };
        // Char immediately after the match.
        let next = s[end..].chars().next();

        // Skip if the number is embedded in a larger token.
        let skip = prev.is_some_and(|c| c.is_alphabetic() || c == '.')
            || next.is_some_and(|c| c == '.' || c.is_alphabetic() || c == '_');

        if skip {
            out.push_str(raw);
            continue;
        }

        // Percentage: replace "50%" with "пятьдесят процентов".
        let pct = next == Some('%');
        if pct {
            // Consume the % too.
            cursor += '%'.len_utf8();
        }

        let replacement = raw
            .parse::<i64>()
            .map(num_to_words_ru)
            .unwrap_or_else(|_| raw.to_string());

        if pct {
            out.push_str(&replacement);
            out.push_str(" процентов");
        } else {
            out.push_str(&replacement);
        }
    }

    out.push_str(&s[cursor..]);
    out
}

/// Feed one delta character into the stream state, appending non-code text to the buffer.
pub fn feed_delta(state: &mut StreamState, delta: &str) {
    for ch in delta.chars() {
        if ch == '`' {
            state.backtick_run += 1;
        } else {
            if state.backtick_run > 0 {
                resolve_backtick_run(state);
            }
            if state.code_depth == 0 && !state.in_inline_code {
                state.buffer.push(ch);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_state_is_zeroed() {
        let s = fresh_state();
        assert!(s.buffer.is_empty());
        assert_eq!(s.code_depth, 0);
        assert!(!s.in_inline_code);
        assert_eq!(s.backtick_run, 0);
    }

    #[test]
    fn triple_backtick_opens_code_fence() {
        let mut s = fresh_state();
        s.backtick_run = 3;
        resolve_backtick_run(&mut s);
        assert_eq!(s.code_depth, 1);
        assert_eq!(s.backtick_run, 0);
    }

    #[test]
    fn triple_backtick_closes_open_fence() {
        let mut s = fresh_state();
        s.code_depth = 1;
        s.backtick_run = 3;
        resolve_backtick_run(&mut s);
        assert_eq!(s.code_depth, 0);
    }

    #[test]
    fn single_backtick_toggles_inline_code() {
        let mut s = fresh_state();
        s.backtick_run = 1;
        resolve_backtick_run(&mut s);
        assert!(s.in_inline_code);
    }

    #[test]
    fn zero_run_is_noop() {
        let mut s = fresh_state();
        resolve_backtick_run(&mut s);
        assert_eq!(s.code_depth, 0);
        assert!(!s.in_inline_code);
    }

    #[test]
    fn drain_mid_sentence_returns_no_chunks() {
        let r = drain_chunks("Hello there");
        assert!(r.chunks.is_empty());
        assert_eq!(r.remainder, "Hello there");
    }

    #[test]
    fn drain_splits_on_sentence_boundary() {
        let r = drain_chunks("Hello there. World is great.");
        assert!(r.chunks.contains(&"Hello there.".to_string()));
        assert!(r.remainder.contains("World"));
    }

    #[test]
    fn drain_splits_on_paragraph_break() {
        let r = drain_chunks("First para.\n\nSecond para.");
        assert!(r.chunks.contains(&"First para.".to_string()));
        assert!(r.remainder.contains("Second para"));
    }

    #[test]
    fn drain_skips_short_chunks() {
        let r = drain_chunks("Hi.\n\nHello world.");
        assert!(!r.chunks.iter().any(|c| c == "Hi"));
    }

    #[test]
    fn drain_handles_multiple_sentences() {
        let r = drain_chunks("One. Two. Three.");
        assert_eq!(r.chunks.len(), 2);
    }

    #[test]
    fn drain_splits_russian_sentences() {
        let r = drain_chunks("Привет, сталкер. Как дела?");
        assert!(r.chunks.contains(&"Привет, сталкер.".to_string()));
    }

    #[test]
    fn drain_splits_on_exclamation_and_question() {
        let r = drain_chunks("Wow! Really? Yes.");
        assert_eq!(r.chunks.len(), 2);
    }

    #[test]
    fn strip_fenced_code_blocks() {
        let input = "Hello\n```ts\nconst x = 1;\n```\nworld";
        assert_eq!(strip_markdown(input), "Hello\nworld");
    }

    #[test]
    fn strip_inline_code() {
        let result = strip_markdown("use `npm install` to install");
        assert_eq!(result, "use  to install");
    }

    #[test]
    fn strip_headers() {
        assert_eq!(strip_markdown("## Hello\nworld"), "Hello\nworld");
    }

    #[test]
    fn strip_preserves_link_text() {
        assert_eq!(
            strip_markdown("[click here](https://example.com)"),
            "click here"
        );
    }

    #[test]
    fn strip_bold_and_italic() {
        assert_eq!(strip_markdown("**bold** and *italic*"), "bold and italic");
    }

    #[test]
    fn strip_blockquote() {
        assert_eq!(strip_markdown("> quoted text"), "quoted text");
    }

    #[test]
    fn strip_unordered_list() {
        assert_eq!(
            strip_markdown("- item one\n- item two"),
            "item one\nitem two"
        );
    }

    #[test]
    fn strip_ordered_list() {
        assert_eq!(strip_markdown("1. first\n2. second"), "first\nsecond");
    }

    #[test]
    fn strip_collapses_blank_lines() {
        assert_eq!(strip_markdown("a\n\n\n\nb"), "a\n\nb");
    }

    #[test]
    fn strip_clean_prose_unchanged() {
        assert_eq!(strip_markdown("Just plain text."), "Just plain text.");
    }

    #[test]
    fn feed_delta_skips_code_blocks() {
        let mut s = fresh_state();
        feed_delta(&mut s, "before ");
        assert_eq!(s.buffer, "before ");
        // Feed ``` then a non-backtick to resolve the run
        feed_delta(&mut s, "```");
        feed_delta(&mut s, "c"); // resolves backtick_run=3 → code_depth=1, 'c' is inside code
        assert_eq!(s.code_depth, 1);
        feed_delta(&mut s, "ode");
        assert_eq!(s.buffer, "before "); // no code text leaked
    }

    #[test]
    fn feed_delta_skips_inline_code() {
        let mut s = fresh_state();
        feed_delta(&mut s, "use ");
        feed_delta(&mut s, "`");
        feed_delta(&mut s, "x"); // should trigger resolve (backtick_run=1), then x is inline-code
        assert!(!s.buffer.contains("x") || s.buffer == "use ");
    }

    // ─ num_to_words_ru ───────────────────────────────────────────

    #[test]
    fn zero() {
        assert_eq!(num_to_words_ru(0), "ноль");
    }

    #[test]
    fn units() {
        let cases = [
            (1, "один"),
            (2, "два"),
            (3, "три"),
            (4, "четыре"),
            (5, "пять"),
            (6, "шесть"),
            (7, "семь"),
            (8, "восемь"),
            (9, "девять"),
        ];
        for (n, w) in cases {
            assert_eq!(num_to_words_ru(n), w, "n={n}");
        }
    }

    #[test]
    fn teens() {
        let cases = [
            (10, "десять"),
            (11, "одиннадцать"),
            (12, "двенадцать"),
            (13, "тринадцать"),
            (14, "четырнадцать"),
            (15, "пятнадцать"),
            (16, "шестнадцать"),
            (17, "семнадцать"),
            (18, "восемнадцать"),
            (19, "девятнадцать"),
        ];
        for (n, w) in cases {
            assert_eq!(num_to_words_ru(n), w, "n={n}");
        }
    }

    #[test]
    fn tens() {
        let cases = [
            (20, "двадцать"),
            (21, "двадцать один"),
            (30, "тридцать"),
            (40, "сорок"),
            (50, "пятьдесят"),
            (60, "шестьдесят"),
            (70, "семьдесят"),
            (80, "восемьдесят"),
            (90, "девяносто"),
            (99, "девяносто девять"),
        ];
        for (n, w) in cases {
            assert_eq!(num_to_words_ru(n), w, "n={n}");
        }
    }

    #[test]
    fn hundreds() {
        let cases = [
            (100, "сто"),
            (101, "сто один"),
            (200, "двести"),
            (300, "триста"),
            (400, "четыреста"),
            (500, "пятьсот"),
            (600, "шестьсот"),
            (700, "семьсот"),
            (800, "восемьсот"),
            (900, "девятьсот"),
            (999, "девятьсот девяносто девять"),
        ];
        for (n, w) in cases {
            assert_eq!(num_to_words_ru(n), w, "n={n}");
        }
    }

    #[test]
    fn thousands() {
        let cases = [
            (1_000, "одна тысяча"),
            (2_000, "две тысячи"),
            (3_000, "три тысячи"),
            (4_000, "четыре тысячи"),
            (5_000, "пять тысяч"),
            (11_000, "одиннадцать тысяч"),
            (21_000, "двадцать одна тысяча"),
            (1_001, "одна тысяча один"),
            (1_100, "одна тысяча сто"),
        ];
        for (n, w) in cases {
            assert_eq!(num_to_words_ru(n), w, "n={n}");
        }
    }

    #[test]
    fn millions() {
        let cases = [
            (1_000_000, "один миллион"),
            (2_000_000, "два миллиона"),
            (5_000_000, "пять миллионов"),
            (1_000_001, "один миллион один"),
        ];
        for (n, w) in cases {
            assert_eq!(num_to_words_ru(n), w, "n={n}");
        }
    }

    #[test]
    fn negative() {
        assert_eq!(num_to_words_ru(-1), "минус один");
        assert_eq!(num_to_words_ru(-100), "минус сто");
    }

    // ─ normalise_numbers ────────────────────────────────────────

    // ─ formatting noise stripping ──────────────────────────────────────────

    #[test]
    fn strip_unicode_horizontal_rule() {
        // A line of U+2500 box-drawing chars should disappear entirely.
        let out = strip_markdown(
            "────────────────────────────────────────────────────────────────────────────────",
        );
        assert!(out.is_empty(), "got: {out:?}");
    }

    #[test]
    fn strip_double_line_box_rule() {
        // U+2550 is inside Box Drawing (U+2500-U+257F) — covered by the same range.
        let out = strip_markdown("════════════════════════════════");
        assert!(out.is_empty(), "got: {out:?}");
    }

    #[test]
    fn strip_block_elements_line() {
        // Block Elements (U+2580–U+259F): █ ▄ ░ etc. appear in LLM progress bars.
        let out = strip_markdown("████████░░░░░░░░");
        assert!(out.is_empty(), "got: {out:?}");
    }

    #[test]
    fn strip_box_chars_keeps_label_text() {
        // '─── Section title ────────' should keep 'Section title'.
        let out = strip_markdown("─── Section title ────────");
        assert!(out.contains("Section title"), "got: {out:?}");
        assert!(!out.contains('─'), "box chars should be gone: {out:?}");
    }

    #[test]
    fn strip_table_separator_row() {
        let out = strip_markdown("|---|---|---|");
        assert!(out.is_empty(), "got: {out:?}");
    }

    #[test]
    fn strip_table_separator_with_alignment() {
        let out = strip_markdown("| :--- | :---: | ---: |");
        assert!(out.is_empty(), "got: {out:?}");
    }

    #[test]
    fn strip_table_data_row_keeps_cell_text() {
        // '| Pattern | Problem | Solution |' → cell content survives, pipes gone.
        let out = strip_markdown("| Pattern | Problem | Solution |");
        assert!(out.contains("Pattern"), "got: {out:?}");
        assert!(out.contains("Problem"), "got: {out:?}");
        assert!(out.contains("Solution"), "got: {out:?}");
        assert!(!out.contains('|'), "pipes should be gone: {out:?}");
    }

    #[test]
    fn strip_full_markdown_table() {
        let table = "| Col A | Col B |\n|---|---|\n| foo | bar |\n| baz | qux |";
        let out = strip_markdown(table);
        assert!(!out.contains('|'), "pipes should be gone: {out:?}");
        assert!(!out.contains("---"), "separator should be gone: {out:?}");
        assert!(out.contains("foo"), "cell text should survive: {out:?}");
        assert!(out.contains("bar"), "cell text should survive: {out:?}");
    }

    #[test]
    fn strip_pure_noise_line() {
        // A line with only punctuation and no letters/digits should vanish.
        let out = strip_markdown("=============================");
        assert!(out.is_empty(), "got: {out:?}");
    }

    #[test]
    fn strip_noise_preserves_surrounding_prose() {
        let text = "Some prose.\n────────────────────\nMore prose.";
        let out = strip_markdown(text);
        assert!(out.contains("Some prose"), "got: {out:?}");
        assert!(out.contains("More prose"), "got: {out:?}");
        assert!(!out.contains('─'), "rule should be gone: {out:?}");
    }

    #[test]
    fn normalise_simple_sequence() {
        // "1, 2, 3" → words
        let out = normalise_numbers("1, 2, 3");
        assert!(out.contains("один"), "got: {out}");
        assert!(out.contains("два"), "got: {out}");
        assert!(out.contains("три"), "got: {out}");
    }

    #[test]
    fn normalise_hundreds() {
        let out = normalise_numbers("100, 200, 300");
        assert!(out.contains("сто"), "got: {out}");
        assert!(out.contains("двести"), "got: {out}");
        assert!(out.contains("триста"), "got: {out}");
    }

    #[test]
    fn normalise_skips_version_strings() {
        // "v2" and "Python3" — letter precedes digit
        let out = normalise_numbers("v2 Python3");
        assert!(out.contains("v2"), "got: {out}");
        assert!(out.contains("Python3"), "got: {out}");
    }

    #[test]
    fn normalise_skips_decimals() {
        let out = normalise_numbers("3.14");
        assert!(out.contains("3.14"), "got: {out}");
    }

    #[test]
    fn normalise_skips_ip_addresses() {
        let out = normalise_numbers("192.168.1.1");
        assert!(out.contains("192.168.1.1"), "got: {out}");
    }

    #[test]
    fn normalise_skips_unit_suffixes() {
        // "50px", "3D" — letter follows digit
        let out = normalise_numbers("50px 3D");
        assert!(out.contains("50px"), "got: {out}");
        assert!(out.contains("3D"), "got: {out}");
    }

    #[test]
    fn normalise_percentage() {
        let out = normalise_numbers("50%");
        assert!(out.contains("пятьдесят"), "got: {out}");
        assert!(out.contains("процентов"), "got: {out}");
    }

    #[test]
    fn normalise_in_strip_markdown() {
        // End-to-end: strip_markdown calls normalise_numbers
        let out = strip_markdown("The server handled 100 requests.");
        assert!(out.contains("сто"), "expected 'сто' in: {out}");
        assert!(!out.contains("100"), "raw digit should be gone: {out}");
    }
}
