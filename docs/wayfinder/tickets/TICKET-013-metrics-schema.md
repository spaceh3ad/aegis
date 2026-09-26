# TICKET-013: Prometheus metrics schema

**Type:** `wayfinder:grilling`
**Status:** closed

## Resolution

Approved schema:

| Metric | Type | Labels | Buckets |
|---|---|---|---|
| `aegis_transactions_total` | Counter | `verdict=allow\|block\|drop` | — |
| `aegis_transactions_blocked_total` | Counter | `rule_id` | — |
| `aegis_inspection_duration_microseconds` | Histogram | — | 1,5,10,25,50,100,250µs |
| `aegis_blocklist_size` | Gauge | — | — |
| `aegis_downstream_up` | Gauge | — | — |
