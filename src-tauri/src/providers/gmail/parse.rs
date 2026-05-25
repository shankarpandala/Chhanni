/// Extract a bare email address from a `From` header value like
/// `"Some Name" <addr@example.com>` or `addr@example.com`. Lower-cased so it
/// can be used as a clustering key.
pub fn parse_sender_email(from: &str) -> Option<String> {
    let trimmed = from.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Prefer the angle-bracketed form when present.
    if let Some(start) = trimmed.find('<') {
        if let Some(end) = trimmed[start + 1..].find('>') {
            let addr = trimmed[start + 1..start + 1 + end].trim();
            if addr.contains('@') {
                return Some(addr.to_ascii_lowercase());
            }
        }
    }

    if trimmed.contains('@') {
        return Some(trimmed.to_ascii_lowercase());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_angle_bracketed_address() {
        assert_eq!(
            parse_sender_email("\"Some Name\" <ADDR@example.com>").as_deref(),
            Some("addr@example.com")
        );
    }

    #[test]
    fn falls_back_to_bare_address() {
        assert_eq!(
            parse_sender_email("plain@example.com").as_deref(),
            Some("plain@example.com")
        );
    }

    #[test]
    fn returns_none_when_no_at_sign() {
        assert!(parse_sender_email("Just A Name").is_none());
    }

    #[test]
    fn returns_none_on_empty() {
        assert!(parse_sender_email("").is_none());
        assert!(parse_sender_email("   ").is_none());
    }
}
