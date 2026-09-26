# TICKET-014: Docker Compose service topology

**Type:** `wayfinder:prototype`
**Status:** closed

## Resolution

```
Host ports:   8899 (RPC) → aegis   3000 (Grafana)   9091 (Prometheus)
Internal:     validator:8900   aegis:9090 (/metrics)

Services:
  aegis        build: .   ports: 8899:8899, 9090:9090
               env: DOWNSTREAM_URL=http://validator:8900
                    BLOCKLIST_PATH=/data/blocklist.txt
               volumes: ./blocklist.txt:/data/blocklist.txt:ro
               depends_on: validator

  validator    image: solanalabs/solana:latest
               command: solana-test-validator --rpc-port 8900 --quiet
               ports: (internal only)

  prometheus   image: prom/prometheus:latest
               volumes: ./infra/prometheus.yml:/etc/prometheus/prometheus.yml
               ports: 9091:9090
               depends_on: aegis

  grafana      image: grafana/grafana:latest
               ports: 3000:3000
               volumes: ./infra/grafana/provisioning:/etc/grafana/provisioning
               depends_on: prometheus
```

Grafana pre-provisioned: datasource pointing to `http://prometheus:9090`, dashboard JSON bundled in `infra/grafana/provisioning/dashboards/`.
