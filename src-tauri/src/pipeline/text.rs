/// Build the deterministic embedding input for one message.
///
/// We deliberately concatenate `subject || \n || sender || \n || snippet[..500]`
/// so that the model attends to the most discriminative fields. Truncating the
/// snippet bounds the prompt and keeps embedding latency stable.
pub fn build_embedding_input(
    subject: Option<&str>,
    sender: Option<&str>,
    snippet: Option<&str>,
) -> String {
    const SNIPPET_LIMIT: usize = 500;
    let mut buf = String::with_capacity(SNIPPET_LIMIT + 200);
    if let Some(s) = subject {
        buf.push_str(s.trim());
    }
    buf.push('\n');
    if let Some(s) = sender {
        buf.push_str(s.trim());
    }
    buf.push('\n');
    if let Some(s) = snippet {
        let trimmed = s.trim();
        // Char-bounded so we don't slice across a UTF-8 codepoint.
        let cap = trimmed
            .char_indices()
            .nth(SNIPPET_LIMIT)
            .map(|(idx, _)| idx)
            .unwrap_or(trimmed.len());
        buf.push_str(&trimmed[..cap]);
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_inputs_yield_two_newlines() {
        let s = build_embedding_input(None, None, None);
        assert_eq!(s, "\n\n");
    }

    #[test]
    fn truncates_snippet_at_500_chars_not_bytes() {
        let snippet = "ä".repeat(800); // 2 bytes per char in UTF-8
        let s = build_embedding_input(Some("subj"), Some("from"), Some(&snippet));
        // After the two header lines + newlines, the remainder is the snippet
        // truncated to 500 chars (1000 bytes).
        let rest = s.splitn(3, '\n').nth(2).unwrap();
        assert_eq!(rest.chars().count(), 500);
    }

    #[test]
    fn trims_surrounding_whitespace() {
        let s = build_embedding_input(Some("  hi  "), Some("  me  "), Some("  body  "));
        assert_eq!(s, "hi\nme\nbody");
    }
}
