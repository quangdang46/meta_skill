//! Property-based tests for safety - ensuring parsers and serializers never panic.

use proptest::prelude::*;

use ms::config::Config;
use ms::core::skill::{BlockType, SkillBlock, SkillMetadata, SkillSection, SkillSpec};

fn arb_block_type() -> impl Strategy<Value = BlockType> {
    prop_oneof![
        Just(BlockType::Text),
        Just(BlockType::Code),
        Just(BlockType::Rule),
        Just(BlockType::Pitfall),
        Just(BlockType::Command),
        Just(BlockType::Checklist),
    ]
}

fn arb_skill_spec() -> impl Strategy<Value = SkillSpec> {
    let block = (r"[a-z][a-z0-9_\-]{2,16}", arb_block_type(), ".{0,120}").prop_map(
        |(id, block_type, content)| SkillBlock {
            id,
            block_type,
            content,
        },
    );

    let section = (
        r"[a-z][a-z0-9_\-]{2,16}",
        ".{1,32}",
        prop::collection::vec(block, 0..4),
    )
        .prop_map(|(id, title, blocks)| SkillSection { id, title, blocks });

    (
        "[a-z][a-z0-9_]{2,16}",
        ".{1,32}",
        r"[0-9]+\.[0-9]+\.[0-9]+",
        ".{0,80}",
        prop::collection::vec("[a-z]{2,12}", 0..4),
        prop::collection::vec(section, 0..3),
    )
        .prop_map(
            |(id, name, version, description, tags, sections)| SkillSpec {
                format_version: SkillSpec::FORMAT_VERSION.to_string(),
                metadata: SkillMetadata {
                    id,
                    name,
                    version,
                    description,
                    tags,
                    ..Default::default()
                },
                sections,
                extends: None,
                replace_rules: false,
                replace_examples: false,
                replace_pitfalls: false,
                replace_checklist: false,
                includes: Vec::new(),
                archive_format_version: None,
                provenance: None,
            },
        )
}

fn arb_config() -> impl Strategy<Value = Config> {
    (
        prop_oneof![
            Just("minimal".to_string()),
            Just("moderate".to_string()),
            Just("standard".to_string()),
        ],
        100u32..2000u32,
        any::<bool>(),
        0u64..10_000u64,
        prop_oneof![Just("hash".to_string()), Just("none".to_string())],
        16u32..512u32,
        0.0f32..1.0f32,
        0.0f32..1.0f32,
    )
        .prop_map(
            |(
                default_level,
                token_budget,
                auto_suggest,
                cooldown_seconds,
                backend,
                dims,
                bm25_weight,
                semantic_weight,
            )| {
                let mut config = Config::default();
                config.disclosure.default_level = default_level;
                config.disclosure.token_budget = token_budget;
                config.disclosure.auto_suggest = auto_suggest;
                config.disclosure.cooldown_seconds = cooldown_seconds;
                config.search.embedding_backend = backend;
                config.search.embedding_dims = dims;
                config.search.bm25_weight = bm25_weight;
                config.search.semantic_weight = semantic_weight;
                config
            },
        )
}

proptest! {
    #[test]
    fn test_validate_spec_never_panics(spec in arb_skill_spec()) {
        let _ = ms::core::validation::validate(&spec);
    }

    #[test]
    fn test_validate_empty_spec_never_panics(_seed in any::<u64>()) {
        let spec = SkillSpec::new("", "");
        let _ = ms::core::validation::validate(&spec);
    }

    #[test]
    fn test_skill_spec_json_serialize_never_panics(spec in arb_skill_spec()) {
        let _ = serde_json::to_string(&spec);
    }

    #[test]
    fn test_skill_spec_json_deserialize_never_panics(input in ".*") {
        let _: Result<SkillSpec, _> = serde_json::from_str(&input);
    }

    #[test]
    fn test_config_toml_serialize_never_panics(config in arb_config()) {
        let _ = toml::to_string(&config);
    }

    #[test]
    fn test_config_toml_deserialize_never_panics(input in ".*") {
        let _: Result<Config, _> = toml::from_str(&input);
    }

    #[test]
    fn test_config_default_never_panics(_seed in any::<u64>()) {
        let _ = Config::default();
    }

    #[test]
    fn test_skill_spec_new_never_panics(id in ".*", name in ".*") {
        let _ = SkillSpec::new(id, name);
    }

    #[test]
    fn test_parse_markdown_never_panics(input in ".*") {
        let _ = ms::core::spec_lens::parse_markdown(&input);
    }

    #[test]
    fn test_parse_markdown_arbitrary_bytes(bytes in prop::collection::vec(any::<u8>(), 0..1000)) {
        let input = String::from_utf8_lossy(&bytes);
        let _ = ms::core::spec_lens::parse_markdown(&input);
    }
}
