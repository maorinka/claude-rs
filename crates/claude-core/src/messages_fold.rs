//! Message-stream folding / collapsing helpers for UI rendering.
//!
//! **RECONSTRUCTED module** — the canonical TS type source
//! `claude-code-leaked/src/types/message.ts` is **missing from the
//! leak snapshot**. It's referenced by 40+ files but never
//! materialised in the tracked tree. Rather than guess at the full
//! discriminated-union shape (`Message` / `RenderableMessage` /
//! `NormalizedMessage` / 14+ `System*` variants / 40+ attachment
//! subtypes), this module handles messages as `serde_json::Value`
//! streams — the on-the-wire shape is JSON regardless, and the
//! consumer helpers only touch a handful of discriminator fields
//! per call.
//!
//! Scope
//! =====
//! Each submodule ports a TS file that folds a stream of messages
//! for display:
//! - [`collapse_hook_summaries`] — TS
//!   `utils/collapseHookSummaries.ts:1-59`
//! - [`collapse_background_bash`] — TS
//!   `utils/collapseBackgroundBashNotifications.ts:1-84`
//! - [`group_tool_uses`] — TS `utils/groupToolUses.ts:1-182`
//! - [`context_analysis`] — TS `utils/contextAnalysis.ts:1-272`
//! - [`process_text_prompt`] — TS
//!   `utils/processUserInput/processTextPrompt.ts:1-100`
//!
//! Fields accessed per consumer are listed at the top of each
//! submodule as provenance for the reconstruction.

pub mod collapse_background_bash;
pub mod collapse_hook_summaries;
pub mod context_analysis;
pub mod group_tool_uses;
pub mod in_process_teammate_helpers;
pub mod process_text_prompt;
