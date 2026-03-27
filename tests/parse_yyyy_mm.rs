#[cfg(test)]
mod tests {
    use lensup::*;

    #[test]
    fn test_parse_yyyy_mm_valid() {
        assert_eq!(parse_yyyy_mm("2026-03"), Some((2026, 3)));
        assert_eq!(parse_yyyy_mm("2026-12"), Some((2026, 12)));
        assert_eq!(parse_yyyy_mm("2026-1"), Some((2026, 1)));
    }

    #[test]
    fn test_parse_yyyy_mm_invalid() {
        assert_eq!(parse_yyyy_mm("2026-00"), None);
        assert_eq!(parse_yyyy_mm("2026-13"), None);
        assert_eq!(parse_yyyy_mm("2026"), None);
        assert_eq!(parse_yyyy_mm("abcd-ef"), None);
        assert_eq!(parse_yyyy_mm("2026-"), None);
    }
}
