# Querying Grafana for Moatless

The Soap Grafana workspace serves Prometheus and Loki over its HTTP API. Moatless
runs in the nonprod cluster, so always use the nonprod datasources `prometheus`
and `loki`, never the `-prod` ones, and filter on `namespace="moatless"`.

```bash
: "${GRAFANA_TOKEN:?GRAFANA_TOKEN is not set in this sandbox; say so in the alert thread}"
G=https://g-0000000000.grafana-workspace.eu-central-1.amazonaws.com
```

If `GRAFANA_TOKEN` is missing, investigate with `kubectl -n moatless` only and say
in the reply that metrics were not available.

## Prometheus

```bash
# Instant value
curl -s -G "$G/api/datasources/proxy/uid/prometheus/api/v1/query" \
  -H "Authorization: Bearer $GRAFANA_TOKEN" \
  --data-urlencode 'query=max by (pod) (moatless_database_pool_connections{namespace="moatless",state="in_use"})' \
  | jq -r '.data.result[] | "\(.metric.pod)  \(.value[1])"'

# Range: the last hour at 1-minute steps
curl -s -G "$G/api/datasources/proxy/uid/prometheus/api/v1/query_range" \
  -H "Authorization: Bearer $GRAFANA_TOKEN" \
  --data-urlencode 'query=max by (pod) (container_memory_working_set_bytes{namespace="moatless",container="backend"}) / 1048576' \
  --data-urlencode "start=$(( $(date +%s) - 3600 ))" --data-urlencode "end=$(date +%s)" \
  --data-urlencode step=60 \
  | jq -r '.data.result[] | .metric.pod + ": " + ([.values[][1] | tonumber | floor | tostring] | join(" "))'
```

## Loki

```bash
curl -s -G "$G/api/datasources/proxy/uid/loki/loki/api/v1/query_range" \
  -H "Authorization: Bearer $GRAFANA_TOKEN" \
  --data-urlencode 'query={namespace="moatless", service_name="moatless", log_level="ERROR"}' \
  --data-urlencode "start=$(( $(date +%s) - 3600 ))000000000" --data-urlencode "end=$(date +%s)000000000" \
  --data-urlencode limit=50 --data-urlencode direction=backward \
  | jq -r '.data.result[].values[][1]' | cut -c1-400
```

- Pass `direction=backward`, or a query that hits `limit` returns the oldest lines.
- Count before reading: `sum by (target) (count_over_time({...}[1h]))` against
  `/loki/api/v1/query` shows which logger is loud.
- A query wider than about 2 days can return HTTP 400; split it.

## Alert state

```bash
curl -s "$G/api/prometheus/grafana/api/v1/rules" -H "Authorization: Bearer $GRAFANA_TOKEN" \
  | jq -r '.data.groups[] | select(.name | test("Moatless")) | .rules[]
           | select(.state != "inactive") | "\(.state)\t\(.name)"'
```

Loki keeps about 7 days and Prometheus about 31. An empty result outside that
window says nothing.
