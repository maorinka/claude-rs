# CLAUDE.md

## General Principles

Avoid over-engineering simple fixes. Prefer minimal, targeted changes unless explicitly asked for a broader refactor.

## Porting & Reference Code

When porting code between languages (especially TS→Rust), ALWAYS read the original reference implementation before writing new code. Never implement from memory or assumptions.

## Communication & Clarification

When asked to redo or regenerate a list/analysis, clarify whether the user wants a fresh complete re-analysis or just an update to the existing one before proceeding.

## Rust Development

For Rust projects: always ensure `target/` is in .gitignore and never include build artifacts in commits. Run `git status` before committing to verify.

## Code Editing

When editing files with duplicate keys or complex structures, use Read to verify the exact location before applying edits. Never assume position based on key name alone.
