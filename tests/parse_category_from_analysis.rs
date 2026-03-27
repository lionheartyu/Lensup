#[cfg(test)]
mod tests {
    use pr_tools::*;

    #[test]
    fn test_parse_category_from_analysis_explicit() {
        assert_eq!(parse_category_from_analysis("分类：bug修复\n内容..."), Some("bug修复"));
        assert_eq!(parse_category_from_analysis("category: feature\n内容..."), Some("功能增强"));
        assert_eq!(parse_category_from_analysis("其他\n内容..."), Some("其他"));
        // 翻译相关
        assert_eq!(parse_category_from_analysis("分类：翻译\n内容..."), Some("功能增强"));
        assert_eq!(parse_category_from_analysis("category: translation\n内容..."), Some("功能增强"));
        assert_eq!(parse_category_from_analysis("翻译\n内容..."), Some("功能增强"));
    }

    #[test]
    fn test_parse_category_from_analysis_none() {
        assert_eq!(parse_category_from_analysis("无分类内容"), None);
    }
}
