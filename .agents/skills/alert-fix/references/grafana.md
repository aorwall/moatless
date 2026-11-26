# Querying Grafana for Moatless

Grafana serves Prometheus and Loki over its HTTP API. Use the datasource uids
and namespace from the Loop's deployment values, and filter every query on that
namespace: the same datasources can carry other products and environments.

```bash
: "${GRAFANA_URL:?GRAFANA_URL is not set in this sandbox; say so in the alert thread}"
G=$GRAFANA_URL
NS=<namespace> PROM=<prometheus-uid> LOKI=<loki-uid>
```

`GRAFANA_URL` is the pod's relay to the `grafana` service proxy
(`http://127.0.0.1:7781/grafana`). The backend drops any `Authorization` header
and injects the Grafana credential itself, so send none. Never call the Grafana
workspace URL directly or send `GRAFANA_TOKEN`: a brokered sandbox holds only a
placeholder, and the direct call returns 401.

If `GRAFANA_URL` is unset, this Workspace does not have the `grafana` service
on. Investigate with `kubectl -n "$NS"` only, and say in the reply that
metrics were not available. From the relay, a 403 means the service is off or
its rules refuse that path, and a 424 that the run-as user has no
`GRAFANA_TOKEN` secret for the backend to inject.

## Prometheus

```bash
# Instant value
curl -s -G "$G/api/datasources/proxy/uid/$PROM/api/v1/query" \
  --data-urlencode "query=max by (pod) (moatless_database_pool_connections{namespace=\"$NS\",state=\"in_use\"})" \
  | jq -r '.data.result[] | "\(.metric.pod)  \(.value[1])"'

# Range: the last hour at 1-minute steps
curl -s -G "$G/api/datasources/proxy/uid/$PROM/api/v1/query_range" \
  --data-urlencode "query=max by (pod) (container_memory_working_set_bytes{namespace=\"$NS\",container=\"backend\"}) / 1048576" \
  --data-urlencode "start=$(( $(date +%s) - 3600 ))" --data-urlencode "end=$(date +%s)" \
  --data-urlencode step=60 \
  | jq -r '.data.result[] | .metric.pod + ": " + ([.values[][1] | tonumber | floor | tostring] | join(" "))'
```

## Loki

```bash
curl -s -G "$G/api/datasources/proxy/uid/$LOKI/loki/api/v1/query_range" \
  --data-urlencode "query={namespace=\"$NS\", service_name=\"moatless\", log_level=\"ERROR\"}" \
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
curl -s "$G/api/prometheus/grafana/api/v1/rules" \
  | jq -r '.data.groups[] | select(.name | test("Moatless")) | .rules[]
           | select(.state != "inactive") | "\(.state)\t\(.name)"'
```

Loki keeps about 7 days and Prometheus about 31. An empty result outside that
window says nothing.
