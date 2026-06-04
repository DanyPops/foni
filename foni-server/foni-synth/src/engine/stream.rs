use regex::Regex;

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
    let para_re = Regex::new(r"\n\n+").expect("infallible");
    let sent_re = Regex::new(r"[.!?!?]\s+").expect("infallible");

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
    s = Regex::new(r"(?s)\n?```.*?```\n?")
        .expect("infallible")
        .replace_all(&s, "\n")
        .to_string();

    // Unclosed fenced code block
    s = Regex::new(r"(?s)```.*")
        .expect("infallible")
        .replace_all(&s, "")
        .to_string();

    // Inline code
    s = Regex::new(r"`[^`]+`")
        .expect("infallible")
        .replace_all(&s, "")
        .to_string();

    // Images
    s = Regex::new(r"!\[[^\]]*\]\([^)]*\)")
        .expect("infallible")
        .replace_all(&s, "")
        .to_string();

    // Links — keep text, drop URL
    s = Regex::new(r"\[([^\]]+)\]\([^)]*\)")
        .expect("infallible")
        .replace_all(&s, "$1")
        .to_string();

    // Headers
    s = Regex::new(r"(?m)^#{1,6}\s+")
        .expect("infallible")
        .replace_all(&s, "")
        .to_string();

    // Bold/italic — no backreferences in Rust regex, handle each variant
    s = Regex::new(r"\*{3}(.+?)\*{3}")
        .expect("infallible")
        .replace_all(&s, "$1")
        .to_string();
    s = Regex::new(r"\*{2}(.+?)\*{2}")
        .expect("infallible")
        .replace_all(&s, "$1")
        .to_string();
    s = Regex::new(r"\*(.+?)\*")
        .expect("infallible")
        .replace_all(&s, "$1")
        .to_string();
    s = Regex::new(r"_{3}(.+?)_{3}")
        .expect("infallible")
        .replace_all(&s, "$1")
        .to_string();
    s = Regex::new(r"_{2}(.+?)_{2}")
        .expect("infallible")
        .replace_all(&s, "$1")
        .to_string();
    s = Regex::new(r"_(.+?)_")
        .expect("infallible")
        .replace_all(&s, "$1")
        .to_string();

    // Blockquote
    s = Regex::new(r"(?m)^>\s*")
        .expect("infallible")
        .replace_all(&s, "")
        .to_string();

    // Horizontal rules
    s = Regex::new(r"(?m)^[-*_]{3,}\s*$")
        .expect("infallible")
        .replace_all(&s, "")
        .to_string();

    // Unordered list bullets
    s = Regex::new(r"(?m)^[\s]*[-*+]\s+")
        .expect("infallible")
        .replace_all(&s, "")
        .to_string();

    // Ordered list numbers
    s = Regex::new(r"(?m)^[\s]*\d+\.\s+")
        .expect("infallible")
        .replace_all(&s, "")
        .to_string();

    // Shell/regex escape sequences
    s = Regex::new(r"\\[|ntrfv\\]")
        .expect("infallible")
        .replace_all(&s, " ")
        .to_string();

    // Trailing backslash
    s = Regex::new(r"(?m)\\\s*$")
        .expect("infallible")
        .replace_all(&s, "")
        .to_string();

    // Path-like tokens (simplified — no lookbehind in Rust regex)
    s = Regex::new(r"(?i)(?:^|\s)/[a-z][a-z0-9_-]*")
        .expect("infallible")
        .replace_all(&s, "")
        .to_string();

    // Collapse multiple blank lines
    s = Regex::new(r"\n{3,}")
        .expect("infallible")
        .replace_all(&s, "\n\n")
        .to_string();

    s.trim().to_string()
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
}
