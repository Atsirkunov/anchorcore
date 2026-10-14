-- R38 markdown chunking: 1-based source line ranges on sections + chunks,
-- section kind (section | front_matter) + structured doc meta JSON
-- (ADR status/date/number) for the markdown section tree.
ALTER TABLE sections ADD COLUMN line_start INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sections ADD COLUMN line_end INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sections ADD COLUMN kind TEXT NOT NULL DEFAULT 'section';
ALTER TABLE sections ADD COLUMN meta TEXT NOT NULL DEFAULT '{}';
ALTER TABLE chunks ADD COLUMN line_start INTEGER NOT NULL DEFAULT 0;
ALTER TABLE chunks ADD COLUMN line_end INTEGER NOT NULL DEFAULT 0;
