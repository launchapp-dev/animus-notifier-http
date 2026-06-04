# animus-notifier-http

HTTP + Slack webhook notifier plugin for [Animus](https://github.com/launchapp-dev/animus-cli).

Notifiers are the outbound counterpart to triggers: the daemon publishes
events; a notifier plugin forwards each event to an external system. This
plugin handles two connector kinds out of the box:

- `webhook` — POST `application/json` to a configured URL with optional
  configured headers (all values resolved from env vars).
- `slack_webhook` — Slack-incoming-webhook-shaped payload with optional
  `username`, `channel`, and `icon_emoji` overrides.

State (outbox, dead-letter, per-project connector config) is partitioned
by project root and stored under
`~/.animus/<repo-scope>/notifications/`. The on-disk shape is identical
to the legacy in-tree `orchestrator-notifications` implementation, so an
upgrade transparently adopts existing state.

## Install

```bash
animus plugin install launchapp-dev/animus-notifier-http@v0.1.0
```

The daemon refuses to use a notifier role unless one is explicitly
configured: `notifier` is treated as an OPTIONAL role by daemon
preflight. Daemons without an installed notifier plugin still start
cleanly and simply skip outbound delivery.

## Configure

Connectors and subscriptions live under `notification_config` in your
project-local `.animus/pm-config.json` (or under the scoped path
`~/.animus/<repo-scope>/daemon/pm-config.json` if you store daemon
settings there). The schema is unchanged from the in-tree predecessor:

```json
{
  "notification_config": {
    "schema": "animus.daemon-notification-config.v1",
    "version": 1,
    "connectors": [
      {
        "type": "webhook",
        "id": "ops-webhook",
        "url_env": "OPS_WEBHOOK_URL"
      },
      {
        "type": "slack_webhook",
        "id": "team-slack",
        "webhook_url_env": "SLACK_WEBHOOK_URL",
        "channel": "#animus"
      }
    ],
    "subscriptions": [
      {
        "id": "all-events-to-ops",
        "connector_id": "ops-webhook",
        "event_types": ["*"]
      }
    ]
  }
}
```

Credentials never live in YAML or JSON: each connector references an
env var (`url_env`, `webhook_url_env`, `headers_env.*`) that the daemon
process must export.

## Protocol

This plugin implements:

- `notifier/notify { event: DaemonEventRecord }` — enqueue + flush in
  one call; returns lifecycle records the daemon mirrors into
  `events.jsonl`.
- `notifier/flush { project_root?: string }` — retry pending deliveries
  from the on-disk outbox.
- `notifier/schema {}` — capability declaration.
- `health/check {}` — standard.

See [`animus-notifier-protocol`](https://github.com/launchapp-dev/animus-protocol)
for the wire types.

## License

MIT
