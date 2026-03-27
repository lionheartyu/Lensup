// 公共工具函数模块，供 main.rs 和 tests/ 复用

/// 解析 YYYY-MM (或 YYYY-M) 字符串为 (year, month)。解析失败或月份超出范围返回 None。
pub fn parse_yyyy_mm(s: &str) -> Option<(i32, u32)> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 2 { return None; }
    if let (Ok(y), Ok(m)) = (parts[0].parse::<i32>(), parts[1].parse::<u32>()) {
        if m >= 1 && m <= 12 {
            return Some((y, m));
        }
    }
    None
}

/// 将年/月转换为单调递增的月份索引，便于区间比较。
pub fn month_index(year: i32, month: u32) -> i32 {
    year * 12 + month as i32
}

/// 将 LLM 的分析文本映射到固定分类。
pub fn classify_to_category(analysis: &str) -> &'static str {
    let s = analysis.to_lowercase();
    if s.contains("bug") || s.contains("修复") || s.contains("修补") {
        return "bug修复";
    }
    if s.contains("翻译") || s.contains("translation") || s.contains("translate") {
        return "功能增强";
    }
    if s.contains("功能") || s.contains("增强") || s.contains("feature") {
        return "功能增强";
    }
    if s.contains("性能") || s.contains("优化") || s.contains("performance") {
        return "性能优化";
    }
    if s.contains("安全") || s.contains("vuln") || s.contains("cve") {
        return "安全修复";
    }
    if s.contains("build") || s.contains("ci") || s.contains("cmake") || s.contains("makefile") {
        return "构建/CI";
    }
    if s.contains("配置") || s.contains("config") || s.contains("配置变更") {
        return "配置变更";
    }
    if s.contains("兼容") || s.contains("compat") {
        return "兼容性";
    }
    if s.contains("文档") || s.contains("man/") || s.contains("readme") {
        return "文档变更";
    }
    if s.contains("重构") || s.contains("refactor") {
        return "重构";
    }
    "其他"
}

/// 尝试从 LLM 的分析文本首行解析明确的分类声明。
pub fn parse_category_from_analysis(analysis: &str) -> Option<&'static str> {
    if let Some(first_line) = analysis.lines().next() {
        let s = first_line.trim().to_lowercase();
        let s = s.strip_prefix("分类：").or_else(|| s.strip_prefix("分类:")).unwrap_or(&s);
        let s = s.strip_prefix("category:").unwrap_or(s).trim();
        match s {
            "bug修复" | "bug" | "修复" | "修补" => return Some("bug修复"),
            "翻译" | "translation" | "translate" => return Some("功能增强"),
            "功能增强" | "功能" | "feature" => return Some("功能增强"),
            "性能优化" | "性能" | "优化" => return Some("性能优化"),
            "安全修复" | "安全" | "cve" | "vuln" => return Some("安全修复"),
            "构建" | "ci" | "构建/ci" | "build" => return Some("构建/CI"),
            "配置变更" | "配置" | "config" => return Some("配置变更"),
            "兼容性" | "兼容" | "compat" => return Some("兼容性"),
            "文档变更" | "文档" => return Some("文档变更"),
            "重构" | "refactor" => return Some("重构"),
            "测试" => return Some("测试"),
            "其他" | "other" => return Some("其他"),
            _ => {}
        }
    }
    None
}
