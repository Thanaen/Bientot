# Contract: ActivityWatch Read-only REST Adapter

**Consumers**: `WorkTime.ActivityWatch`  
**Supported servers**: `aw-server-rust` (primary), Python `aw-server`  
**Default base URL**: `http://127.0.0.1:5600`
**Applies**: only while ActivityWatch is the selected activity source; the
built-in source is the default (D-021, [built-in source contract](built-in-activity-source.md))

## General rules

- Base URL is configurable and validated as HTTP(S).
- Requests are read-only and bounded to the dates needed for calculation.
- Default request timeout is 5 seconds.
- Temporary failures retain previously observed activity as stale/degraded and
  retry on later refreshes; they do not fabricate zero work.
- Retention uses the latest successfully read source intervals in memory, so
  local corrections and Windows state still recalculate during an outage.
  Elapsed time beyond retained evidence is unknown and uncredited.
- Temporary failures defer source attempts by 30, 60, 120, 240, then at most
  300 seconds. Local refresh continues every 30 seconds. A successful read
  resets the retry delay, even when its event coverage is incomplete.
- An authoritative empty or malformed snapshot replaces previous evidence.
  Selection ambiguity, non-transient errors, and a new workday or requested
  bucket invalidate retained evidence. Retention does not survive restart.
- HTTP 401 and 403 are authorization failures. HTTP 408, 429, and server
  errors are retryable; other unsupported responses are incompatible.
  Only 401 invokes the existing optional bearer-token retry.
- Optional authentication uses `Authorization: Bearer <key>`.
- Secrets and raw event payloads are never logged.

## Probe server

`GET /api/0/info`

Expected fields:

```json
{
  "hostname": "device-name",
  "version": "v0.x.y (rust)",
  "testing": false,
  "device_id": "optional-device-id"
}
```

`device_id` and implementation hints may be absent on older compatible servers.
Unknown additional fields are ignored.

Outcomes:

- `200`: parse capabilities and continue;
- timeout/connection failure: server unavailable;
- malformed body: incompatible server;
- redirects away from the configured local endpoint: do not follow without
  explicit validation.

## List buckets

`GET /api/0/buckets/`

Expected shape: object keyed by bucket ID. WorkTime selects buckets whose type
or ID identifies AFK status, preferring the probed local host/device. The
selected bucket remains visible and user-overridable when multiple candidates
are plausible.

Required metadata:

- bucket ID;
- type, normally `afkstatus`;
- hostname when present;
- client and creation metadata when present.

Outcomes:

- `200`: discover candidates;
- `401`: request/update optional API key;
- no candidate: degraded `afk-bucket-missing`;
- multiple equal candidates: degraded `bucket-selection-required`.

## Read AFK events

`GET /api/0/buckets/{bucket_id}/events?start={rfc3339}&end={rfc3339}`

Expected event:

```json
{
  "id": 123,
  "timestamp": "2026-07-27T08:00:00Z",
  "duration": 42.5,
  "data": {
    "status": "not-afk"
  }
}
```

Accepted status values are `afk` and `not-afk`. WorkTime:

- parses timestamps as instants;
- rejects negative/non-finite durations;
- clamps returned intervals to the requested range;
- sorts and normalizes out-of-order or overlapping events;
- marks malformed/unknown-status coverage as unknown;
- never assumes the server's boundary clipping semantics.

ActivityWatch heartbeat updates can produce short overlaps between `afk` and
`not-afk` events. WorkTime unions the conflicting intervals before measuring
their duration. A connected conflict of at most 100 ms, strictly inside the
requested range, is classified as AFK. This compatibility tolerance excludes
the disputed time from credited work. Longer conflicts, conflicts touching a
query boundary, and explicit unknown statuses remain unknown. Missing coverage
still degrades the workday summary and suppresses its displayed forecast.

## Compatibility fixtures

Contract tests contain sanitized responses for both server implementations and
cover:

- healthy server with one local AFK bucket;
- Rust and Python info variants;
- optional/missing `device_id`;
- bearer `401` then successful retry;
- multiple buckets and host mismatch;
- empty, stale, overlapping, malformed, and future events;
- temporary connection failure and recovery.

CI also runs a minimal live smoke test against both server implementations when
their supported test binaries are available.

## Detection during onboarding

Onboarding sends `GET /api/0/info` and `GET /api/0/buckets/` to the default
base URL once, without an API key. A compatible server with at least one AFK
bucket, or a `401` answer, counts as detected, and onboarding offers
ActivityWatch. Any other outcome counts as not detected: onboarding shows
nothing about ActivityWatch and keeps the built-in source. Detection never
blocks onboarding.

## Polling

Healthy current-day data refreshes every 30 seconds and immediately after a
relevant local transition or settings change. Failures back off to a maximum of
5 minutes while UI remains degraded. Manual refresh bypasses the current
backoff for its own source request. Concurrent ordinary refreshes do not
inherit or consume that bypass.
