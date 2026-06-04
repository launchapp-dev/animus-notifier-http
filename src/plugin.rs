//! Stdio JSON-RPC entrypoint for the `animus-notifier-http` plugin.
//!
//! Implements `notifier/notify`, `notifier/flush`, `notifier/schema`,
//! and `health/check` on top of the generic `Plugin` builder from
//! `animus-plugin-runtime`. State is partitioned by `project_root`: the
//! plugin keeps a per-project [`DaemonNotificationRuntime`] so a single
//! plugin process can serve every project the daemon supervises.

use std::collections::HashMap;
use std::sync::Arc;

use animus_notifier_protocol::{
    NotifierFlushParams, NotifierFlushResult, NotifierLifecycleEvent, NotifierNotifyParams,
    NotifierNotifyResult, NotifierSchema, METHOD_NOTIFIER_FLUSH, METHOD_NOTIFIER_NOTIFY,
    METHOD_NOTIFIER_SCHEMA, PLUGIN_KIND_NOTIFIER,
};
use animus_plugin_protocol::error_codes;
use animus_plugin_runtime::{register_method, Plugin};
use anyhow::Result;
use tokio::sync::Mutex;

use crate::runtime::{DaemonNotificationRuntime, NotificationLifecycleEvent};

const PLUGIN_NAME: &str = "animus-notifier-http";
const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");
const PLUGIN_DESCRIPTION: &str =
    "HTTP + Slack webhook notifier plugin for Animus (forwards daemon events to external systems with retry + dead-letter).";

/// Stable entrypoint for the plugin process. Call from `#[tokio::main]`
/// in `main.rs`.
pub async fn run() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .try_init();

    let state: Arc<Mutex<HashMap<String, DaemonNotificationRuntime>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let notify_state = state.clone();
    let flush_state = state.clone();

    let plugin = Plugin::new(PLUGIN_NAME, PLUGIN_VERSION, PLUGIN_KIND_NOTIFIER)
        .description(PLUGIN_DESCRIPTION)
        .methods([
            METHOD_NOTIFIER_NOTIFY,
            METHOD_NOTIFIER_FLUSH,
            METHOD_NOTIFIER_SCHEMA,
            "health/check",
        ]);

    let plugin = register_method!(
        plugin,
        METHOD_NOTIFIER_NOTIFY,
        NotifierNotifyParams => NotifierNotifyResult,
        move |params, _ctx| {
            let state = notify_state.clone();
            async move { handle_notify(state, params).await }
        },
    );

    let plugin = register_method!(
        plugin,
        METHOD_NOTIFIER_FLUSH,
        NotifierFlushParams => NotifierFlushResult,
        move |params, _ctx| {
            let state = flush_state.clone();
            async move { handle_flush(state, params).await }
        },
    );

    let plugin = register_method!(
        plugin,
        METHOD_NOTIFIER_SCHEMA,
        serde_json::Value => NotifierSchema,
        |_req, _ctx| async move {
            Ok(NotifierSchema {
                connector_kinds: vec!["webhook".to_string(), "slack_webhook".to_string()],
                supports_flush: true,
            })
        },
    );

    plugin.run().await
}

async fn handle_notify(
    state: Arc<Mutex<HashMap<String, DaemonNotificationRuntime>>>,
    params: NotifierNotifyParams,
) -> Result<NotifierNotifyResult, animus_plugin_protocol::RpcError> {
    let project_root = match params.event.project_root.clone() {
        Some(value) if !value.trim().is_empty() => value,
        _ => {
            return Err(animus_plugin_protocol::RpcError {
                code: error_codes::INVALID_PARAMS,
                message: "notifier/notify event.project_root is required".to_string(),
                data: None,
            });
        }
    };

    let mut state_guard = state.lock().await;
    let runtime = match state_guard.get_mut(&project_root) {
        Some(existing) => existing,
        None => {
            let new_runtime = DaemonNotificationRuntime::new(&project_root).map_err(|error| {
                animus_plugin_protocol::RpcError {
                    code: error_codes::INTERNAL_ERROR,
                    message: format!("failed to initialize notifier runtime: {error}"),
                    data: None,
                }
            })?;
            state_guard.insert(project_root.clone(), new_runtime);
            state_guard
                .get_mut(&project_root)
                .expect("just inserted entry must be present")
        }
    };

    let enqueue_events = runtime.enqueue_for_event(&params.event).map_err(|error| {
        animus_plugin_protocol::RpcError {
            code: error_codes::INTERNAL_ERROR,
            message: format!("notifier/notify enqueue failed: {error}"),
            data: None,
        }
    })?;
    let flush_events = runtime.flush_due_deliveries().await.map_err(|error| {
        animus_plugin_protocol::RpcError {
            code: error_codes::INTERNAL_ERROR,
            message: format!("notifier/notify flush failed: {error}"),
            data: None,
        }
    })?;

    let mut lifecycle = Vec::with_capacity(enqueue_events.len() + flush_events.len());
    lifecycle.extend(enqueue_events.into_iter().map(to_protocol_lifecycle));
    let delivered = flush_events
        .iter()
        .filter(|e| e.event_type == "notification-delivery-sent")
        .count() as u32;
    lifecycle.extend(flush_events.into_iter().map(to_protocol_lifecycle));

    let accepted = !lifecycle.is_empty();
    Ok(NotifierNotifyResult { accepted, delivered, lifecycle_events: lifecycle })
}

async fn handle_flush(
    state: Arc<Mutex<HashMap<String, DaemonNotificationRuntime>>>,
    params: NotifierFlushParams,
) -> Result<NotifierFlushResult, animus_plugin_protocol::RpcError> {
    let mut state_guard = state.lock().await;

    let targets: Vec<String> = match params.project_root.as_deref() {
        Some(root) if !root.trim().is_empty() => {
            if !state_guard.contains_key(root) {
                let runtime = match DaemonNotificationRuntime::new(root) {
                    Ok(runtime) => runtime,
                    Err(_error) => return Ok(NotifierFlushResult::default()),
                };
                state_guard.insert(root.to_string(), runtime);
            }
            vec![root.to_string()]
        }
        _ => state_guard.keys().cloned().collect(),
    };

    let mut lifecycle = Vec::new();
    for project_root in targets {
        if let Some(runtime) = state_guard.get_mut(&project_root) {
            match runtime.flush_due_deliveries().await {
                Ok(events) => lifecycle.extend(events.into_iter().map(to_protocol_lifecycle)),
                Err(error) => {
                    return Err(animus_plugin_protocol::RpcError {
                        code: error_codes::INTERNAL_ERROR,
                        message: format!(
                            "notifier/flush failed for project {project_root}: {error}"
                        ),
                        data: None,
                    });
                }
            }
        }
    }

    Ok(NotifierFlushResult { lifecycle_events: lifecycle })
}

fn to_protocol_lifecycle(event: NotificationLifecycleEvent) -> NotifierLifecycleEvent {
    NotifierLifecycleEvent {
        event_type: event.event_type,
        project_root: event.project_root,
        data: event.data,
    }
}
