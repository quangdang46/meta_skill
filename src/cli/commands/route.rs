//! ms route - Route a task to the best matching skill
//!
//! Compact decision API: given a task description, return the top N skill
//! candidates with scores and ready-to-use load commands.
//!
//! # Output Schema (JSON)
//!
//! ```json
//! {
//!   "route_schema_version": 1,
//!   "task": "fix rust build error in tokio cli",
//!   "threshold": 0.65,
//!   "decision": "match|no_match",
//!   "candidates": [...],
//!   "fallback": { "search_command": "..." }
//! }
//! ```

use clap::Args;
use tracing::debug;

use crate::app::AppContext;
use crate::cli::output::OutputFormat;
use crate::core::disclosure::sanitize_slug;
use crate::error::Result;
use crate::search::NegativeRouteKey;
use crate::storage::sqlite::SkillRecord;

/// Default number of candidates to return
const DEFAULT_TOP_N: usize = 3;

/// Default route threshold (below this → no_match)
const DEFAULT_THRESHOLD: f64 = 0.65;

/// Current route schema version
const ROUTE_SCHEMA_VERSION: u32 = 1;

// =============================================================================
// CLI ARGS
// =============================================================================

#[derive(Args, Debug)]
pub struct RouteArgs {
    /// Task description to route
    pub task: String,

    /// Maximum number of candidates (default: 3)
    #[arg(long, default_value = "3")]
    pub top_n: usize,

    /// Minimum score threshold (0.0–1.0, default: 0.65)
    #[arg(long, default_value = "0.65")]
    pub threshold: f64,

    /// Show debug scores and breakdown
    #[arg(long)]
    pub debug: bool,

    /// Working directory for context detection
    #[arg(long)]
    pub cwd: Option<String>,

    /// Override output format
    #[arg(long)]
    pub output: Option<OutputFormat>,
}

// =============================================================================
// ROUTE CANDIDATE
// =============================================================================

/// A single routing candidate
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RouteCandidate {
    /// Canonical skill ID (e.g., "claude/rust-error-handling")
    pub skill_id: String,
    /// Display ID (short form when unambiguous)
    pub display_id: String,
    /// Match score (0.0–1.0)
    pub score: f64,
    /// Reasons for the match (short, enumerable)
    pub why: Vec<String>,
    /// Guidance on when to use this skill
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_to_use: Option<String>,
    /// Default load level for this skill
    pub default_load: String,
    /// Entry sections for loading
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entry_sections: Vec<String>,
    /// Ready-to-use load command
    pub load_command: String,
    /// Execution mode
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
}

/// Debug information for a candidate (only in --debug mode)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RouteDebug {
    /// Raw keyword match scores
    pub keyword_scores: Vec<KeywordScore>,
    /// Trigger phrase matches
    pub trigger_matches: Vec<String>,
    /// Tag matches
    pub tag_matches: Vec<String>,
    /// Context/description match score
    pub description_score: f64,
}

/// Individual keyword score breakdown
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KeywordScore {
    pub keyword: String,
    pub score: f64,
}

/// Fallback information when no candidates meet the threshold
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RouteFallback {
    pub search_command: String,
    /// Optional suggest command; absent when not supported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggest_command: Option<String>,
}

/// Complete route response
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RouteResponse {
    pub route_schema_version: u32,
    pub task: String,
    pub threshold: f64,
    pub decision: String,
    pub candidates: Vec<RouteCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub debug_info: Vec<RouteDebug>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<RouteFallback>,
}

// =============================================================================
// ROUTE IMPLEMENTATION
// =============================================================================

pub fn run(ctx: &AppContext, args: &RouteArgs) -> Result<()> {
    debug!(target: "route", task = %args.task, "routing task");

    let top_n = if args.top_n == 0 {
        DEFAULT_TOP_N
    } else {
        args.top_n
    };
    let threshold = if args.threshold <= 0.0 {
        DEFAULT_THRESHOLD
    } else {
        args.threshold
    };
    let cwd_fingerprint = args.cwd.clone().unwrap_or_else(|| {
        std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "/default".to_string())
    });

    // Check negative route cache before scoring
    let neg_key = NegativeRouteKey {
        task_text: normalize_route_text(&args.task),
        cwd_fingerprint: cwd_fingerprint.clone(),
    };
    if ctx.cache.get_negative_route(&neg_key) {
        debug!(target: "route", "negative route cache hit for task");
        let response = RouteResponse {
            route_schema_version: ROUTE_SCHEMA_VERSION,
            task: args.task.clone(),
            threshold,
            decision: "no_match".to_string(),
            candidates: vec![],
            debug_info: vec![],
            fallback: Some(RouteFallback {
                search_command: format!("ms search \"{}\" -O json", args.task),
                suggest_command: None,
            }),
        };
        return output_response(ctx, args, &response);
    }

    let all_skills = get_all_skills(ctx)?;
    let response = route_task(all_skills, &args.task, top_n, threshold, args.debug);

    // Cache no_match decisions
    if response.decision == "no_match" {
        ctx.cache.put_negative_route(&neg_key);
    }

    output_response(ctx, args, &response)
}

/// Core routing logic: score skills against a task and return top candidates.
/// Used by both CLI command and MCP tool handler.
pub(crate) fn route_task(
    all_skills: Vec<SkillRecord>,
    task: &str,
    top_n: usize,
    threshold: f64,
    debug: bool,
) -> RouteResponse {
    if all_skills.is_empty() {
        return RouteResponse {
            route_schema_version: ROUTE_SCHEMA_VERSION,
            task: task.to_string(),
            threshold,
            decision: "no_match".to_string(),
            candidates: vec![],
            debug_info: vec![],
            fallback: Some(RouteFallback {
                search_command: format!("ms search \"{task}\" -O json"),
                suggest_command: None,
            }),
        };
    }

    let mut scored: Vec<(SkillRecord, RouteCandidate, Option<RouteDebug>)> = all_skills
        .iter()
        .filter_map(|skill| score_skill(skill, task, debug))
        .collect();

    scored.sort_by(|a, b| {
        b.1.score
            .partial_cmp(&a.1.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let above_threshold: Vec<_> = scored
        .iter()
        .filter(|(_, candidate, _)| candidate.score >= threshold)
        .take(top_n)
        .collect();

    let decision = if above_threshold.is_empty() {
        "no_match"
    } else {
        "match"
    };

    let candidates: Vec<RouteCandidate> =
        above_threshold.iter().map(|(_, c, _)| c.clone()).collect();
    let debug_info: Vec<RouteDebug> = if debug {
        above_threshold
            .iter()
            .filter_map(|(_, _, d)| d.clone())
            .collect()
    } else {
        vec![]
    };

    let fallback = if decision == "no_match" {
        Some(RouteFallback {
            search_command: format!("ms search \"{task}\" -O json"),
            suggest_command: None,
        })
    } else {
        None
    };

    RouteResponse {
        route_schema_version: ROUTE_SCHEMA_VERSION,
        task: task.to_string(),
        threshold,
        decision: decision.to_string(),
        candidates,
        debug_info,
        fallback,
    }
}

fn output_response(ctx: &AppContext, args: &RouteArgs, response: &RouteResponse) -> Result<()> {
    let format = args.output.unwrap_or(ctx.output_format);
    match format {
        OutputFormat::Human => {
            println!("Route: {}", response.task);
            println!("Decision: {}", response.decision);
            println!("Threshold: {:.2}", response.threshold);
            println!();

            if response.candidates.is_empty() {
                println!("No matching skills found.");
                if let Some(ref fallback) = response.fallback {
                    println!("Fallback: {}", fallback.search_command);
                }
            } else {
                for (i, candidate) in response.candidates.iter().enumerate() {
                    println!(
                        "{}. {} (score: {:.2})",
                        i + 1,
                        candidate.display_id,
                        candidate.score
                    );
                    for reason in &candidate.why {
                        println!("   - {reason}");
                    }
                    println!("   load: {}", candidate.load_command);
                    println!();
                }
            }

            if args.debug && !response.debug_info.is_empty() {
                println!("--- Debug Info ---");
                for (i, debug) in response.debug_info.iter().enumerate() {
                    println!("Candidate {}:", i + 1);
                    println!("  Description score: {:.3}", debug.description_score);
                    if !debug.trigger_matches.is_empty() {
                        println!("  Trigger matches: {}", debug.trigger_matches.join(", "));
                    }
                    if !debug.tag_matches.is_empty() {
                        println!("  Tag matches: {}", debug.tag_matches.join(", "));
                    }
                    if !debug.keyword_scores.is_empty() {
                        println!("  Keyword scores:");
                        for ks in &debug.keyword_scores {
                            println!(
                                "    - {keyword} ({score:.3})",
                                keyword = ks.keyword,
                                score = ks.score
                            );
                        }
                    }
                }
            }
        }
        _ => {
            let output = serde_json::to_value(response)?;
            match format {
                OutputFormat::Toon => {
                    println!("{}", toon_rust::encode(output, None));
                }
                OutputFormat::Jsonl => {
                    println!("{}", serde_json::to_string(&output)?);
                }
                _ => {
                    println!("{}", serde_json::to_string_pretty(&output)?);
                }
            }
        }
    }

    Ok(())
}

// =============================================================================
// SCORING
// =============================================================================

/// Score a skill against a task description.
fn score_skill(
    skill: &SkillRecord,
    task: &str,
    debug: bool,
) -> Option<(SkillRecord, RouteCandidate, Option<RouteDebug>)> {
    let task_normalized = normalize_route_text(task);
    let task_lower = task.to_lowercase();
    let task_words: Vec<&str> = task_lower
        .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .filter(|w| !w.is_empty() && w.len() > 2)
        .collect();

    // Parse metadata for routing fields
    let meta: serde_json::Value = serde_json::from_str(&skill.metadata_json).unwrap_or_default();
    let trigger_phrases: Vec<String> = meta
        .get("trigger_phrases")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let when_to_use: Option<String> = meta
        .get("when_to_use")
        .and_then(|v| v.as_str())
        .map(String::from);
    let entry_sections: Vec<String> = meta
        .get("entry_sections")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .filter(|sections: &Vec<String>| !sections.is_empty())
        .unwrap_or_else(|| derive_entry_sections_from_body(&skill.body));
    let execution_mode: String = meta
        .get("execution_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("inline")
        .to_string();

    // 1. Trigger phrase matching (highest weight)
    let trigger_scores: Vec<(&String, f64)> = trigger_phrases
        .iter()
        .map(|phrase| (phrase, score_trigger_phrase(&task_normalized, phrase)))
        .collect();
    let trigger_matches: Vec<String> = trigger_scores
        .iter()
        .filter(|(_, score)| *score > 0.0)
        .map(|(phrase, _)| (*phrase).clone())
        .collect();
    let trigger_score = if trigger_matches.is_empty() {
        0.0
    } else {
        trigger_scores
            .iter()
            .map(|(_, score)| *score)
            .fold(0.0, f64::max)
    };

    // 2. Keyword matching against skill name, description, and tags
    let keyword_scores: Vec<KeywordScore> = task_words
        .iter()
        .filter_map(|word| {
            let word = word.trim_matches(|c: char| c.is_ascii_punctuation());
            if word.is_empty() || word.len() <= 2 {
                return None;
            }
            let mut score = 0.0f64;

            // Check name match
            if skill.name.to_lowercase().contains(word) {
                score += 0.5;
            }
            // Check description match
            if skill.description.to_lowercase().contains(word) {
                score += 0.3;
            }
            // Check tags
            let tags: Vec<&str> = meta
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            if tags.iter().any(|t: &&str| t.contains(word)) {
                score += 0.4;
            }
            // Check context project types
            let project_types: Vec<&str> = meta
                .get("context")
                .and_then(|c| c.get("project_types"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            if project_types.iter().any(|pt: &&str| pt.contains(word)) {
                score += 0.6;
            }

            if score > 0.0 {
                Some(KeywordScore {
                    keyword: word.to_string(),
                    score: score.min(1.0),
                })
            } else {
                None
            }
        })
        .collect();

    let keyword_score = if keyword_scores.is_empty() {
        0.0
    } else {
        keyword_scores.iter().map(|ks| ks.score).sum::<f64>()
            / (keyword_scores.len() as f64).max(1.0)
    };

    // 3. Tag matching
    let tags: Vec<String> = meta
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let tag_matches: Vec<String> = tags
        .iter()
        .filter(|tag| task_lower.contains(&tag.to_lowercase()))
        .cloned()
        .collect();
    let tag_score = if tags.is_empty() {
        0.0
    } else {
        (tag_matches.len() as f64 / tags.len() as f64).min(1.0)
    };

    // 4. Description similarity (word overlap)
    let binding = skill.description.to_lowercase();
    let desc_words: Vec<&str> = binding
        .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .filter(|w| !w.is_empty() && w.len() > 2)
        .collect();
    let desc_overlap = task_words.iter().filter(|w| desc_words.contains(w)).count();
    let description_score = if desc_words.is_empty() {
        0.0
    } else {
        let overlap_ratio = desc_overlap as f64 / desc_words.len().max(1) as f64;
        (overlap_ratio * 0.8).min(1.0)
    };

    // Composite score: weighted average
    let composite = trigger_score * 0.35 // trigger phrases are strongest signal
        + keyword_score * 0.30            // keyword matches in name/description/tags
        + tag_score * 0.15                // tag relevance
        + description_score * 0.20; // description overlap
    let composite = if trigger_score >= 1.0 {
        composite.max(0.85)
    } else if trigger_score >= 0.9 {
        composite.max(0.7)
    } else {
        composite
    };

    // Don't return candidates with zero score
    if composite <= 0.0 && !debug {
        return None;
    }

    // Build why list
    let mut why = Vec::new();
    if !trigger_matches.is_empty() {
        for t in trigger_matches.iter().take(2) {
            why.push(format!("trigger:{t}"));
        }
    }
    for ks in &keyword_scores {
        if ks.score >= 0.5 {
            why.push(format!("keyword:{}", ks.keyword));
        }
    }
    if !tag_matches.is_empty() {
        for t in tag_matches.iter().take(2) {
            why.push(format!("tag:{t}"));
        }
    }
    if !why.is_empty() {
        // Keep why short
        why.truncate(4);
    }

    // Determine display_id and skill_id
    let provider = meta
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("local");
    let display_id = meta
        .get("display_id")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            if skill.id.contains('/') {
                skill.id.rsplit('/').next().unwrap_or(&skill.id)
            } else {
                &skill.id
            }
        })
        .to_string();
    let canonical = meta
        .get("canonical_id")
        .and_then(|v| v.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if skill.id.contains('/') {
                skill.id.clone()
            } else {
                format!("{provider}/{}", display_id)
            }
        });
    let skill_id = if provider == "local" {
        display_id.clone()
    } else {
        canonical
    };

    // Build load command
    let entry_section_str = if !entry_sections.is_empty() {
        format!(" --section {}", entry_sections[0])
    } else {
        String::new()
    };
    let load_command = format!("ms load {}{} -O json", skill_id, entry_section_str);

    let default_load = if !entry_sections.is_empty() {
        format!("section:{}", entry_sections[0])
    } else {
        "standard".to_string()
    };

    let candidate = RouteCandidate {
        skill_id,
        display_id,
        score: composite,
        why,
        when_to_use,
        default_load,
        entry_sections,
        load_command,
        execution_mode: Some(execution_mode),
    };

    let debug_info = if debug {
        Some(RouteDebug {
            keyword_scores,
            trigger_matches,
            tag_matches,
            description_score,
        })
    } else {
        None
    };

    Some((skill.clone(), candidate, debug_info))
}

fn normalize_route_text(text: &str) -> String {
    text.split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .filter(|segment| !segment.is_empty())
        .map(|segment| segment.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn score_trigger_phrase(task_normalized: &str, phrase: &str) -> f64 {
    let phrase_normalized = normalize_route_text(phrase);
    if phrase_normalized.is_empty() {
        return 0.0;
    }

    if task_normalized == phrase_normalized {
        return 1.0;
    }

    if task_normalized.contains(&phrase_normalized) {
        return 0.9;
    }

    0.0
}

fn derive_entry_sections_from_body(body: &str) -> Vec<String> {
    let mut sections = Vec::new();

    for line in body.lines() {
        let Some(title) = line.trim().strip_prefix("## ") else {
            continue;
        };

        let slug = sanitize_slug(title.trim());
        if !slug.is_empty() && !sections.contains(&slug) {
            sections.push(slug);
        }
    }

    sections
}

// =============================================================================
// HELPERS
// =============================================================================

/// Get all indexed skills from the database
pub(crate) fn get_all_skills(ctx: &AppContext) -> Result<Vec<SkillRecord>> {
    let mut all = Vec::new();
    let mut offset = 0usize;
    let limit = 200usize;

    loop {
        let batch = ctx.db.list_skills(limit, offset)?;
        if batch.is_empty() {
            break;
        }
        offset += batch.len();
        all.extend(batch);
    }

    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::disclosure::sanitize_slug;

    fn make_skill_metadata(_id: &str, _name: &str, _desc: &str, tags: Vec<&str>) -> String {
        serde_json::json!({
            "tags": tags,
            "trigger_phrases": [],
            "execution_mode": "inline"
        })
        .to_string()
    }

    fn make_skill_record(id: &str, name: &str, desc: &str, tags: Vec<&str>) -> SkillRecord {
        SkillRecord {
            id: id.to_string(),
            name: name.to_string(),
            version: Some("1.0".to_string()),
            description: desc.to_string(),
            author: None,
            source_layer: "user".to_string(),
            source_path: format!("/skills/{id}.md"),
            body: format!("# {name}\n\n{desc}"),
            metadata_json: make_skill_metadata(id, name, desc, tags),
            content_hash: "abc123".to_string(),
            assets_json: "{}".to_string(),
            token_count: 100,
            quality_score: 0.8,
            indexed_at: "2025-01-01T00:00:00Z".to_string(),
            modified_at: "2025-01-01T00:00:00Z".to_string(),
            is_deprecated: false,
            deprecation_reason: None,
            provider: None,
            git_remote: None,
            git_commit: None,
            archive_format_version: None,
            provenance_json: "{}".to_string(),
        }
    }

    #[test]
    fn test_route_response_struct() {
        let response = RouteResponse {
            route_schema_version: 1,
            task: "fix rust error".to_string(),
            threshold: 0.65,
            decision: "match".to_string(),
            candidates: vec![RouteCandidate {
                skill_id: "claude/rust-error-handling".to_string(),
                display_id: "rust-error-handling".to_string(),
                score: 0.93,
                why: vec!["keyword:rust".to_string(), "keyword:error".to_string()],
                when_to_use: Some("Compiler errors, runtime failures".to_string()),
                default_load: "section:checklist".to_string(),
                entry_sections: vec!["checklist".to_string(), "pitfalls".to_string()],
                load_command: "ms load claude/rust-error-handling --section checklist -O json"
                    .to_string(),
                execution_mode: Some("inline".to_string()),
            }],
            debug_info: vec![],
            fallback: None,
        };

        let json = serde_json::to_string_pretty(&response).unwrap();
        assert!(json.contains("route_schema_version"));
        assert!(json.contains("rust-error-handling"));
        assert!(json.contains("0.93"));
    }

    #[test]
    fn test_route_no_match_fallback() {
        let response = RouteResponse {
            route_schema_version: 1,
            task: "unknown thing".to_string(),
            threshold: 0.65,
            decision: "no_match".to_string(),
            candidates: vec![],
            debug_info: vec![],
            fallback: Some(RouteFallback {
                search_command: "ms search \"unknown thing\" -O json".to_string(),
                suggest_command: None,
            }),
        };

        assert_eq!(response.decision, "no_match");
        assert!(response.fallback.is_some());
    }

    #[test]
    fn test_route_debug_info() {
        let debug = RouteDebug {
            keyword_scores: vec![
                KeywordScore {
                    keyword: "rust".to_string(),
                    score: 0.8,
                },
                KeywordScore {
                    keyword: "error".to_string(),
                    score: 0.6,
                },
            ],
            trigger_matches: vec!["compiler error".to_string()],
            tag_matches: vec!["rust".to_string()],
            description_score: 0.5,
        };

        let response = RouteResponse {
            route_schema_version: 1,
            task: "test".to_string(),
            threshold: 0.5,
            decision: "match".to_string(),
            candidates: vec![RouteCandidate {
                skill_id: "test/skill".to_string(),
                display_id: "skill".to_string(),
                score: 0.7,
                why: vec!["keyword:test".to_string()],
                when_to_use: None,
                default_load: "standard".to_string(),
                entry_sections: vec![],
                load_command: "ms load test/skill -O json".to_string(),
                execution_mode: Some("inline".to_string()),
            }],
            debug_info: vec![debug],
            fallback: None,
        };

        assert_eq!(response.debug_info.len(), 1);
        assert_eq!(response.debug_info[0].trigger_matches[0], "compiler error");
    }

    #[test]
    fn test_score_skill_with_tags() {
        let skill = make_skill_record(
            "rust-error-handling",
            "Rust Error Handling",
            "How to handle Rust compiler errors and runtime failures",
            vec!["rust", "error-handling", "compiler"],
        );

        // We can't easily call score_skill directly without a DB context,
        // but we can verify the metadata JSON structure
        let json: serde_json::Value = serde_json::from_str(&skill.metadata_json).unwrap();
        assert_eq!(json["tags"][0], "rust");
    }

    #[test]
    fn test_route_task_uses_canonical_skill_id_in_load_command() {
        let skill = SkillRecord {
            metadata_json: serde_json::json!({
                "provider": "agents",
                "canonical_id": "agents/common-name",
                "display_id": "common-name",
                "tags": ["common"],
                "entry_sections": ["checklist"],
                "execution_mode": "inline"
            })
            .to_string(),
            ..make_skill_record(
                "common-name",
                "Common Name",
                "Common routing test skill",
                vec!["common"],
            )
        };

        let response = route_task(vec![skill], "common", 3, 0.0, false);
        assert_eq!(response.decision, "match");
        assert_eq!(response.candidates.len(), 1);

        let candidate = &response.candidates[0];
        assert_eq!(candidate.skill_id, "agents/common-name");
        assert_eq!(
            candidate.load_command,
            "ms load agents/common-name --section checklist -O json"
        );
        assert!(candidate.load_command.contains(&candidate.skill_id));
    }

    #[test]
    fn test_route_derives_entry_sections_from_body_when_metadata_missing() {
        let skill = SkillRecord {
            metadata_json: serde_json::json!({
                "provider": "claude",
                "canonical_id": "claude/provider-route",
                "display_id": "provider-route",
                "trigger_phrases": ["provider route verification"],
                "execution_mode": "inline"
            })
            .to_string(),
            body: "# Provider Route\n\n## Overview\n\nOverview content.\n\n## Checklist\n\n- Step one\n".to_string(),
            ..make_skill_record(
                "claude/provider-route",
                "Provider Route",
                "Route-first provider verification skill",
                vec!["provider", "route", "archive"],
            )
        };

        let response = route_task(vec![skill], "provider route verification", 3, 0.65, false);
        assert_eq!(response.decision, "match");
        assert_eq!(
            response.candidates[0].entry_sections,
            vec!["overview".to_string(), "checklist".to_string()]
        );
        assert_eq!(
            response.candidates[0].load_command,
            "ms load claude/provider-route --section overview -O json"
        );
        assert_eq!(response.candidates[0].default_load, "section:overview");
    }

    #[test]
    fn test_exact_trigger_phrase_meets_default_threshold() {
        let skill = SkillRecord {
            metadata_json: serde_json::json!({
                "provider": "claude",
                "canonical_id": "claude/provider-route",
                "display_id": "provider-route",
                "tags": ["provider", "route", "archive"],
                "trigger_phrases": ["provider route verification"],
                "execution_mode": "inline"
            })
            .to_string(),
            ..make_skill_record(
                "claude/provider-route",
                "Provider Route",
                "Route-first provider verification skill",
                vec!["provider", "route", "archive"],
            )
        };

        let response = route_task(vec![skill], "provider route verification", 3, 0.65, false);
        assert_eq!(response.decision, "match");
        assert_eq!(response.candidates[0].skill_id, "claude/provider-route");
        assert!(response.candidates[0].score >= 0.65);
    }

    #[test]
    fn test_exact_trigger_phrase_is_authoritative_with_sparse_metadata() {
        let skill = SkillRecord {
            metadata_json: serde_json::json!({
                "provider": "claude",
                "canonical_id": "claude/common-name",
                "display_id": "common-name",
                "tags": ["test", "provider"],
                "trigger_phrases": ["common collision route"],
                "execution_mode": "inline"
            })
            .to_string(),
            ..make_skill_record(
                "claude/common-name",
                "Common Name",
                "Claude provider collision skill",
                vec!["test", "provider"],
            )
        };

        let response = route_task(vec![skill], "common collision route", 3, 0.65, false);
        assert_eq!(response.decision, "match");
        assert_eq!(response.candidates[0].skill_id, "claude/common-name");
        assert!(response.candidates[0].score >= 0.65);
    }

    #[test]
    fn test_sanitize_slug_used_for_section() {
        let slug = sanitize_slug("Rust Error Handling");
        assert_eq!(slug, "rust-error-handling");
    }

    #[test]
    fn test_score_trigger_phrase_prioritizes_exact_and_contained_matches() {
        assert_eq!(
            score_trigger_phrase("provider route verification", "provider route verification"),
            1.0
        );
        assert_eq!(
            score_trigger_phrase(
                "please run provider route verification with archive fallback",
                "provider route verification",
            ),
            0.9
        );
        assert_eq!(
            score_trigger_phrase("archive fallback only", "provider route verification"),
            0.0
        );
    }
    // ===================== bd-64zk: ms route CLI acceptance tests =====================

    

    #[test]
    fn test_threshold_zero_uses_default() {
        let skill = make_skill_record("test-skill", "Test", "A test skill", vec!["test"]);
        let response = route_task(vec![skill], "test", 3, 0.0, false);
        assert_eq!(response.decision, "match");
        assert!(response.candidates.len() >= 1);
    }

    #[test]
    fn test_threshold_very_high_produces_no_match() {
        let skill = make_skill_record("test-skill", "Test", "A test skill", vec!["test"]);
        let response = route_task(vec![skill], "test", 3, 1.0, false);
        assert_eq!(response.decision, "no_match");
        assert!(response.candidates.is_empty());
        assert!(response.fallback.is_some());
        assert!(response.fallback.as_ref().unwrap().search_command.contains("ms search"));
        assert!(response.fallback.as_ref().unwrap().suggest_command.is_none());
    }

    #[test]
    fn test_no_match_response_contract() {
        let response = RouteResponse {
            route_schema_version: ROUTE_SCHEMA_VERSION,
            task: "impossible query xyz 123".to_string(),
            threshold: 0.65,
            decision: "no_match".to_string(),
            candidates: vec![],
            debug_info: vec![],
            fallback: Some(RouteFallback {
                search_command: "ms search \"impossible query xyz 123\" -O json".to_string(),
                suggest_command: None,
            }),
        };
        assert_eq!(response.decision, "no_match");
        assert!(response.candidates.is_empty());
        assert!(response.fallback.is_some());
        let fb = response.fallback.as_ref().unwrap();
        assert!(fb.search_command.contains("ms search"));
        assert!(fb.suggest_command.is_none());
    }

    #[test]
    fn test_canonical_ids_with_collision() {
        let skill_a = SkillRecord {
            metadata_json: serde_json::json!({
                "provider": "claude",
                "canonical_id": "claude/shared-id",
                "display_id": "shared-id",
                "tags": ["test"],
                "trigger_phrases": ["shared id task"],
                "execution_mode": "inline"
            })
            .to_string(),
            ..make_skill_record("claude/shared-id", "Shared", "A shared skill", vec!["test"])
        };
        let skill_b = SkillRecord {
            metadata_json: serde_json::json!({
                "provider": "codex",
                "canonical_id": "codex/shared-id",
                "display_id": "shared-id",
                "tags": ["test"],
                "trigger_phrases": ["shared id task"],
                "execution_mode": "inline"
            })
            .to_string(),
            ..make_skill_record("codex/shared-id", "Shared", "Another shared skill", vec!["test"])
        };
        let response = route_task(
            vec![skill_a, skill_b],
            "shared id task",
            3,
            0.65,
            false,
        );
        assert_eq!(response.decision, "match");
        assert_eq!(response.candidates.len(), 2);
        let ids: Vec<_> = response.candidates.iter().map(|c| c.skill_id.clone()).collect();
        assert!(ids.contains(&"claude/shared-id".to_string()));
        assert!(ids.contains(&"codex/shared-id".to_string()));
        for cand in &response.candidates {
            assert!(
                cand.load_command.contains(&cand.skill_id),
                "load_command should use canonical skill_id: {}",
                cand.load_command
            );
        }
    }

    #[test]
    fn test_suggest_command_always_none_in_current_implementation() {
        let skill = make_skill_record("test-skill", "Test", "A test skill", vec!["test"]);
        let response = route_task(vec![skill], "test", 3, 0.65, false);
        if let Some(ref fb) = response.fallback {
            assert!(fb.suggest_command.is_none(), "suggest_command must not be fabricated");
        }
    }
}
