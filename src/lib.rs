//! Library half of `animus-notifier-http`.
//!
//! The HTTP + Slack webhook notifier plugin for Animus. Implements the
//! `notifier/notify`, `notifier/flush`, `notifier/schema`, and
//! `health/check` JSON-RPC methods on top of stdin/stdout, with the same
//! retry + dead-letter semantics the legacy in-tree
//! `orchestrator-notifications` crate provided.
//!
//! State (outbox, dead-letter, per-project connector config) lives under
//! `~/.animus/<repo-scope>/notifications/` exactly as before, so an
//! upgrade from the in-tree implementation to this plugin transparently
//! adopts existing on-disk state.

pub mod plugin;
pub mod runtime;

pub use plugin::run;
pub use runtime::{DaemonNotificationRuntime, NotificationLifecycleEvent};
