use std::process::Command;
use std::env;
use std::fs::{create_dir_all, OpenOptions};
use std::io::{Write, Read};
use std::collections::HashSet;
use chrono::{DateTime, Datelike, FixedOffset};
use regex::Regex;
use std::path::Path;
use tracing::{debug, error, info, warn};
use tracing_appender::rolling::RollingFileAppender;
use tracing_appender::non_blocking::NonBlocking;
use tracing_subscriber::prelude::*;

// simple logging helpers
// use `tracing` macros (info, warn, error) for logging


// parse YYYY-MM (or YYYY-M) into (year, month)
/// Parse a string formatted as "YYYY-MM" or "YYYY-M" into (year, month).
/// Returns None if parsing fails or month is out of range.
fn parse_yyyy_mm(s: &str) -> Option<(i32, u32)> {
	let parts: Vec<&str> = s.split('-').collect();
	if parts.len() != 2 { return None; }
	if let (Ok(y), Ok(m)) = (parts[0].parse::<i32>(), parts[1].parse::<u32>()) {
		if m >= 1 && m <= 12 {
			return Some((y, m));
		}
	}
	None
}

/// Convert a year/month into a monotonically increasing month index.
/// Useful for comparing month ranges.
fn month_index(year: i32, month: u32) -> i32 {
	year * 12 + month as i32
}

// 将 LLM 的分析文本映射到五类之一：bug修复、功能增强、文档变更、重构、其他
/// Heuristic mapping from free-text LLM analysis to a fixed category.
/// This uses a set of keyword checks (Chinese + English) and returns
/// one of the fixed category strings used by the reports.
fn classify_to_category(analysis: &str) -> &'static str {
	let s = analysis.to_lowercase();
	// 优先匹配包含关键字的情况
	if s.contains("bug") || s.contains("修复") || s.contains("修补") {
		return "bug修复";
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
	// 其余归为其他
	"其他"
}

// 尝试从 LLM 的分析文本首行解析明确的分类声明（例如："分类：其他" 或单行的 "其他"）
/// Attempt to parse an explicit category declared by the LLM.
/// Many LLM prompts start with a single-line category (e.g. "分类：其他").
/// If found, return the normalized category string used by the app.
fn parse_category_from_analysis(analysis: &str) -> Option<&'static str> {
	if let Some(first_line) = analysis.lines().next() {
		let s = first_line.trim().to_lowercase();
		// 移除前缀如 "分类：" 或 "分类:" 或 "category:" 等
		let s = s.strip_prefix("分类：").or_else(|| s.strip_prefix("分类:")).unwrap_or(&s);
		let s = s.strip_prefix("category:").unwrap_or(s).trim();
		match s {
			"bug修复" | "bug" | "修复" | "修补" => return Some("bug修复"),
			"功能增强" | "功能" | "feature" => return Some("功能增强"),
			"性能优化" | "性能" | "优化" => return Some("性能优化"),
			"安全修复" | "安全" | "cve" | "vuln" => return Some("安全修复"),
			"构建" | "ci" | "构建/ci" | "build" => return Some("构建/CI"),
			"配置变更" | "配置" | "config" => return Some("配置变更"),
			"兼容性" | "兼容" | "compat" => return Some("兼容性"),
			"文档变更" | "文档" | "translation" | "翻译" => return Some("文档变更"),
			"重构" | "refactor" => return Some("重构"),
			"测试" => return Some("测试"),
			"其他" | "other" => return Some("其他"),
			_ => {}
		}
	}
	None
}

// (已移除) 不再提取并写入单独的 module 文件夹；改为把分析归类到固定的分类文件中

// 获取 commit 哈希值及提交时间（ISO8601）列表
/// Return a Vec of (commit_hash, commit_date (with offset), subject) for the
/// given repository path. Uses `git log --pretty=format:%H|%cI|%s`.
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

// 获取某个 commit 的 diff
/// Return the full `git show <hash>` output as a string. On failure this will
/// panic (like before) — callers expect a valid diff string for LLM analysis.
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

// 调用 LLM API 进行分析并分类，并打印调试信息
/// Call the configured LLM API to analyze a commit (subject + diff).
/// The function returns the textual analysis produced by the model. It logs a
/// short preview and the HTTP status. The full response is returned if the
/// JSON structure doesn't match the expected shape.
async fn analyze_with_llm(api_url: &str, api_key: &str, diff: &str) -> Result<String, reqwest::Error> {
	let prompt = "请对以下提交（包含 commit subject 与 diff）做详细分析：\n1) 用一句话给出分类（如：bug修复、功能增强、无影响的翻译、文档变更、重构、测试、其他）；\n2) 给出 2-4 行的简要说明，说明修改目的和主要影响；\n3) 列出可能受影响的模块或文件路径（如果能推断）；\n4) 评估回归风险并给出建议（如是否需要回归测试、注意点等）。\n请先输出分类（单行），随后用小标题和段落输出其他内容，使用中文。";
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
	// initialize tracing subscriber for logging output and show only INFO+ in the terminal
	// (DEBUG messages will be hidden). You can still enable DEBUG by setting RUST_LOG=debug
	// Allow RUST_LOG to control verbosity; default to INFO if unset.
	let level = env::var("RUST_LOG").ok().and_then(|v| v.parse::<tracing::Level>().ok()).unwrap_or(tracing::Level::INFO);

	// Ensure logs directory exists
	if let Err(e) = create_dir_all("logs") {
		error!("无法创建 logs 目录: {}", e);
	}

	// Create a rolling daily file appender under logs/
	// Filename: logs/pr-tools-YYYY-MM-DD.log (handled by tracing_appender)
	let file_appender: RollingFileAppender = tracing_appender::rolling::daily("logs", "pr-tools.log");
	let (non_blocking, _guard): (NonBlocking, _) = tracing_appender::non_blocking(file_appender);

	// Build two layers: one writing to file (no ANSI colors), another to stdout.
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

	// optional: CLI args --from YYYY-MM and --to YYYY-MM, or fall back to env ANALYSIS_FROM / ANALYSIS_TO
	let mut from_month: Option<(i32, u32)> = None;
	let mut to_month: Option<(i32, u32)> = None;
	// 默认只生成按月份归档的分类报告（不再生成根级 monthly 文件）。
	// 使用 --root 或 ANALYSIS_ONLY_CATEGORIZED=0 可以恢复生成根文件。
	let mut only_categorized = true;
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
				only_categorized = true;
			}
			"--root" => {
				// 显式要求生成根级月度文件（legacy 行为）
				only_categorized = false;
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

	// If user only provided one bound (e.g. --from 2026-03 but not --to),
	// treat it as a single-month selection (from == to). This avoids
	// interpreting --from as open-ended and accidentally including other months.
	if from_month.is_some() && to_month.is_none() {
		to_month = from_month;
	}
	if to_month.is_some() && from_month.is_none() {
		from_month = to_month;
	}

	// 区间反向自动交换
	if let (Some((from_y, from_m)), Some((to_y, to_m))) = (from_month, to_month) {
		let from_idx = month_index(from_y, from_m);
		let to_idx = month_index(to_y, to_m);
		if from_idx > to_idx {
			warn!("区间参数 from > to，自动交换顺序: {}-{} <-> {}-{}", from_y, from_m, to_y, to_m);
			from_month = Some((to_y, to_m));
			to_month = Some((from_y, from_m));
		}
	}

	// allow env var to control only-categorized. If set, it overrides default/CLI.
	if let Ok(v) = env::var("ANALYSIS_ONLY_CATEGORIZED") {
		let lv = v.to_lowercase();
		if lv == "1" || lv == "true" || lv == "yes" {
			only_categorized = true;
		} else if lv == "0" || lv == "false" || lv == "no" {
			only_categorized = false;
		}
	}

	// also allow ANALYSIS_LIMIT to override commit_limit
	if let Ok(v) = env::var("ANALYSIS_LIMIT") {
		if let Ok(n) = v.parse::<usize>() {
			commit_limit = n;
		}
	}

	// ensure reports dir
	if let Err(e) = create_dir_all("reports") {
		error!("无法创建 reports 目录: {}", e);
		return;
	} else {
		debug!("确保存在 reports/ 目录");
	}

	// 2. 在获取 commit 列表前先尝试 pull 最新代码（保证分析使用远端最新提交）
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


	let re = Regex::new(r"`([0-9a-fA-F]{7,40})`").unwrap();

	// 区间模式：from 和 to 都有，且不同
	if let (Some((from_y, from_m)), Some((to_y, to_m))) = (from_month, to_month) {
		let from_str = format!("{:04}-{:02}", from_y, from_m);
		let to_str = format!("{:04}-{:02}", to_y, to_m);
		let range_dir = format!("reports/{}-{}-{}", repo_name, from_str, to_str);
		let file_path = format!("{}/{}-{}.md", range_dir, from_str, to_str);
		// 过滤区间内的提交
		let from_idx = month_index(from_y, from_m);
		let to_idx = month_index(to_y, to_m);
		let mut entries: Vec<(String, DateTime<FixedOffset>, String)> = commits.into_iter().filter(|(_h, dt, _s)| {
			let idx = month_index(dt.year(), dt.month());
			idx >= from_idx && idx <= to_idx
		}).collect();
		// respect commit_limit: 0 means no limit
		if entries.is_empty() {
			warn!("区间内无符合条件的提交，直接退出");
			println!("区间内无符合条件的提交，直接退出");
			return;
		}
		if commit_limit > 0 && entries.len() > commit_limit {
			entries.truncate(commit_limit);
		}
		// 读取已存在哈希
		let mut existing_hashes: HashSet<String> = HashSet::new();
		if only_categorized {
			if let Ok(rd) = std::fs::read_dir(&range_dir) {
				for entry in rd.flatten() {
					if let Ok(ft) = entry.file_type() {
						if ft.is_file() {
							if let Ok(mut cf) = OpenOptions::new().read(true).open(entry.path()) {
								let mut ccontent = String::new();
								if cf.read_to_string(&mut ccontent).is_ok() {
									for cap in re.captures_iter(&ccontent) {
										if let Some(m) = cap.get(1) {
											existing_hashes.insert(m.as_str().to_string());
										}
									}
								}
							}
						}
					}
				}
				info!("已从分类文件中读取到 {} 个已存在哈希 (range={}-{})", existing_hashes.len(), from_str, to_str);
			}
		} else {
			if let Ok(mut f) = OpenOptions::new().read(true).open(&file_path) {
				let mut content = String::new();
				if f.read_to_string(&mut content).is_ok() {
					for cap in re.captures_iter(&content) {
						if let Some(m) = cap.get(1) {
							existing_hashes.insert(m.as_str().to_string());
						}
					}
				}
				info!("已从根区间文件读取到 {} 个已存在哈希 (file={})", existing_hashes.len(), file_path);
			}
		}

		let mut report = String::new();
		if existing_hashes.is_empty() {
			report.push_str(&format!("#{}-{} 提交分析\n\n", from_str, to_str));
		} else {
			if let Ok(mut f) = OpenOptions::new().read(true).open(&file_path) {
				let mut content = String::new();
				if f.read_to_string(&mut content).is_ok() {
					report.push_str(&content);
				}
			}
		}

		for (h, dt, subject) in entries {
			if existing_hashes.contains(&h) {
				info!("{} 已存在，跳过", h);
				continue;
			}
			info!("开始分析 commit {} (range={}-{})", h, from_str, to_str);
			let diff = get_commit_diff(&repo_path, &h);
			let header = format!("Commit subject: {}\nCommit date: {}\n\n", subject, dt.to_rfc3339());
			let combined = format!("{}{}", header, diff);
			match analyze_with_llm(&api_url, &api_key, &combined).await {
				Ok(analysis) => {
					info!("commit {} 分析报告首行： {}", h, analysis.lines().next().unwrap_or(""));
					report.push_str(&format!("\n## Commit `{}` - {} - {}\n\n{}\n\n", h, subject, dt.to_rfc3339(), analysis));
					let category = parse_category_from_analysis(&analysis).unwrap_or_else(|| classify_to_category(&analysis));
					create_dir_all(&range_dir).ok();
					let safe_category = category.replace('/', "__").replace('\\', "_").replace(' ', "_");
					let cat_file = format!("{}/{}.md", range_dir, safe_category);
					let mut cat_existing: HashSet<String> = HashSet::new();
					if let Ok(mut cf) = OpenOptions::new().read(true).open(&cat_file) {
						let mut ccontent = String::new();
						if cf.read_to_string(&mut ccontent).is_ok() {
							for cap in re.captures_iter(&ccontent) {
								if let Some(m) = cap.get(1) {
									cat_existing.insert(m.as_str().to_string());
								}
							}
						}
					}
					if !cat_existing.contains(&h) {
						let mut c_report = String::new();
						if cat_existing.is_empty() {
							c_report.push_str(&format!("#{}-{} 提交分析\n\n", from_str, to_str));
						} else {
							if let Ok(mut cf) = OpenOptions::new().read(true).open(&cat_file) {
								let mut ccontent = String::new();
								if cf.read_to_string(&mut ccontent).is_ok() {
									c_report.push_str(&ccontent);
								}
							}
						}
						c_report.push_str(&format!("\n## Commit `{}` - {} - {}\n\n{}\n\n", h, subject, dt.to_rfc3339(), analysis));
						if let Some(parent) = Path::new(&cat_file).parent() {
							create_dir_all(parent).ok();
						}
						let mut cfw = match OpenOptions::new().create(true).write(true).truncate(true).open(&cat_file) {
							Ok(f) => f,
							Err(e) => {
								error!("无法写入分类报告文件 {}: {}", cat_file, e);
								continue;
							}
						};
						let bytes = c_report.as_bytes().len();
						if let Err(e) = cfw.write_all(c_report.as_bytes()) {
							error!("写入分类报告失败 {}: {}", cat_file, e);
						} else {
							info!("已写入分类文件 {} ({} 字节)", cat_file, bytes);
						}
					}
					existing_hashes.insert(h.clone());
				},
				Err(e) => {
						error!("分析失败: {}", e);
					report.push_str(&format!("\n## Commit `{}` - {}\n\n分析失败: {}\n\n", h, subject, e));
					existing_hashes.insert(h.clone());
				}
			}
		}

		if !only_categorized {
			if report.contains("由 pr-tools 自动生成") == false {
				report.push_str("\n---\n\n> 由 pr-tools 自动生成\n");
			}

			let bytes = report.as_bytes().len();
			let mut f = OpenOptions::new().create(true).write(true).truncate(true).open(&file_path).expect("无法写入报告文件");
			f.write_all(report.as_bytes()).expect("写入报告失败");
			info!("已写入区间文件 {} ({} 字节)", file_path, bytes);
			println!("已写入 {}", file_path);
		} else {
			println!("只生成分类报告，跳过区间根文件 {}", file_path);
		}
		return;
	}

	// 否则，按月分组（原有逻辑）
}
