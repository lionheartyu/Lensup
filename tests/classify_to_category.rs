#[cfg(test)]
mod tests {
    use pr_tools::*;

    #[test]
    fn test_classify_to_category_bug() {
        assert_eq!(classify_to_category("修复 bug"), "bug修复");
        assert_eq!(classify_to_category("This is a bug fix"), "bug修复");
    }

    #[test]
    fn test_classify_to_category_feature() {
        assert_eq!(classify_to_category("新增功能"), "功能增强");
        assert_eq!(classify_to_category("feature: add something"), "功能增强");
    }

    #[test]
    fn test_classify_to_category_translation() {
        assert_eq!(classify_to_category("翻译更新"), "功能增强");
        assert_eq!(classify_to_category("translation: update"), "功能增强");
        assert_eq!(classify_to_category("This commit translates docs"), "功能增强");
    }

    #[test]
    fn test_classify_to_category_other() {
        assert_eq!(classify_to_category("无关内容"), "其他");
    }
}
