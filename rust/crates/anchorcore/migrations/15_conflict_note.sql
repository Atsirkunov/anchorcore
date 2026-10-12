-- R27.4: free-text reason on conflict links (docs/plan-code-sections.md).
ALTER TABLE relationships ADD COLUMN note TEXT NOT NULL DEFAULT '';
