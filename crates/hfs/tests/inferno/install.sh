#!/bin/bash
#
# Load Inferno test data into HFS
#
# Usage: ./install.sh [HFS_URL]
#
# Environment variables:
#   HFS_PORT - Port where HFS is running (default: 8080)
#   HFS_HOST - Host where HFS is running (default: localhost)
#   HFS_URL  - Full URL to HFS (overrides HFS_HOST and HFS_PORT)
#
# Examples:
#   ./install.sh                           # Uses localhost:8080
#   HFS_PORT=8088 ./install.sh             # Uses localhost:8088
#   ./install.sh http://localhost:9000     # Uses specified URL
#

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Determine HFS URL
if [ -n "$1" ]; then
    HFS_URL="$1"
elif [ -n "$HFS_URL" ]; then
    : # Use HFS_URL from environment
else
    HFS_HOST="${HFS_HOST:-localhost}"
    HFS_PORT="${HFS_PORT:-8080}"
    HFS_URL="http://${HFS_HOST}:${HFS_PORT}"
fi

echo "Loading Inferno test data into HFS at ${HFS_URL}..."
echo ""

# Does this server support transaction bundles?
#
# Not every backend can. A transaction is all-or-nothing, which S3 cannot
# provide — it has no atomic multi-object operation — so an s3-elasticsearch
# deployment declines them rather than committing partially (#489). The
# fixtures are all transaction bundles, so on those backends they are rewritten
# to batch bundles first (see to_batch.jq).
#
# Asked, not assumed: the CapabilityStatement is derived from the configured
# backend, so this stays correct as backends are added without the loader
# needing to know their names.
TRANSFORM=""
SUPPORTS_TRANSACTION=$(curl -s "${HFS_URL}/metadata" \
    | jq -r 'try ([.rest[]?.interaction[]?.code] | index("transaction") != null) catch false' 2>/dev/null)

if [ "$SUPPORTS_TRANSACTION" = "true" ]; then
    echo "Server advertises 'transaction'; loading bundles as-is."
else
    TRANSFORM="$SCRIPT_DIR/to_batch.jq"
    if [ ! -f "$TRANSFORM" ]; then
        echo "ERROR: server does not support transactions and $TRANSFORM is missing"
        exit 1
    fi
    echo "Server does not advertise 'transaction'; rewriting bundles to batch."
    echo "  (urn:uuid references are resolved client-side; POST becomes PUT)"
fi
echo ""

FAILED=0
SUCCESS=0
SKIPPED=0

for FILE in "$SCRIPT_DIR"/*.json; do
    FILENAME=$(basename "$FILE")
    echo "Processing $FILENAME..."

    # Determine resource type and endpoint
    RESOURCE_TYPE=$(jq -r '.resourceType // empty' "$FILE")
    BUNDLE_TYPE=$(jq -r '.type // empty' "$FILE")

    if [ "$BUNDLE_TYPE" = "transaction" ]; then
        ENDPOINT="/"
        if [ -n "$TRANSFORM" ]; then
            echo "  Type: transaction bundle -> rewritten to batch -> POST $ENDPOINT"
        else
            echo "  Type: transaction bundle -> POST $ENDPOINT"
        fi
    elif [ "$RESOURCE_TYPE" = "SearchParameter" ]; then
        ENDPOINT="/SearchParameter"
        echo "  Type: SearchParameter -> POST $ENDPOINT"
    elif [ "$RESOURCE_TYPE" = "Group" ]; then
        ENDPOINT="/Group"
        echo "  Type: Group -> POST $ENDPOINT"
    else
        echo "  WARNING: Unknown type (resourceType=$RESOURCE_TYPE, type=$BUNDLE_TYPE), skipping"
        SKIPPED=$((SKIPPED + 1))
        continue
    fi

    # Rewrite only transaction bundles; SearchParameter and Group posts are
    # single resources and unaffected.
    PAYLOAD="$FILE"
    if [ -n "$TRANSFORM" ] && [ "$BUNDLE_TYPE" = "transaction" ]; then
        PAYLOAD=$(mktemp)
        if ! jq -f "$TRANSFORM" "$FILE" > "$PAYLOAD"; then
            echo "  FAILED (could not rewrite $FILENAME to batch)"
            rm -f "$PAYLOAD"
            FAILED=$((FAILED + 1))
            continue
        fi
    fi

    RESPONSE=$(curl -s -w "\n%{http_code}" -X POST "${HFS_URL}${ENDPOINT}" \
        -H "Content-Type: application/fhir+json" \
        --data-binary @"$PAYLOAD")

    [ "$PAYLOAD" != "$FILE" ] && rm -f "$PAYLOAD"

    HTTP_CODE=$(echo "$RESPONSE" | tail -n1)
    BODY=$(echo "$RESPONSE" | sed '$d')

    if [ "$HTTP_CODE" -ge 200 ] && [ "$HTTP_CODE" -lt 300 ]; then
        echo "  Success (HTTP $HTTP_CODE)"
        SUCCESS=$((SUCCESS + 1))
    else
        echo "  FAILED (HTTP $HTTP_CODE)"
        echo "  $BODY" | head -c 500
        echo ""
        FAILED=1
    fi
done

echo ""
echo "Summary: $SUCCESS succeeded, $SKIPPED skipped"

if [ "$FAILED" -eq 1 ]; then
    echo "One or more files failed to load"
    exit 1
fi

echo "All test data loaded successfully"
