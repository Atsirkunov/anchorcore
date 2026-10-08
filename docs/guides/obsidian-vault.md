# Obsidian vault → AnchorCore

Keep your vault. Point AnchorCore at the folder. Same notes, better answers.

## Why

AI plugins for Obsidian usually stuff whole notes into the model's memory:
slow, expensive, and answers come back without sources. AnchorCore instead
finds the exact passages and cites them — the AI sees a handful of quoted
sections, not 40 files.

## Setup (3 steps, nothing to migrate)

1. Sources → Local folder → select your vault directory → Sync.
2. Ask — e.g. "what did I decide about X, and why?"
3. Every answer cites `file › section` — click through to the note.

New and edited notes are picked up automatically. Your data stays on your
machine; the vault itself is only ever read, never modified.

## Notes

- Headings become the section tree, so answers cite exact sections.
- `[[wikilinks]]`, `#tags`, and frontmatter are read as plain text today;
  richer handling (links as relationships, tags as topics, dates as metadata)
  is on the roadmap.
