-- Migration 014: Add archive format version and provenance to skills
-- Supports snapshot importer that preserves scripts, references, checksums

ALTER TABLE skills ADD COLUMN archive_format_version TEXT;
ALTER TABLE skills ADD COLUMN provenance_json TEXT NOT NULL DEFAULT '{}';
