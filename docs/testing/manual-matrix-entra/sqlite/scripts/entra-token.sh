#!/bin/bash
# entra-token.sh [hfs|ro] [scope]: prints an access token (client credentials) to stdout; never log its output
set -a; . $HOME/entra-1182.env; set +a
case "${1:-hfs}" in hfs) id=$HFS_CLIENT_ID; sec=$HFS_CLIENT_SECRET;; ro) id=$RO_CLIENT_ID; sec=$RO_CLIENT_SECRET;; esac
scope=${2:-api://$HFS_CLIENT_ID/.default}
curl -s -X POST "https://login.microsoftonline.com/$ENTRA_TENANT/oauth2/v2.0/token" --data-urlencode "client_id=$id" --data-urlencode "client_secret=$sec" --data-urlencode "grant_type=client_credentials" --data-urlencode "scope=$scope" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d.get("access_token") or ("ERROR "+d.get("error","")+" "+d.get("error_description","")[:200]), end="")'
