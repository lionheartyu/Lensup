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
	let prompt = "请对以下提交（包含 commit subject 与 diff）做详细分析：\n1) 先将 commit subject 翻译成中文，作为‘一句话总结’输出在第二行；\n2) 用一句话给出分类（如：bug修复、功能增强、无影响的翻译、文档变更、重构、测试、其他）；\n3) 给出 2-4 行的简要说明，说明修改目的和主要影响；\n4) 列出可能受影响的模块或文件路径（如果能推断）；\n5) 评估回归风险并给出建议（如是否需要回归测试、注意点等）。\n请先输出分类（单行），再输出 subject 中文翻译（单行），随后用小标题和段落输出其他内容，全部使用中文。";
	let client = reqwest::Client::new();
	let user_content = format!("{}\n
以下是 diff 内容：\n{}", prompt, diff);
	let req_body = serde_json::json!({
		"model": "deepseek-chat",
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
	let repo_path = env::var("REPO_PATH").expect("请设置 REPO_PATH 环境变量");
	// 获取库名（repo_path 最后一级目录名）
	let repo_name = Path::new(&repo_path)
		.file_name()
		.and_then(|s| s.to_str())
		.unwrap_or("repo");
	let mut commit_limit: usize = env::var("COMMIT_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(5);
	let delay_months: i32 = env::var("ANALYSIS_DELAY_MONTHS").ok().and_then(|v| v.parse().ok()).unwrap_or(6);
	info!("配置: repo_path={}, commit_limit={}, delay_months={}, only_categorized(default)={}", repo_path, commit_limit, delay_months, true);

	// 可选参数：命令行 --from YYYY-MM 和 --to YYYY-MM，或回退到环境变量 ANALYSIS_FROM / ANALYSIS_TO
	let mut from_month: Option<(i32, u32)> = None;
	let mut to_month: Option<(i32, u32)> = None;
	// 旧逻辑遗留变量，已不再使用，无需定义 only_categorized
	let args: Vec<String> = env::args().collect();
	let mut i = 1;
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
			   match analyze_with_llm(&api_url, &api_key, &combined).await {
				   Ok(analysis) => {
					   let category = parse_category_from_analysis(&analysis).unwrap_or_else(|| classify_to_category(&analysis));
					   let mut summary = String::new();
					   let mut impact = String::new();
					   let mut lines = analysis.lines();
					   // 跳过首行分类
					   let _ = lines.next();
					   if let Some(line) = lines.next() {
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
					   // bug修复和安全修复强制建议合入
					   let suggestion = match category {
						   "bug修复" | "安全修复" => "建议合入".to_string(),
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
		   for (cat, commits) in &type_map {
			   category_count.insert(cat.clone(), commits.len());
			   for (_hash, _summary, _impact, suggestion) in commits {
				   if suggestion == "建议合入" {
					   suggest_merge += 1;
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
		   report.push_str(&format!("\n建议合入：{} 个\n\n", suggest_merge));
		   report.push_str("---\n\n");

		   // 章节2：分类汇总表格
		   report.push_str("## 分类汇总\n\n");
		   report.push_str("| 分类 | 短Hash | 一句话总结 | 建议 |\n|---|---|---|---|\n");
		   for (cat, commits) in &type_map {
			   for (hash, summary, _impact, suggestion) in commits {
				   // summary 为空、为分类名或无效时 fallback 用 impact 或 commit subject
				   let mut one_line = summary.trim().to_string();
				   // 移除所有常见 LLM 前缀（如“**一句话总结**：”、“**分类**：”、“分类：”等）
				   let mut s = one_line.as_str();
				   let strip_prefixes = [
					   "**一句话总结**：", "**一句话总结**:", "**一句话总结**", "**一句话总结：**", "**一句话总结:**", "**一句话总结：", "**一句话总结:",
					   "一句话总结：", "一句话总结:", "一句话总结",
					   "**分类**：", "**分类**:", "**分类**", "分类：", "分类:", "分类"
				   ];
				   for prefix in strip_prefixes.iter() {
					   if let Some(rest) = s.strip_prefix(prefix) {
						   s = rest.trim_start_matches('：').trim_start_matches(':').trim();
					   }
				   }
				   one_line = s.to_string();
				   // 判断是否为分类名、空、“分类”、或常见无效模板内容
				   let category_names = [
					   "bug修复", "功能增强", "性能优化", "安全修复", "构建/CI", "配置变更", "兼容性", "文档变更", "重构", "测试", "其他",
					   "分类", "**分类**"
				   ];
				   // 常见无效/模板化描述
				   let invalid_summaries = [
					   "有一句话总结", "一句话总结", "重构的描述", "功能增强的描述", "bug修复的描述", "安全修复的描述", "性能优化的描述", "文档变更的描述", "测试的描述", "其他的描述", "无效内容", "暂无", "无"
				   ];
				   let cleaned = one_line.trim_matches('*').trim_matches('：').trim_matches(':').trim();
				   let is_invalid = one_line.is_empty()
					   || category_names.iter().any(|n| n == &one_line)
					   || cleaned == "分类"
					   || invalid_summaries.iter().any(|inv| cleaned == *inv);
				   if is_invalid {
					   one_line = _impact.trim().to_string();
				   }
				   // 如果 impact 也无效，再 fallback 用 subject
				   let cleaned2 = one_line.trim_matches('*').trim_matches('：').trim_matches(':').trim();
				   let is_still_invalid = one_line.is_empty()
					   || category_names.iter().any(|n| n == &one_line)
					   || cleaned2 == "分类"
					   || invalid_summaries.iter().any(|inv| cleaned2 == *inv);
				   if is_still_invalid {
					   let subject = entries.iter().find_map(|(h, _dt, subj)| {
						   if h.chars().take(8).collect::<String>() == *hash {
							   Some(subj)
						   } else {
							   None
						   }
					   })
					   .map(|s| s.trim().to_string())
					   .unwrap_or_else(|| "无".to_string());
					   one_line = subject;
				   }
				   if one_line.chars().count() > 40 {
					   one_line = one_line.chars().take(40).collect::<String>() + "...";
				   }
				   report.push_str(&format!("| {} | {} | {} | {} |\n", cat, hash, one_line.replace('|', " "), suggestion));
			   }
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
