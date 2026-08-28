# Home Assistant integration

How Kenny's Home Assistant sends events to Almanac (K7), and why it is
split into two pieces.

## Why two scripts

`script.http_post_with_retry` knows nothing about Almanac: give it a
URL, a bearer token and a JSON payload and it POSTs, retrying on any
non-2xx or connection failure with 5s / 30s / 120s backoff, and raising
exactly one notification if every attempt fails. Any future service can
reuse it unchanged.

`script.almanac_send` is a thin wrapper that only supplies Almanac's
address, token and field shape. It contains no retry logic of its own.

The split is deliberate (Kenny, 2026-08-28: "DRY is belangrijk en
modulariteit ook"): retry behaviour is a property of talking to any
HTTP service, not of talking to Almanac.

## What this closes

Almanac already guarantees delivery once it has answered 202 — the
journal and worker loop see to that even across a crash (AR16). The one
gap that guarantee cannot cover is *Almanac being unreachable at the
moment of sending*: process restarting, LXC rebooting, network down.
Only the sender can close that, which is exactly what the retry script
does. Together they make the chain lossless end to end.

## Status

`script.http_post_with_retry` is **installed** (storage mode).

Two pieces are **not** installed yet, deliberately:

1. **`rest_command.post_json_with_auth`** — a script cannot perform HTTP
   by itself; it needs a `rest_command`. The ha-mcp server does not
   allow writing `rest_command` (it is a YAML-only integration outside
   its allowlist), so this is a manual paste into `configuration.yaml`:

   ```yaml
   rest_command:
     post_json_with_auth:
       url: "{{ url }}"
       method: post
       content_type: "application/json"
       payload: "{{ payload | to_json }}"
       headers:
         Authorization: "Bearer {{ token }}"
       timeout: 15
   ```

2. **`script.almanac_send`** — written but not installed. It references
   `!secret almanac_base_url` and `!secret almanac_token`, and neither
   exists yet because Almanac has no deployment and no issued token
   (tokens come from the L4b dashboard). Installing it now would make
   HA's config check fail on the missing secrets. It lands with the L5
   deployment, once there is an address and a token to point it at.

## Usage, once complete

```yaml
# from any automation
action: script.almanac_send
data:
  title: "Wasmachine klaar"
  entity_id: "{{ trigger.entity_id }}"
```

## Note on a best-practice warning

The retry loop's exit condition is a template comparison on the HTTP
status. The ha-mcp best-practice checker flags template comparisons and
suggests a native `numeric_state` condition — that suggestion does not
apply here: `numeric_state` tests an *entity*, and this compares a
script-local response variable, which has no entity. The template is
the only available construct.
