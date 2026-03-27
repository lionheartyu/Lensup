# 100%AI Coding
# lensup 操作指南

## 1. 前置条件
- Rust 工具链（建议 stable）
- Git
- 可用的 LLM API Key 和服务地址

## 2. 环境变量
- `DEEPSEEK_API_KEY`：LLM 服务的 API Key（必填）
- `LLM_BASE_URL`：LLM API 完整 URL（必填）
- `REPO_PATH`：本地待分析仓库路径（必填）
- `COMMIT_LIMIT`：最大分析提交数（可选，默认5，0为不限制）
- `ANALYSIS_FROM`/`ANALYSIS_TO`：分析区间（YYYY-MM，可选）
- `RUST_LOG`：日志级别（可选，info/debug/warn/error）

## 3. 编译
```sh
cargo build --release
```

## 4. 运行
分析单月：比如分析3月份的commit
```sh
 cargo run -- --from 2026-03
 或运行二进制文件
 ./target/debug/pr-tools --from 2026-03
```
分析区间：比如分析2月份和3月份的commit`
```sh
cargo run -- --from 2026-02 --to 2026-03
或运行二进制文件
 ./target/debug/pr-tools --from 2026-02 --to 2026-03
```

## 5. 输出说明
- 报告生成在 `reports/` 目录下，文件名如 `systemd-2026-03.md` 或 `xxx-2026-02-2026-03.md`
- 日志输出到终端和 `logs/` 目录

## 6. 常见问题
- LLM 调用失败：检查 API Key、URL、网络
- 构建失败：确保 Rust 和 Git 安装正常

## 7. 最小运行示例
```sh
DEEPSEEK_API_KEY=xxx LLM_BASE_URL=https://... REPO_PATH=/your/repo cargo run -- --from 2026-03
```

