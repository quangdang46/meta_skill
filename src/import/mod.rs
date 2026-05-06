//! Skill import module for parsing unstructured prompts and documents.
//!
//! This module provides tools to analyze unstructured text (system prompts,
//! documentation, READMEs) and classify content blocks into appropriate
//! SkillSpec sections (rules, examples, pitfalls, checklists, etc.).
//!
//! # Architecture
//!
//! The import pipeline consists of:
//! 1. **Content Parser** - Splits text into logical blocks
//! 2. **Block Classifiers** - Classify each block by type
//! 3. **Skill Generator** - Transform classified blocks into SkillSpec
//!
//! # Example
//!
//! ```ignore
//! use ms::import::{ContentParser, SkillGenerator, ImportHints};
//!
//! // Parse unstructured text into classified blocks
//! let parser = ContentParser::new();
//! let blocks = parser.parse(prompt_text);
//!
//! // Generate a SkillSpec from the classified blocks
//! let generator = SkillGenerator::new();
//! let result = generator.generate(blocks, &ImportHints::default());
//!
//! println!("Generated skill: {}", result.skill.metadata.id);
//! println!("Rules: {}, Examples: {}", result.stats.rules_count, result.stats.examples_count);
//! ```

mod classifiers;
pub mod formatting;
mod generator;
mod parser;
pub mod provider;
mod types;

pub use classifiers::*;
pub use generator::*;
pub use parser::*;
pub use types::*;
