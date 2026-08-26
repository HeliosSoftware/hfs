# HTS `/metadata` — request & response

Captured: 2026-08-24 10:23:58 -04:00  
Endpoint: `http://127.0.0.1:8090/metadata`  
Response size (raw): 8481 bytes  
Response size (pretty): 12903 bytes

## Request

```http
GET /metadata HTTP/1.1
Host: 127.0.0.1:8090
Accept: application/fhir+json
Accept-Encoding: identity
```

### Equivalent curl

```bash
curl -H "Accept: application/fhir+json" \
     -H "Accept-Encoding: identity" \
     "http://127.0.0.1:8090/metadata"
```

## Response

### Status & headers

```http
HTTP/1.1 200 OK
content-type: application/json
vary: accept-encoding
vary: origin, access-control-request-method, access-control-request-headers
access-control-allow-origin: *
access-control-expose-headers: *
content-length: 8481
date: Mon, 24 Aug 2026 14:23:57 GMT
```

### Body (complete CapabilityStatement, pretty-printed)

```json
{
  "resourceType": "CapabilityStatement",
  "url": "http://heliossoftware.com/fhir/hts/CapabilityStatement/hts",
  "version": "0.2.1",
  "name": "HeliosTerminologyServer",
  "title": "Helios Terminology Server",
  "instantiates": [
    "http://hl7.org/fhir/CapabilityStatement/terminology-server"
  ],
  "status": "active",
  "kind": "instance",
  "date": "2026-04-01",
  "fhirVersion": "4.0.1",
  "format": [
    "application/fhir+json",
    "application/fhir+xml"
  ],
  "extension": [
    {
      "url": "http://hl7.org/fhir/uv/application-feature/StructureDefinition/feature",
      "extension": [
        {
          "url": "definition",
          "valueCanonical": "http://hl7.org/fhir/uv/tx-tests/FeatureDefinition/test-version"
        },
        {
          "url": "value",
          "valueCode": "1.7.0"
        }
      ]
    },
    {
      "url": "http://hl7.org/fhir/uv/application-feature/StructureDefinition/feature",
      "extension": [
        {
          "url": "definition",
          "valueCanonical": "http://hl7.org/fhir/uv/tx-ecosystem/FeatureDefinition/CodeSystemAsParameter"
        },
        {
          "url": "value",
          "valueBoolean": true
        }
      ]
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-10"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-11"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-12"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-13"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-14"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-15"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-16"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-17"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-18"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-19"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-2"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-20"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-21"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-22"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-23"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-24"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-25"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-26"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-27"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-28"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-29"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-3"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-30"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-31"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-4"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-5"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-6"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-7"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-8"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/filler-9"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/limbs"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/source"
    },
    {
      "url": "http://hl7.org/fhir/StructureDefinition/capabilitystatement-supported-system",
      "valueUri": "http://example.org/cs/target"
    }
  ],
  "software": {
    "name": "Helios Terminology Server",
    "version": "0.2.1",
    "releaseDate": "2026-04-01"
  },
  "implementation": {
    "description": "Helios Terminology Server SQLite backend"
  },
  "rest": [
    {
      "mode": "server",
      "resource": [
        {
          "type": "CodeSystem",
          "interaction": [
            {
              "code": "read"
            },
            {
              "code": "create"
            },
            {
              "code": "update"
            },
            {
              "code": "delete"
            },
            {
              "code": "search-type"
            }
          ],
          "searchParam": [
            {
              "name": "url",
              "type": "uri",
              "documentation": "Canonical URL of the resource"
            },
            {
              "name": "version",
              "type": "token",
              "documentation": "Business version"
            },
            {
              "name": "name",
              "type": "string",
              "documentation": "Computer-friendly name"
            },
            {
              "name": "title",
              "type": "string",
              "documentation": "Human-friendly title"
            },
            {
              "name": "status",
              "type": "token",
              "documentation": "Publication status"
            }
          ],
          "operation": [
            {
              "name": "lookup",
              "definition": "http://hl7.org/fhir/OperationDefinition/CodeSystem-lookup"
            },
            {
              "name": "validate-code",
              "definition": "http://hl7.org/fhir/OperationDefinition/CodeSystem-validate-code"
            },
            {
              "name": "subsumes",
              "definition": "http://hl7.org/fhir/OperationDefinition/CodeSystem-subsumes"
            }
          ]
        },
        {
          "type": "ValueSet",
          "interaction": [
            {
              "code": "read"
            },
            {
              "code": "create"
            },
            {
              "code": "update"
            },
            {
              "code": "delete"
            },
            {
              "code": "search-type"
            }
          ],
          "searchParam": [
            {
              "name": "url",
              "type": "uri",
              "documentation": "Canonical URL of the resource"
            },
            {
              "name": "version",
              "type": "token",
              "documentation": "Business version"
            },
            {
              "name": "name",
              "type": "string",
              "documentation": "Computer-friendly name"
            },
            {
              "name": "title",
              "type": "string",
              "documentation": "Human-friendly title"
            },
            {
              "name": "status",
              "type": "token",
              "documentation": "Publication status"
            }
          ],
          "operation": [
            {
              "name": "expand",
              "definition": "http://hl7.org/fhir/OperationDefinition/ValueSet-expand"
            },
            {
              "name": "validate-code",
              "definition": "http://hl7.org/fhir/OperationDefinition/ValueSet-validate-code"
            }
          ]
        },
        {
          "type": "ConceptMap",
          "interaction": [
            {
              "code": "read"
            },
            {
              "code": "create"
            },
            {
              "code": "update"
            },
            {
              "code": "delete"
            },
            {
              "code": "search-type"
            }
          ],
          "searchParam": [
            {
              "name": "url",
              "type": "uri",
              "documentation": "Canonical URL of the resource"
            },
            {
              "name": "version",
              "type": "token",
              "documentation": "Business version"
            },
            {
              "name": "name",
              "type": "string",
              "documentation": "Computer-friendly name"
            },
            {
              "name": "title",
              "type": "string",
              "documentation": "Human-friendly title"
            },
            {
              "name": "status",
              "type": "token",
              "documentation": "Publication status"
            }
          ],
          "operation": [
            {
              "name": "translate",
              "definition": "http://hl7.org/fhir/OperationDefinition/ConceptMap-translate"
            },
            {
              "name": "closure",
              "definition": "http://hl7.org/fhir/OperationDefinition/ConceptMap-closure"
            }
          ]
        }
      ],
      "operation": [
        {
          "name": "versions",
          "definition": "http://hl7.org/fhir/OperationDefinition/Resource-versions"
        },
        {
          "name": "lookup",
          "definition": "http://hl7.org/fhir/OperationDefinition/CodeSystem-lookup"
        },
        {
          "name": "validate-code",
          "definition": "http://hl7.org/fhir/OperationDefinition/CodeSystem-validate-code"
        },
        {
          "name": "subsumes",
          "definition": "http://hl7.org/fhir/OperationDefinition/CodeSystem-subsumes"
        },
        {
          "name": "expand",
          "definition": "http://hl7.org/fhir/OperationDefinition/ValueSet-expand"
        },
        {
          "name": "translate",
          "definition": "http://hl7.org/fhir/OperationDefinition/ConceptMap-translate"
        },
        {
          "name": "closure",
          "definition": "http://hl7.org/fhir/OperationDefinition/ConceptMap-closure"
        }
      ]
    }
  ]
}
```