// =============================
//
//           lensup1.0
// =============================

use std::process::Command;
use std::env;
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use chrono::{DateTime, Datelike, FixedOffset};
use lensup::{parse_yyyy_mm, month_index, classify_to_category, parse_category_from_analysis};
// use tokio::runtime::Runtime; // 已不再需要
use futures::future::{join_all, ready};
// 用LLM对详细分析内容生成30字以内完整句子
async fn llm_summarize_30(api_url: &str, api_key: &str, detail: String) -> Result<String, reqwest::Error> {
	let prompt = "请你仅用一句完整的中文句子、严格30字左右，不超过40字，精准总结下面内容的核心要点，禁止输出任何模板、分类名、无效内容、英文、拼音，只能输出一句有用的中文句子，结尾必须是句号：";
	let user_content = format!("{}\n\n{}", prompt, detail);
	let req_body = serde_json::json!({
		"model": "deepseek-v3.2",
		"messages": [
			{"role": "user", "content": user_content}
		],
		"stream": false,
		"max_tokens": 80
	});
	let client = reqwest::Client::new();
	let mut last_err = None;
	for attempt in 1..=50 {
		tracing::debug!("llm_summarize_30 attempt {}/50", attempt);
		let resp = match client
			.post(api_url)
			.bearer_auth(api_key)
			.json(&req_body)
			.send()
			.await {
			Ok(r) => r,
			Err(e) => {
				last_err = Some(e);
				tokio::time::sleep(std::time::Duration::from_secs(2)).await;
				continue;
			}
		};
		let text = match resp.text().await {
			Ok(t) => t,
			Err(e) => {
				last_err = Some(e);
				tokio::time::sleep(std::time::Duration::from_secs(2)).await;
				continue;
			}
		};
		// 检查是否为 LLM 超限或报错
		if text.contains("ModelAccountTpmRateLimitExceeded") || text.contains("error") {
			tracing::warn!("llm_summarize_30 LLM 超限或报错，重试");
			tokio::time::sleep(std::time::Duration::from_secs(2)).await;
			continue;
		}
		let result = serde_json::from_str::<serde_json::Value>(&text)
			.ok()
			.and_then(|v| {
				v.get("choices")
					.and_then(|choices| choices.get(0))
					.and_then(|c| c.get("message"))
					.and_then(|m| m.get("content"))
					.and_then(|c| c.as_str())
					.map(|s| s.trim().to_string())
			});
		if let Some(s) = result {
			return Ok(s);
		} else {
			tracing::warn!("llm_summarize_30 无法从 LLM JSON 提取 message.content，重试");
			tokio::time::sleep(std::time::Duration::from_secs(2)).await;
			continue;
		}
	}
	tracing::error!("llm_summarize_30 连续50次失败，跳过此次调用");
	if let Some(e) = last_err {
		Err(e)
	} else {
		panic!("llm_summarize_30 连续50次失败")
	}
}
use std::path::Path;
use tracing::{debug, error, info, warn};
use tracing_appender::rolling::RollingFileAppender;
use tracing_appender::non_blocking::NonBlocking;
use tracing_subscriber::prelude::*;


// 日志记录直接使用 tracing 宏（info, warn, error）


// 解析 YYYY-MM (或 YYYY-M) 字符串为 (year, month)

// 已移除：不再提取并写入单独的 module 文件夹，统一归类到固定分类文件

// 获取 commit 哈希值及提交时间（ISO8601）列表
// 返回指定仓库路径下所有提交的 (commit_hash, commit_date(带时区), subject) 元组列表。
// 使用 `git log --pretty=format:%H|%cI|%s` 命令获取。
fn get_commit_hashes_with_date(repo_path: &str) -> Vec<(String, DateTime<FixedOffset>, String)> {
	// 输出格式：<hash>|<commit-date ISO8601>|<subject>
	let output = Command::new("git")
		.arg("-C")
		.arg(repo_path)
		.arg("log")
		.arg("--pretty=format:%H|%cI|%s")
		.output()
		.expect("failed to execute git log");
	let stdout = String::from_utf8_lossy(&output.stdout);
	let mut res = Vec::new();
	for line in stdout.lines() {
		// split_once on first '|' to get hash, then split remaining on first '|' for date and subject
		if let Some((h, rest)) = line.split_once('|') {
			if let Some((d, s)) = rest.split_once('|') {
				if let Ok(dt) = DateTime::parse_from_rfc3339(d) {
					res.push((h.to_string(), dt, s.to_string()));
				} else {
					debug!("无法解析 commit 日期: {} (commit {})", d, h);
				}
			}
		}
	}
	res
}

// 获取指定 commit 的完整 diff 内容。
// 失败时会 panic，调用者需保证 hash 有效。
fn get_commit_diff(repo_path: &str, hash: &str) -> String {
	let output = Command::new("git")
		.arg("-C")
		.arg(repo_path)
		.arg("show")
		.arg(hash)
		.output()
		.expect("failed to execute git show");
	String::from_utf8_lossy(&output.stdout).to_string()
}

// 调用 LLM API 对提交（subject + diff）进行分析和分类，并打印调试信息。
// 返回模型生成的分析文本。
// 若返回 JSON 结构不符预期，则直接返回原始文本。
async fn analyze_with_llm(api_url: &str, api_key: &str, diff: &str) -> Result<String, reqwest::Error> {
	 let prompt = r#"请对以下提交（包含 commit subject 与 diff）做全方位、深入、细致的分析：

【多角度思考要求】
1. 不仅要结合 diff 内容本身，还要考虑：
	- 变更的上下游影响、历史背景、潜在风险、兼容性、性能、可维护性、可扩展性等多维度。
	- 结合行业最佳实践、常见风险点、边界场景，主动思考可能遗漏的影响。
	- 反思本次变更是否存在隐患、是否有更优实现、是否影响已有功能或接口。
2. 输出前请自查每一项内容，确保无模板化、无空洞、无泛泛而谈，所有结论均有具体细节和洞察。
3. 鼓励多思考几秒钟，反复推敲，力求输出高质量、有深度、有广度的分析。
4. 最后一点，一句话总结的内容必须是中文描述，禁止输出英文、拼音、无信息量的模板化内容！！！！！！！！！！
5. 如果一句话总结的内容还是英文、拼音、无信息量的模板化内容，说明分析不够深入，请重新分析并生成总结，直到满足要求为止。
6. 你可以思考几秒钟来生成更准确、更有洞察力的分析结果。
【输出要求】
(1) 用一句话总结本次提交的核心变更，必须结合diff内容进行智能归纳，不能只翻译subject，不能机械复述，不能出现“分类: xxx”“一句话总结: xxx”等模板内容，必须简洁、准确、有洞察力，信息量高，25字以上，35字以内。禁止输出“详细说明”“合并操作”“合并提交”“无实际变更”“无内容”“无效内容””英文标题”等无效模板，必须输出具体变更内容。
(2) 用一句话对本次提交进行唯一且准确的分类，分类必须严格基于diff内容，不可主观猜测，分类标准如下：
	- bug修复：修复功能、逻辑、崩溃、异常、数据错误等问题
	- 安全修复：修复安全漏洞、权限、注入、越权等
	- 功能增强：新增功能、接口、参数、配置、扩展等
	- 性能优化：提升速度、内存、并发、资源占用等
	- 文档变更：仅修改文档、注释、README等
	- 重构：重命名、结构调整、代码风格、格式化、无业务影响的重排
	- 测试：新增或调整测试用例、mock、CI脚本等
	- 构建/CI：构建脚本、CI/CD流程、依赖升级等
	- 配置变更：配置文件、环境变量、部署参数等
	- 兼容性：适配新平台、版本、API兼容等
	- 其他：无法归入以上类别的
分类必须唯一且准确，不能多选，不能模糊，不能主观猜测，必须结合diff内容。
(3) 用2-4行简要说明本次修改的目的、主要影响、关键点，避免空洞描述，鼓励结合diff细节说明影响范围、上下游风险、兼容性等；
(4) 列出可能受影响的模块或文件路径（如能推断）；
(5) 评估回归风险并给出建议（如是否需要回归测试、注意点等），并结合实际变更内容给出具体建议。

【输出格式严格要求】
- 表格中一句话总结的内容必须是中文描述，禁止输出英文、拼音、无信息量的模板化内容。
- 第一行：分类（如“bug修复”）
- 后续：分段详细说明、受影响模块、建议等，全部用中文。

【反例】
- “一句话总结：修复了一个bug” ❌（不能有“xxx总结”前缀，不能只翻译subject，不能空洞）
- “分类: 文档变更” ❌（不能有“分类:”等模板内容）
- “优化” ❌（过于宽泛、无信息量）
- “修复问题” ❌（无具体内容）
- “增加功能” ❌（无具体内容）
- “优化了代码” ❌（无具体内容）
- “一句话总结：优化了代码结构” ❌（有模板前缀）
- “修复bug” ❌（无具体内容）
- “修复了一个小问题” ❌（无具体内容）
- “优化性能” ❌（无具体内容）
- “调整代码结构” ❌（无具体内容）
- “详细说明” ❌（禁止出现）
- “合并操作” ❌（禁止出现）
- “合并提交” ❌（禁止出现）
- “无实际变更” ❌（禁止出现）
- “无内容” ❌（禁止出现）
- “无效内容” ❌（禁止出现）
- “修复了README中的错别字” ✅（直接描述核心变更）
- “优化了数据同步逻辑，提升性能” ✅
- “重构部分代码，提升可维护性” ✅
- “修复登录接口参数校验漏洞” ✅
- “新增API接口支持批量导入” ✅
- “调整配置文件格式，兼容旧版本” ✅
- “删除无用依赖，减小包体积” ✅
- “修复用户注册时邮箱校验逻辑” ✅
- “完善单元测试覆盖边界场景” ✅
- “修复内存泄漏，提升稳定性” ✅
- “调整日志输出，便于排查问题” ✅
- “优化缓存命中率，减少数据库压力” ✅
- “修复多线程下的竞态条件” ✅
- “修复安全漏洞，防止SQL注入” ✅

【边界说明】
- 分类必须唯一且准确，严格基于diff内容，不能主观猜测或多选。
- 一句话总结必须输出具体变更内容，禁止输出“详细说明”“合并操作”等无效模板。
- 总结必须体现diff的具体变化、影响面或修复点，不能只描述“修复bug”“优化代码”等宽泛内容。
- 遇到大批量格式化、重命名、注释调整等无业务影响的提交，可直接说明“批量格式化代码，无业务影响”或“调整注释，无功能变更”等。
- 若diff涉及多个模块或影响面较广，建议在总结中点明“涉及X模块”或“影响Y功能”等。
- 若diff内容极少或无实际变更，也需如实说明。

【自查与反思】
- 输出前请再次自查每一项内容，确保所有要求均已严格执行，所有结论均有充分细节和洞察。
- 禁止输出任何英文、拼音、无信息量、模板化内容。
- 若有任何不确定之处，宁可多思考几秒钟，力求输出最优分析。
- 如果一句话总结的内容还是英文、拼音、无信息量的模板化内容，说明分析不够深入，请重新分析并生成总结，直到满足要求为止。

表格内容必须全中文，禁止输出英文、拼音、无信息量的模板化内容！！！！！！！！！！
你可以多思考几秒钟来生成更准确、更有洞察力的分析结果。
"#;
	let client = reqwest::Client::new();
	let user_content = format!("{}\n
以下是 diff 内容：\n{}", prompt, diff);
	let req_body = serde_json::json!({
		"model": "deepseek-v3.2",
		"messages": [
			{"role": "user", "content": user_content}
		],
		"stream": false,
		"max_tokens": 200
	});
	debug!("向 LLM 发送请求到 {} (payload bytes: {})", api_url, user_content.len());
	let resp = client
		.post(api_url)
		.bearer_auth(api_key)
		.json(&req_body)
		.send()
		.await?;
	let status = resp.status();
	let text = resp.text().await?;
	// Do not print full response body to stdout. Log status and a short preview.
	info!("LLM 返回状态码: {}，响应大小: {} 字节", status, text.len());
	let preview: String = text.chars().take(200).collect();
	debug!("LLM 返回内容预览: {}", preview);
	// 尝试解析 json 并提取 message.content
	let result = serde_json::from_str::<serde_json::Value>(&text)
		.ok()
		.and_then(|v| {
			v.get("choices")
				.and_then(|choices| choices.get(0))
				.and_then(|c| c.get("message"))
				.and_then(|m| m.get("content"))
				.and_then(|c| c.as_str())
				.map(|s| s.trim().to_string())
		});
	if let Some(s) = result {
		debug!("从 LLM 响应中提取到了文本，长度={} 字符", s.len());
		Ok(s)
	} else {
		warn!("无法按预期从 LLM JSON 中提取 message.content，返回原始文本（长度={}）", text.len());
		Ok(text.trim().to_string())
	}
}

#[tokio::main]
async fn main() {
	// 初始化日志系统，默认只在终端显示 INFO 及以上级别（可通过 RUST_LOG=debug 开启 DEBUG）。
	// 日志同时写入 logs 目录下的滚动日志文件。
	let level = env::var("RUST_LOG").ok().and_then(|v| v.parse::<tracing::Level>().ok()).unwrap_or(tracing::Level::INFO);

	// 确保 logs 目录存在
	if let Err(e) = create_dir_all("logs") {
		error!("无法创建 logs 目录: {}", e);
	}

	// 创建每日滚动日志文件 logs/lensup.log
	let file_appender: RollingFileAppender = tracing_appender::rolling::daily("logs", "lensup.log");
	let (non_blocking, _guard): (NonBlocking, _) = tracing_appender::non_blocking(file_appender);

	// 日志分为文件和终端两层，文件无颜色，终端有颜色
	let file_layer = tracing_subscriber::fmt::layer()
		.with_writer(non_blocking)
		.with_ansi(false)
		.with_target(false)
		.with_level(true);

	let stdout_layer = tracing_subscriber::fmt::layer()
		.with_writer(std::io::stdout)
		.with_ansi(true)
		.with_level(true);

	tracing_subscriber::registry()
		.with(file_layer)
		.with(stdout_layer)
		.with(tracing::level_filters::LevelFilter::from_level(level))
		.init();
	// 建议使用 dotenv 加载 .env 文件
	dotenv::dotenv().ok();

	// 1. 获取 LLM API key、API url、repo 路径、commit 数量等参数
	let api_key = env::var("DEEPSEEK_API_KEY").expect("请设置 DEEPSEEK_API_KEY 环境变量");
	let api_url = env::var("LLM_BASE_URL").expect("请设置 LLM_BASE_URL 环境变量");

	let args: Vec<String> = env::args().collect();

	// 新方式：必须指定 lensup <repo_name> ...
	let repo_name = if args.len() > 1 && !args[1].starts_with("--") {
		args[1].clone()
	} else {
		eprintln!("用法: lensup <repo_name> --from YYYY-MM [--to YYYY-MM] ...");
		std::process::exit(1);
	};

	// 只用 .env 的 REPO_PATH 作为根目录
	let repo_root = env::var("REPO_PATH").expect("请设置 REPO_PATH 环境变量为仓库根目录");
	let repo_path = format!("{}/{}", repo_root.trim_end_matches('/'), repo_name);

	let repo_name = Path::new(&repo_path)
		.file_name()
		.and_then(|s| s.to_str())
		.unwrap_or("repo");

	// 参数解析起始下标（repo_name 占用 args[1]，从 2 开始）
	let mut i = 2;
	let mut commit_limit: usize = env::var("COMMIT_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
	let delay_months: i32 = env::var("ANALYSIS_DELAY_MONTHS").ok().and_then(|v| v.parse().ok()).unwrap_or(6);
	info!("配置: repo_path={}, commit_limit={}, delay_months={}, only_categorized(default)={}", repo_path, commit_limit, delay_months, true);

	// 可选参数：命令行 --from YYYY-MM 和 --to YYYY-MM，或回退到环境变量 ANALYSIS_FROM / ANALYSIS_TO
	let mut from_month: Option<(i32, u32)> = None;
	let mut to_month: Option<(i32, u32)> = None;
	while i < args.len() {
		match args[i].as_str() {
			"--from" => {
				if i + 1 < args.len() {
					from_month = parse_yyyy_mm(&args[i + 1]);
					if from_month.is_none() {
						eprintln!("参数 --from {} 解析失败，应为 YYYY-MM", args[i + 1]);
						return;
					}
					i += 1;
				}
			}
			"--to" => {
				if i + 1 < args.len() {
					to_month = parse_yyyy_mm(&args[i + 1]);
					if to_month.is_none() {
						eprintln!("参数 --to {} 解析失败，应为 YYYY-MM", args[i + 1]);
						return;
					}
					i += 1;
				}
			}
			"--only-categorized" => {
				// 已废弃参数，无需处理
			}
			"--root" => {
				// 已废弃参数，无需处理
			}
			"--limit" => {
				if i + 1 < args.len() {
					if let Ok(n) = args[i + 1].parse::<usize>() {
						commit_limit = n;
						i += 1;
					}
				}
			}
			_ => {}
		}
		i += 1;
	}
	if from_month.is_none() {
		if let Ok(s) = env::var("ANALYSIS_FROM") {
			from_month = parse_yyyy_mm(&s);
			if from_month.is_none() {
				eprintln!("环境变量 ANALYSIS_FROM={} 解析失败，应为 YYYY-MM", s);
				return;
			}
		}
	}
	if to_month.is_none() {
		if let Ok(s) = env::var("ANALYSIS_TO") {
			to_month = parse_yyyy_mm(&s);
			if to_month.is_none() {
				eprintln!("环境变量 ANALYSIS_TO={} 解析失败，应为 YYYY-MM", s);
				return;
			}
		}
	}

	debug!("日期范围: from={:?} to={:?}", from_month, to_month);

	// 如果用户只提供了一个边界（如 --from 2026-03 但没有 --to），则视为单月分析（from == to）
	if from_month.is_some() && to_month.is_none() {
		to_month = from_month;
	}
	if to_month.is_some() && from_month.is_none() {
		from_month = to_month;
	}

	// 区间反向时自动交换
	if let (Some((from_y, from_m)), Some((to_y, to_m))) = (from_month, to_month) {
		let from_idx = month_index(from_y, from_m);
		let to_idx = month_index(to_y, to_m);
		if from_idx > to_idx {
			warn!("区间参数 from > to，自动交换顺序: {}-{} <-> {}-{}", from_y, from_m, to_y, to_m);
			from_month = Some((to_y, to_m));
			to_month = Some((from_y, from_m));
		}
	}

	// 允许 ANALYSIS_LIMIT 环境变量覆盖 commit_limit
	if let Ok(v) = env::var("ANALYSIS_LIMIT") {
		if let Ok(n) = v.parse::<usize>() {
			commit_limit = n;
		}
	}

	// 确保 reports 目录存在
	if let Err(e) = create_dir_all("reports") {
		error!("无法创建 reports 目录: {}", e);
		return;
	} else {
		debug!("确保存在 reports/ 目录");
	}

	// 2. 获取 commit 列表前先 git pull 保证分析最新提交
	info!("在 {} 上执行 git pull 更新仓库", repo_path);
	match Command::new("git").arg("-C").arg(&repo_path).arg("pull").output() {
		Ok(out) => {
			if out.status.success() {
				info!("git pull 完成，输出大小 {} 字节", out.stdout.len());
			} else {
				error!("git pull 返回非零状态: {}", String::from_utf8_lossy(&out.stderr));
			}
		}
		Err(e) => {
			error!("执行 git pull 失败: {}", e);
		}
	}

	// 获取 commit 列表（含时间）
	let commits = get_commit_hashes_with_date(&repo_path);
	if commits.is_empty() {
		warn!("仓库无可用提交，直接退出");
		println!("仓库无可用提交，直接退出");
		return;
	}
	info!("从仓库获取到 {} 条提交（raw）", commits.len());



	// 新规则：只生成一个md文档，所有模块分析写入该文件
	if let (Some((from_y, from_m)), Some((to_y, to_m))) = (from_month, to_month) {
		let from_str = format!("{:04}-{:02}", from_y, from_m);
		let to_str = format!("{:04}-{:02}", to_y, to_m);
		let file_name = if from_str == to_str {
			format!("{}-{}.md", repo_name, from_str)
		} else {
			format!("{}-{}-{}.md", repo_name, from_str, to_str)
		};
		let file_path = format!("reports/{}", file_name);
		let from_idx = month_index(from_y, from_m);
		let to_idx = month_index(to_y, to_m);
		let mut entries: Vec<(String, DateTime<FixedOffset>, String)> = commits.into_iter().filter(|(_h, dt, _s)| {
			let idx = month_index(dt.year(), dt.month());
			idx >= from_idx && idx <= to_idx
		}).collect();
		let total_in_range = entries.len();
		info!("区间 {}-{} 至 {}-{} 内剩余 commit 数量: {}", from_y, from_m, to_y, to_m, total_in_range);
		if entries.is_empty() {
			warn!("区间内无符合条件的提交，直接退出");
			println!("区间内无符合条件的提交，直接退出");
			return;
		}
		if commit_limit > 0 && entries.len() > commit_limit {
			entries.truncate(commit_limit);
		}

		   use std::collections::BTreeMap;
		   let mut type_map: BTreeMap<String, Vec<(String, String, String, String)>> = BTreeMap::new(); // 分类 -> [(hash, summary, impact, suggestion)]
		   let mut detail_map: BTreeMap<String, Vec<String>> = BTreeMap::new(); // 分类 -> [详细分析]
		   let total_to_analyze = entries.len();
		   for (idx, (h, dt, subject)) in entries.iter().enumerate() {
			   let diff = get_commit_diff(&repo_path, h);
			   let header = format!("Commit subject: {}\nCommit date: {}\n\n", subject, dt.to_rfc3339());
			   let combined = format!("{}{}", header, diff);
			   let short_hash = h.chars().take(8).collect::<String>();
			   let entry_title = format!("**Commit `{}`**  \n**长哈希：{}**  \n**提交时间：{}**  \n**提交标题：{}**  \n\n", short_hash, h, dt.format("%Y-%m-%d %H:%M:%S %:z"), subject);
			   // 检查diff是否无代码修改（只包含diff --git、index、---、+++、@@等元信息，无+/-代码行）
			   let is_no_code_change = diff.lines().all(|l| {
				   let l = l.trim_start();
				   l.is_empty() ||
				   l.starts_with("diff --git") ||
				   l.starts_with("index ") ||
				   l.starts_with("--- ") ||
				   l.starts_with("+++ ") ||
				   l.starts_with("@@") ||
				   l.starts_with("commit ") ||
				   l.starts_with("Author:") ||
				   l.starts_with("Date:")
			   }) || !diff.lines().any(|l| {
				   let l = l.trim_start();
				   (l.starts_with('+') || l.starts_with('-')) && !l.starts_with("+++ ") && !l.starts_with("--- ")
			   });
			   match analyze_with_llm(&api_url, &api_key, &combined).await {
				   Ok(analysis) => {
					   let category = parse_category_from_analysis(&analysis).unwrap_or_else(|| classify_to_category(&analysis));
					   let mut summary = String::new();
					   let mut impact = String::new();
					   let mut lines = analysis.lines();
					   // 跳过首行分类
					   let _ = lines.next();
					   if is_no_code_change {
						   // 用详细分析内容的第一行（20字内，完整）作为summary
						   // 跳过首行分类，lines已在上面next一次
						   let mut found = None;
						   for line in lines.by_ref() {
							   let l = line.trim();
							   if !l.is_empty() {
								   // 取第一行非空内容，截取20字（不截断汉字）
								   let mut chars = l.chars();
								   let mut s = String::new();
								   for _ in 0..20 {
									   if let Some(c) = chars.next() {
										   s.push(c);
									   } else {
										   break;
									   }
								   }
								   found = Some(s);
								   break;
							   }
						   }
						   summary = found.unwrap_or_else(|| "不涉及核心代码修改".to_string());
					   } else if let Some(line) = lines.next() {
						   summary = line.trim().to_string();
					   }
					   if let Some(line) = lines.next() {
						   impact = line.trim().to_string();
					   }
					   // 智能提取建议段落：优先匹配“建议：”或“【建议】”开头的行，否则兜底“人工复核”
					   let mut suggestion = None;
					   for line in analysis.lines() {
						   let l = line.trim();
						   if l.starts_with("建议：") || l.starts_with("建议:") {
							   let s = l.trim_start_matches("建议：").trim_start_matches("建议:").trim();
							   if !s.is_empty() {
								   suggestion = Some(s.to_string());
								   break;
							   }
						   } else if l.starts_with("【建议】") {
							   let s = l.trim_start_matches("【建议】").trim();
							   if !s.is_empty() {
								   suggestion = Some(s.to_string());
								   break;
							   }
						   }
					   }
					   // 兼容原有等级关键词
					   if suggestion.is_none() {
						   let mut found = None;
						   let mut found_level = 10;
						   for line in analysis.lines() {
							   let l = line.trim();
							   if l.contains("立刻合入") && found_level > 1 {
								   found = Some("立刻合入".to_string());
								   found_level = 1;
							   }
							   if l.contains("建议合入") && found_level > 2 {
								   found = Some("建议合入".to_string());
								   found_level = 2;
							   }
							   if l.contains("人工复核") && found_level > 3 {
								   found = Some("人工复核".to_string());
								   found_level = 3;
							   }
							   if l.contains("不影响") && found_level > 4 {
								   found = Some("不影响".to_string());
								   found_level = 4;
							   }
						   }
						   suggestion = found;
					   }
					   // bug修复强制“立刻合入”；安全修复“建议合入”；功能增强类：翻译/版本更新建议合入，无代码变更建议“不影响”，其余由LLM分析内容判断
					   let suggestion = match &*category {
						   "bug修复" => "立刻合入".to_string(),
						   "安全修复" => "建议合入".to_string(),
						   "功能增强" => {
							   // 判断是否为翻译或版本更新
							   let lower_subject = subject.to_lowercase();
							   if lower_subject.contains("翻译") || lower_subject.contains("translation") || lower_subject.contains("manpage") || lower_subject.contains("版本") || lower_subject.contains("release") || lower_subject.contains("ver") {
								   "建议合入".to_string()
							   } else if is_no_code_change {
								   "不影响".to_string()  
							   } else {
								   // LLM分析内容中优先级：建议合入 > 人工复核
								   let mut found = None;
								   let mut found_level = 10;
								   for line in analysis.lines() {
									   let l = line.trim();
									   if l.contains("建议合入") && found_level > 1 {
										   found = Some("建议合入".to_string());
										   found_level = 1;
									   }
									   if l.contains("人工复核") && found_level > 2 {
										   found = Some("人工复核".to_string());
										   found_level = 2;
									   }
								   }
								   found.unwrap_or_else(|| "人工复核".to_string())
							   }
						   },
						   _ => suggestion.unwrap_or_else(|| "人工复核".to_string()),
					   };

					   // summary 为空时 fallback 用 impact
					   let summary_final = if summary.is_empty() { impact.clone() } else { summary.clone() };
					   type_map.entry(category.to_string()).or_default().push((short_hash.clone(), summary_final, impact.clone(), suggestion.clone()));
					   // 详细分析正文：不再输出 summary/一句话总结，只输出剩余内容，且用长哈希
					   let mut entry = String::new();
					   entry.push_str(&entry_title);
					   let mut lines = analysis.lines();
					   // 跳过首行分类和 summary
					   let _ = lines.next();
					   let _ = lines.next();
					   // 跳过第三行（原本 impact，防止 LLM 把一句话总结写到第三行）
					   let _ = lines.next();
					   let mut found_suggestion_in_body = false;
					   while let Some(line) = lines.next() {
						   let trimmed = line.trim_start();
						   if trimmed.contains("一句话总结") { continue; }
						   // 检查是否为建议段落
						   if trimmed.starts_with("建议：") || trimmed.starts_with("建议:") || trimmed.starts_with("【建议】") {
							   found_suggestion_in_body = true;
							   entry.push_str(line);
							   entry.push('\n');
							   continue;
						   }
						   if trimmed.starts_with('#') {
							   let title = trimmed.trim_start_matches('#').trim();
							   if !title.is_empty() {
								   entry.push_str(&format!("**{}**\n", title));
							   }
						   } else {
							   entry.push_str(line);
							   entry.push('\n');
						   }
					   }
					   // 如果正文没有建议段落，补上一行
					   if !found_suggestion_in_body {
						   entry.push_str(&format!("回归风险与建议：{}\n", suggestion));
					   }
					   entry.push_str("\n---\n\n");
					   detail_map.entry(category.to_string()).or_default().push(entry);
				   },
				   Err(e) => {
					   let mut entry = String::new();
					   entry.push_str(&entry_title);
					   entry.push_str(&format!("**分析失败：{}**\n\n---\n\n", e));
					   detail_map.entry("分析失败".to_string()).or_default().push(entry);
					   type_map.entry("分析失败".to_string()).or_default().push((short_hash.clone(), "分析失败".to_string(), String::new(), "人工复核".to_string()));
				   }
			   }
			   let left = total_to_analyze - idx - 1;
			   info!("本次分析后剩余 commit 数量: {}", left);
		   }


		   let mut report = String::new();
		   // 章节1：报告总结
		   report.push_str(&format!("# {} 提交分析报告\n\n", repo_name));
		   report.push_str("## 总结\n\n");
		   let total_commits = entries.len();
		   // 统计各分类数量、建议合入/个人复核数量
		   let mut category_count: BTreeMap<String, usize> = BTreeMap::new();
		   let mut suggest_merge = 0;
		   let mut instant_merge = 0;
		   for (cat, commits) in &type_map {
			   category_count.insert(cat.clone(), commits.len());
			   for (_hash, _summary, _impact, suggestion) in commits {
				   if suggestion == "建议合入" {
					   suggest_merge += 1;
					   // 安全修复也计入立刻合入
					   if cat == "安全修复" {
						   instant_merge += 1;
					   }
				   }
				   if suggestion == "立刻合入" {
					   instant_merge += 1;
				   }
			   }
		   }
		   if from_str == to_str {
			   report.push_str(&format!("本报告扫描了 {} 包 {} 月份的提交，共计 {} 个。\n", repo_name, from_str, total_commits));
		   } else {
			   report.push_str(&format!("本报告扫描了 {} 包 {} 至 {} 的提交，共计 {} 个。\n", repo_name, from_str, to_str, total_commits));
		   }
		   // 分类统计
		   report.push_str("\n各类型提交统计：\n");
		   for (cat, count) in &category_count {
			   report.push_str(&format!("- {}：{} 个\n", cat, count));
		   }
		   report.push_str(&format!("\n建议合入：{} 个\n立刻合入：{} 个\n\n", suggest_merge, instant_merge));
		   report.push_str("---\n\n");

		   // 章节2：分类汇总表格
		   report.push_str("## 分类汇总\n\n");
		   report.push_str("| 分类 | 短Hash | 一句话总结 | 建议 |\n|---|---|---|---|\n");

		   // 收集所有需要 LLM 总结的条目，异步批量处理
		   use std::future::Future;
		   use std::pin::Pin;
		   let mut summary_tasks: Vec<Pin<Box<dyn Future<Output = Result<String, reqwest::Error>>>>> = Vec::new();
		   let mut summary_keys = Vec::new();
		   for (cat, commits) in &type_map {
			   for (hash, _summary, _impact, suggestion) in commits {
				   let mut detail_text = None;
				   if let Some(entries) = detail_map.get(cat) {
					   if let Some(detail) = entries.iter().find(|e| e.contains(hash)) {
						   detail_text = Some(detail.replace("|", " "));
					   }
				   }
				   summary_keys.push((cat.clone(), hash.clone(), suggestion.clone()));
				   if let Some(detail) = detail_text {
					   summary_tasks.push(Box::pin(llm_summarize_30(&api_url, &api_key, detail)));
				   } else {
					   summary_tasks.push(Box::pin(ready(Ok::<_, reqwest::Error>("无有效总结。".to_string()))));
				   }
			   }
		   }
		   let summary_results = join_all(summary_tasks).await;
		   for ((cat, hash, suggestion), summary_res) in summary_keys.into_iter().zip(summary_results.into_iter()) {
			   let one_line = match summary_res {
				   Ok(s) => if s.is_empty() { "无有效总结。".to_string() } else { s.replace('|', " ") },
				   Err(_) => "LLM总结失败。".to_string(),
			   };
			   report.push_str(&format!("| {} | {} | {} | {} |\n", cat, hash, one_line, suggestion));
		   }
		   report.push_str("\n---\n\n");

		   // 章节3：详细分析（按分类，固定顺序，全部输出）
		   let category_order = [
			   "bug修复", "功能增强", "性能优化", "安全修复", "构建/CI", "配置变更", "兼容性", "文档变更", "重构", "测试", "其他", "分析失败"
		   ];
		   for cat in &category_order {
			   report.push_str(&format!("## {}\n\n", cat));
			   if let Some(entries) = detail_map.get(*cat) {
				   for entry in entries {
					   // 确保每个 entry 结尾有换行和分隔符，避免片段不完整
					   let trimmed = entry.trim_end();
					   report.push_str(trimmed);
					   if !trimmed.ends_with("---") {
						   report.push_str("\n---\n\n");
					   } else {
						   report.push_str("\n\n");
					   }
				   }
			   } else {
				   report.push_str("（本分类本月无相关提交）\n\n");
			   }
		   }

		   report.push_str("> 由 lensup 自动生成\n");
		   let bytes = report.as_bytes().len();
		   let mut f = OpenOptions::new().create(true).write(true).truncate(true).open(&file_path).expect("无法写入报告文件");
		   f.write_all(report.as_bytes()).expect("写入报告失败");
		   info!("已写入分析报告 {} ({} 字节)", file_path, bytes);
		   println!("已写入 {}", file_path);
		   return;
	}

	// 否则，按月分组（原有逻辑）
}
