#[cfg(test)]
mod tests {
    use pr_tools::*;

    #[test]
    fn test_month_index() {
        assert_eq!(month_index(2026, 3), 2026 * 12 + 3);
        assert_eq!(month_index(2000, 1), 2000 * 12 + 1);
        assert_eq!(month_index(0, 12), 0 * 12 + 12);
    }
}
