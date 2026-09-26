# TICKET-005: blocklist-reload

**Type:** `wayfinder:grilling`
**Status:** closed

## Question

See MAP.md decisions-so-far.

## Resolution

SIGHUP triggers atomic AHashSet swap. File-watcher and polling ruled out.
