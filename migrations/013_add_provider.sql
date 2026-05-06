-- 013_add_provider.sql
-- Add provider column to skills table for tracking skill origin

ALTER TABLE skills ADD COLUMN provider TEXT;
