# Contract: view tree (api 1.0, this step)

A card is a JSON tree of nodes `{ "type": "<primitive>", ... }`. The host
draws it with its own components and the design-system tokens; plugins never
choose colours, fonts or pixel sizes.

## Primitives

| Type | Fields | Drawn as |
| --- | --- | --- |
| `stack` | `children` | vertical list |
| `row` | `children` | horizontal line, wraps nothing: overflow is cut with an ellipsis |
| `text` | `key`, `args` (object of strings), `role` (`title`, `body`, `muted`, `value`, `mono`) | text from the plugin's locale file; `{name}` replaced by `args.name`, nothing else |
| `duration` | `value` (seconds), `since` (instant, optional) | the app's duration format; with `since`, the value grows with the clock, drawn by the host |
| `countdown` | `until` (instant) | time left, drawn by the host |
| `time` | `at` (instant) | the app's clock format in the user's zone |
| `badge` | `text` (a `text` node), `tone` (`info`, `success`, `warning`, `error`, `muted`) | the design system's badge |
| `progress` | `value`, `max`, `label` (a `text` node, required) | the design system's progress bar |
| `button` | `command`, `arg` (string, optional), `label` (a `text` node) | the design system's button; only in the action row |

The card's root is `{ "type": "card", "lines": [ ... ], "actions": [ ... ] }`:
`lines` holds at most 4 nodes, each drawn as one line; `actions` holds at
most 2 `button` nodes.

## Validation

At reception the host checks, and rejects the whole tree otherwise:

- depth at most 6, at most 64 nodes;
- each text, after substitution, at most 200 characters, `args` included;
- `key` exists in the plugin's locale for the app language or its fallback;
- `command` is a command the manifest declares;
- every primitive exists in the `api` the manifest declares;
- instants are valid RFC 3339; durations are between 0 and 7 days.

A rejected tree keeps the previous card and counts as a failure.

## Accessibility

- The host reads the card as: plugin name, then each line, then each button.
- Each `button` and `progress` has its `label` as accessible name.
- A card update is never announced; only the user's navigation reads it.

## Example

```json
{
  "type": "card",
  "lines": [
    { "type": "row", "children": [
      { "type": "text", "key": "longest", "role": "body" },
      { "type": "duration", "value": 5400 }
    ]},
    { "type": "row", "children": [
      { "type": "text", "key": "since-break", "role": "body" },
      { "type": "duration", "value": 0, "since": "2026-11-08T13:42:00Z" }
    ]},
    { "type": "row", "children": [
      { "type": "text", "key": "leave", "role": "muted" },
      { "type": "time", "at": "2026-11-08T16:55:00Z" }
    ]}
  ],
  "actions": [
    { "type": "button", "command": "toggle-range",
      "label": { "type": "text", "key": "show-week" } }
  ]
}
```
