#!/bin/bash
# A1–A5 against Entra ID tokens; prints status codes, OperationOutcome text and claims — never a token
source /home/angela/pass-1182/env.sh; unset CURL_HOME; cd $WORK
oo(){ python3 -c 'import sys,json
try:
  d=json.load(sys.stdin); i=(d.get("issue") or [{}])[0]; print(d.get("resourceType"), i.get("code",""), (i.get("details") or {}).get("text") or i.get("diagnostics",""))
except Exception as e: print("(non-JSON)")'; }
req(){ m=$1; u=$2; shift 2; curl -s -o /tmp/a.out -w '%{http_code}' -X $m "$@" "$HFS$u"; }
L0=$(wc -l < hfs-sqlite-entra.log)
echo "## A1–A5 $(date -u +%FT%TZ)"
echo "### A1 no token"
printf 'GET /Patient -> %s ' $(req GET /Patient); oo < /tmp/a.out
for p in /health /metadata /.well-known/smart-configuration; do printf 'GET %s -> %s\n' $p $(req GET $p); done
printf 'GET /ui -> %s location host: ' $(curl -s -o /dev/null -D /tmp/h.txt -w '%{http_code}' $HFS/ui); grep -i '^location' /tmp/h.txt | sed -E 's#^location: *##I; s#(https?://[^/]+).*#\1#' | tr -d '\r'; echo
printf 'GET /ui/login (follow to IdP) -> '; curl -s -o /dev/null -D /tmp/h2.txt -w '%{http_code} ' $HFS/ui/login; grep -i '^location' /tmp/h2.txt | sed -E 's#^location: *##I; s#\?.*##; s#[0-9a-f]{8}-[0-9a-f-]{27}#<tenant>#' | tr -d '\r'
FULL=$(etok hfs); RO=$(etok ro)
echo "### A2 HFS_CLIENT_ID token: $(echo -n "$FULL" | python3 claims.py)"
printf 'GET /Patient?_count=1 -> %s\n' $(req GET '/Patient?_count=1' -H "Authorization: Bearer $FULL")
printf 'POST /Patient -> %s ' $(req POST /Patient -H "Authorization: Bearer $FULL" -H 'Content-Type: application/fhir+json' -d '{"resourceType":"Patient","name":[{"family":"EntraA2","given":["Probe"]}],"gender":"other"}'); python3 -c 'import json;d=json.load(open("/tmp/a.out"));print(d.get("resourceType"),d.get("id"))'
echo "### A3 RO_CLIENT_ID token: $(echo -n "$RO" | python3 claims.py)"
printf 'GET /Patient?_count=1 -> %s\n' $(req GET '/Patient?_count=1' -H "Authorization: Bearer $RO")
printf 'POST /Patient -> %s ' $(req POST /Patient -H "Authorization: Bearer $RO" -H 'Content-Type: application/fhir+json' -d '{"resourceType":"Patient","name":[{"family":"EntraA3"}]}'); oo < /tmp/a.out
echo "### A4"
BAD=$(python3 - "$FULL" <<'PY'
import sys; t=sys.argv[1]; h,p,s=t.split('.'); s=('A' if s[0]!='A' else 'B')+s[1:]; print(f'{h}.{p}.{s}',end='')
PY
)
printf 'tampered signature -> %s ' $(req GET '/Patient?_count=1' -H "Authorization: Bearer $BAD"); oo < /tmp/a.out
EXP=$(cat private/a4-expiring.jwt); echo "saved token: $(python3 claims.py < private/a4-expiring.jwt), now $(date -u +%FT%TZ)"
printf 'expired token -> %s ' $(req GET '/Patient?_count=1' -H "Authorization: Bearer $EXP"); oo < /tmp/a.out
GRAPH=$(etok hfs https://graph.microsoft.com/.default); echo "Graph token: $(echo -n "$GRAPH" | python3 claims.py)"
printf 'Graph token -> %s ' $(req GET '/Patient?_count=1' -H "Authorization: Bearer $GRAPH"); oo < /tmp/a.out
echo "server log for A4:"; tail -n +$((L0+1)) hfs-sqlite-entra.log | grep -a 'Authentication failed' | sed -E 's/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/<id>/g' | cut -c1-200 | tail -6
echo "### A5 /.well-known/smart-configuration"
curl -s $HFS/.well-known/smart-configuration | python3 -c 'import sys,json,re; d=json.load(sys.stdin); red=lambda v: re.sub(r"[0-9a-f]{8}-[0-9a-f-]{27}","<tenant>",v) if isinstance(v,str) else v; [print(" ",k,"=",red(d.get(k))) for k in ("issuer","token_endpoint","authorization_endpoint","jwks_uri","end_session_endpoint","grant_types_supported","capabilities")]'
