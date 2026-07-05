# lensup 操作指南

## 1. 前置条件
- Rust 工具链（建议 stable）
- Git
- 可用的 LLM API Key 和服务地址

## 2. 环境变量
- `DEEPSEEK_API_KEY`：LLM 服务的 API Key（必填）
- `LLM_BASE_URL`：LLM API 完整 URL（必填）
- `REPO_PATH`：本地待分析仓库路径（必填）
- `COMMIT_LIMIT`：最大分析提交数（可选，默认0为不限制）
- `ANALYSIS_DELAY_MONTHS`:(默认6个月，但是不分析需要填写日期)如果用 --from/--to 指定了具体区间，这个参数不会生效。
- `RUST_LOG`：日志级别（可设置，info/debug/warn/error）

## 3. 编译
在rust环境配置好的情况下
```sh
chmod +x set_mirror.sh
./set_mirror.sh

chmod +x build.sh
./build.sh
```
## 此处是REPO_PATH中去设置仓库路径的操作。
## 4. 运行
分析单月：比如分析apt 3月份的commit
```sh
lensup apt --from 2026-03
```
分析区间：比如分析systemd 2月份和3月份的commit`
```sh
lensup systemd --from 2026-02 --to 2026-03
```

## 5. 输出说明
- 报告生成在 `reports/` 目录下，文件名如 `systemd-2026-03.md` 或 `xxx-2026-02-2026-03.md`
- 日志输出到终端和 `logs/` 目录

## 6. 常见问题
- LLM 调用失败：检查 API Key、URL、网络
- 构建失败：确保 Rust 和 Git 安装正常

