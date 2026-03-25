# lensup
# 100% AI Coding
一个用 LLM 分析提交并按月生成分类报告的小工具。

功能简介
- 从本地 git 仓库抓取提交记录。
- 将提交的 diff 发送到配置的 LLM 进行分析。
- 将分析结果归类为若干固定类别（如：bug修复、功能增强、文档变更、重构、其他等）。
- 按月将分类好的 Markdown 报告写入 `reports/` 目录下。
- 结构化日志同时输出到终端和每日滚动的文件（`logs/`）。

快速开始

前置条件
- 已安装 Rust 工具链（stable）和 Cargo
- 已安装 Git（且可在 PATH 中使用）
- 可访问的 LLM 服务地址与 API Key（参见下方环境变量）

编译

```sh
cargo build --release
```

运行示例

按单月运行（示例）：

```sh
# 设置必需的环境变量
export DEEPSEEK_API_KEY="..."
export LLM_BASE_URL="https://your-llm-endpoint/v1/chat/completions"
export REPO_PATH="/path/to/your/repo"

# 分析 2026-03
cargo run -- --from 2026-03
```

或直接运行已编译的二进制：

```sh
RUST_LOG=debug ./target/release/pr-tools --from 2026-03
```

环境变量说明
- DEEPSEEK_API_KEY（必需）— LLM 服务的 API Key
- LLM_BASE_URL（必需）— LLM API 的完整 URL，用于 POST 请求
- REPO_PATH（必需）— 要分析的本地仓库路径
- COMMIT_LIMIT（可选）— 最大处理提交数（默认：5）(填0或负数表示不限制)
- ANALYSIS_DELAY_MONTHS（可选）— 自动选择月份时的延迟阈值（单位：月，默认：6）
- ANALYSIS_FROM / ANALYSIS_TO（可选）— YYYY-MM 格式的区间，会覆盖默认选择
- ANALYSIS_ONLY_CATEGORIZED（可选）— 设置为 0/false 时同时写入根级月度文件（默认：true，只写分类文件）
- ANALYSIS_LIMIT（可选）— 等同于覆盖 COMMIT_LIMIT
- RUST_LOG（可选）— 日志级别（info、debug、warn、error），默认：info

日志
- 程序会同时将日志输出到 stdout 和位于 `logs/` 的每日滚动日志文件中。
- 默认情况下每天会生成一个 `logs/pr-tools.log`（按日期滚动）。若需要详细调试信息，可使用 `RUST_LOG=debug`。

报告目录结构
- 根级报告位于 `reports/`，按月的分类报告在 `reports/YYYY-MM/` 下。
- 例如：
  - `reports/2026-03/功能增强.md`
  - `reports/2026-03/bug修复.md`

故障排查
- 构建时链接器崩溃：某些环境下 rust-lld 会导致链接失败或崩溃。仓库中提供了 `.cargo/config.toml`（将链接器切换为系统的 `gcc`）作为临时解决方案。如果遇到链接问题，可尝试安装系统链接器或删除该配置以尝试其他链接器。
- LLM 调用错误/超时：程序会在日志中记录 HTTP 状态码及响应预览。若遇到网络或超时问题，可考虑在代码中添加重试或增大超时设置。

自定义建议
- 通过环境变量让日志路径和保留策略可配置化。
- 为解析函数（例如 `parse_yyyy_mm`）添加单元测试。
- 在 LLM 调用处加入重试/退避和速率限制处理。

许可证与贡献
- 欢迎提交 issue 或 PR，共同改进。

---

快速运行示例

```sh
# 最小示例
DEEPSEEK_API_KEY="..." LLM_BASE_URL="https://..." REPO_PATH="/path/to/repo" cargo run -- --from 2026-03
```

