# fhirpath-lab server engine API
![](static/Square44x44Logo.scale-150.png "Fhirpath-lab logo")

Some fhirpath engines supported in the fhirpath-lab require server processing
to be able to evaluate expressions. This is typically for 2 reasons:

* Language/engine is not available in the browser (e.g. .NET/JAVA)
* Hosting the service seperates the engine maintenance from the fhirpath-lab itself

Currently the Firely SDK for .NET and the HAPI FHIR server/FHIR for Linux(IBM) for JAVA require a hosted service.

These hosted services are available for use by anyone, but are not guaranteed to be available, and when called by the fhirpath-lab will come in through one of these callers:

* https://fhirpath-lab.com
* https://dev.fhirpath-lab.com
* https://fhirpath-lab.azurewebsites.net
* https://fhirpath-lab-dev.azurewebsites.net/
* http://localhost:3000 - Which is where I test locally to the services

As the fhirpath-lab is a web only client, you'll need to implement CORS for the above domains to be able to call your service.

To add in another engine into the fhirpath-lab, the VueJS code will need to be updated to include the new engine:
* static/config.json - Add in the new service URL
* static/config.local.json - Add in the new service URL (for local debugging only)
* helpers/user_settings.ts - Provide the named form of the new service
* pages/FhirPath/index.vue
    - include the new engine in the `executionEngines` array (end of file)
    - update `evaluateFhirPathExpression` to support the newly selected engine (easiest to copy the HAPI portion)
    - ensure that your new engine name is included in the `trace` message that reports usage, and doesn't conflict with any other engine names<br/>*(i.e. don't report `HAPI` if you're not using the HAPI engine)*

## API Definition
The API takes a FHIR Parameters resource as the input, and returns a FHIR Parameters resource as the output.

The engine should evaluate the context expression against the test resource, and then run the expression against each item returned (If no context expression is provided, just evaluate the expression on the test resource).

### Input Parameters Resource:
| Parameter Name | Description | Type | Cardinality |
|----------------|-------------|------|:-----------:|
| `context` | The context to execute the expression against | string | 0..1 |
| `expression` | The fhirpath expression to execute | string | 1..1 |
| `validate` | Whether to validate the expression before executing it (experimental) | boolean | 0..1 |
| `variables` | The variables to pass to the expression | multi-part | 0..* |
| *(variables.part.name)* | The name of a variable to use | string | 1..1 | 
| *(variables.part.value)* | The value of the variable| *(any)* | 0..1 | 
| `resource` | The resource to execute the expression against | Resource | 1..1 |
| `terminologyServer` | The terminology server to use for terminology lookups if not natively supported | string | 0..1 |

### Output Parameters Resource:
| Parameter Name | Description | Type | Cardinality |
|----------------|-------------|------|:-----------:|
| `parameters` | The parameters that were passed in - detailed below | multi-part | 1..1 |
| `result` | The result of the expression execution - each iteration of the result is for a different context<br/>The context will be in the valueString property, and the result(s) in the part collection | string | 0..* |

#### Output Resource: `parameters` Part
| Part Name | Description | Type | Cardinality |
|-----------|-------------|------|:-----------:|
| evaluator | The name (and version) of the engine performing the evaluation | string | 1..1 |
| expectedReturnType | The expected return type of the expression (via static analysis) | string | 0..1 |
| parseDebugTree | The debug output from the parser as a JSON tree (Abstract Syntax Tree)<br/>*This is JSON embedded in the string* | string | 0..1 |
| parseDebug | The debug output from the parser (unformatted - just messages for debugging) | string | 0..1 |
| expression | The expression that was executed | string | 1..1 |
| context | The context that was used to execute the expression | string | 0..1 |
| resource | The resource that was used to execute the expression | Resource | 1..1 |
| variables | The variables that were passed to the expression | multi-part | 0..* |

#### Output Resource: `result` Part
The result part itself is also broken down into 2 parts:
* trace - `name` = `'trace'`, `valueString` = the name provided to the trace call in the expression,<br/> `part` = the trace output, with the name of those parts being the datatype also.
* result - `name` = datatype (this could be primitive types, resource type, or complex type)

> **Note:** if a value cannot be represented by a FHIR type in the parameters (such as backbone elements), then that output should be serialized into a JSON fragment and put into a string extension `http://fhir.forms-lab.com/StructureDefinition/json-value` on the part.
``` json
        {
          "extension": [
            {
              "url": "http://fhir.forms-lab.com/StructureDefinition/json-value",
              "valueString": "{\r\n  \"relationship\": [\r\n    {\r\n      \"coding\": [\r\n        {\r\n          \"system\": \"http://terminology.hl7.org/CodeSystem/v2-0131\",\r\n          \"code\": \"N\"\r\n        }\r\n      ]\r\n    }\r\n  ],\r\n  \"name\": {\r\n    \"family\": \"du Marché\",\r\n    \"_family\": {\r\n      \"extension\": [\r\n        {\r\n          \"url\": \"http://hl7.org/fhir/StructureDefinition/humanname-own-prefix\",\r\n          \"valueString\": \"VV\"\r\n        }\r\n      ]\r\n    },\r\n    \"given\": [\r\n      \"Bénédicte\"\r\n    ]\r\n  },\r\n  \"telecom\": [\r\n    {\r\n      \"system\": \"phone\",\r\n      \"value\": \"+33 (237) 998327\"\r\n    }\r\n  ],\r\n  \"address\": {\r\n    \"use\": \"home\",\r\n    \"type\": \"both\",\r\n    \"line\": [\r\n      \"534 Erewhon St\"\r\n    ],\r\n    \"city\": \"PleasantVille\",\r\n    \"district\": \"Rainbow\",\r\n    \"state\": \"Vic\",\r\n    \"postalCode\": \"3999\",\r\n    \"period\": {\r\n      \"start\": \"1974-12-25\"\r\n    }\r\n  },\r\n  \"gender\": \"female\",\r\n  \"period\": {\r\n    \"start\": \"2012\"\r\n  }\r\n}\r\n"
            }
          ],
          "name": "Patient#Contact"
        },
```


### Example Request:

``` rest
POST: [service-url]
Content-Type: application/json
```
``` json
{
  "resourceType": "Parameters",
  "parameter": [
    {
      "name": "expression",
      "valueString": "trace('trc').given.join(' ')\n.combine(family).join(', ') | %varValue"
    },
    {
      "name": "context",
      "valueString": "name"
    },
    {
      "name": "validate",
      "valueBoolean": true
    },
    {
      "name": "variables",
      "part": [
        {
          "name": "varValue",
          "valueString": "testMe"
        }
      ]
    },
    {
      "name": "resource",
      "resource": {
        "resourceType": "Patient",
        "id": "example",
        "identifier": [
          {
            "use": "usual",
            "type": {
              "coding": [
                {
                  "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                  "code": "MR"
                }
              ]
            },
            "system": "urn:oid:1.2.36.146.595.217.0.1",
            "value": "12345",
            "period": {
              "start": "2001-05-06"
            },
            "assigner": {
              "display": "Acme Healthcare"
            }
          }
        ],
        "active": true,
        "name": [
          {
            "use": "official",
            "family": "Chalmers",
            "given": [
              "Peter",
              "James"
            ]
          },
          {
            "use": "usual",
            "given": [
              "Jim"
            ]
          },
          {
            "use": "maiden",
            "family": "Windsor",
            "given": [
              "Peter",
              "James"
            ],
            "period": {
              "end": "2002"
            }
          }
        ],
        "telecom": [
          {
            "use": "home"
          },
          {
            "system": "phone",
            "value": "(03) 5555 6473",
            "use": "work",
            "rank": 1
          },
          {
            "system": "phone",
            "value": "(03) 3410 5613",
            "use": "mobile",
            "rank": 2
          },
          {
            "system": "phone",
            "value": "(03) 5555 8834",
            "use": "old",
            "period": {
              "end": "2014"
            }
          }
        ],
        "gender": "male",
        "birthDate": "1974-12-25",
        "_birthDate": {
          "extension": [
            {
              "url": "http://hl7.org/fhir/StructureDefinition/patient-birthTime",
              "valueDateTime": "1974-12-26T06:35:45+11:00"
            }
          ]
        },
        "deceasedBoolean": false,
        "address": [
          {
            "use": "home",
            "type": "both",
            "text": "534 Erewhon St PeasantVille, Rainbow, Vic  3999",
            "line": [
              "534 Erewhon St"
            ],
            "city": "PleasantVille",
            "district": "Rainbow",
            "state": "Vic",
            "postalCode": "3999",
            "period": {
              "start": "1974-12-25"
            }
          }
        ],
        "contact": [
          {
            "relationship": [
              {
                "coding": [
                  {
                    "system": "http://terminology.hl7.org/CodeSystem/v2-0131",
                    "code": "N"
                  }
                ]
              }
            ],
            "name": {
              "family": "du Marché",
              "_family": {
                "extension": [
                  {
                    "url": "http://hl7.org/fhir/StructureDefinition/humanname-own-prefix",
                    "valueString": "VV"
                  }
                ]
              },
              "given": [
                "Bénédicte"
              ]
            },
            "telecom": [
              {
                "system": "phone",
                "value": "+33 (237) 998327"
              }
            ],
            "address": {
              "use": "home",
              "type": "both",
              "line": [
                "534 Erewhon St"
              ],
              "city": "PleasantVille",
              "district": "Rainbow",
              "state": "Vic",
              "postalCode": "3999",
              "period": {
                "start": "1974-12-25"
              }
            },
            "gender": "female",
            "period": {
              "start": "2012"
            }
          }
        ],
        "managingOrganization": {
          "reference": "Organization/1"
        }
      }
    },
    {
      "name": "terminologyserver",
      "valueString": "https://sqlonfhir-r4.azurewebsites.net/fhir"
    }
  ]
}
```

### Example Response:

``` json
{
  "resourceType": "Parameters",
  "id": "fhirpath",
  "parameter": [
    {
      "name": "parameters",
      "part": [
        {
          "name": "evaluator",
          "valueString": "Firely-5.3.0 (R4B)"
        },
        {
          "name": "expectedReturnType",
          "valueString": "string[]"
        },
        {
          "name": "parseDebugTree",
          "valueString": "{\r\n  \"ExpressionType\": \"BinaryExpression\",\r\n  \"Name\": \"|\",\r\n  \"Arguments\": [\r\n    {\r\n      \"ExpressionType\": \"FunctionCallExpression\",\r\n      \"Name\": \"join\",\r\n      \"Arguments\": [\r\n        {\r\n          \"ExpressionType\": \"FunctionCallExpression\",\r\n          \"Name\": \"combine\",\r\n          \"Arguments\": [\r\n            {\r\n              \"ExpressionType\": \"FunctionCallExpression\",\r\n              \"Name\": \"join\",\r\n              \"Arguments\": [\r\n                {\r\n                  \"ExpressionType\": \"ChildExpression\",\r\n                  \"Name\": \"given\",\r\n                  \"Arguments\": [\r\n                    {\r\n                      \"ExpressionType\": \"FunctionCallExpression\",\r\n                      \"Name\": \"trace\",\r\n                      \"Arguments\": [\r\n                        {\r\n                          \"ExpressionType\": \"AxisExpression\",\r\n                          \"Name\": \"builtin.that\",\r\n                          \"ReturnType\": \"HumanName\"\r\n                        },\r\n                        {\r\n                          \"ExpressionType\": \"ConstantExpression\",\r\n                          \"Name\": \"trc\",\r\n                          \"ReturnType\": \"string\"\r\n                        }\r\n                      ],\r\n                      \"ReturnType\": \"HumanName\"\r\n                    }\r\n                  ],\r\n                  \"ReturnType\": \"string[]\"\r\n                },\r\n                {\r\n                  \"ExpressionType\": \"ConstantExpression\",\r\n                  \"Name\": \" \",\r\n                  \"ReturnType\": \"string\"\r\n                }\r\n              ],\r\n              \"ReturnType\": \"string\"\r\n            },\r\n            {\r\n              \"ExpressionType\": \"ChildExpression\",\r\n              \"Name\": \"family\",\r\n              \"Arguments\": [\r\n                {\r\n                  \"ExpressionType\": \"AxisExpression\",\r\n                  \"Name\": \"builtin.that\",\r\n                  \"ReturnType\": \"HumanName\"\r\n                }\r\n              ],\r\n              \"ReturnType\": \"string\"\r\n            }\r\n          ],\r\n          \"ReturnType\": \"string[]\"\r\n        },\r\n        {\r\n          \"ExpressionType\": \"ConstantExpression\",\r\n          \"Name\": \", \",\r\n          \"ReturnType\": \"string\"\r\n        }\r\n      ],\r\n      \"ReturnType\": \"string\"\r\n    },\r\n    {\r\n      \"ExpressionType\": \"VariableRefExpression\",\r\n      \"Name\": \"varValue\",\r\n      \"ReturnType\": \"string\"\r\n    }\r\n  ],\r\n  \"ReturnType\": \"string[]\"\r\n}"
        },
        {
          "name": "parseDebug",
          "valueString": "trace('trc') : HumanName\r\n.given : string[]\r\n.join(' ') : string\r\n.combine(family : string\r\n) : string[]\r\n.join(', ') : string\r\n|\r\n%varValue : string\r\n\r\n : string[] (op: |)\r\n"
        },
        {
          "name": "context",
          "valueString": "name"
        },
        {
          "name": "expression",
          "valueString": "trace('trc').given.join(' ')\n.combine(family).join(', ') | %varValue"
        },
        {
          "name": "resource",
          "resource": {
            "resourceType": "Patient",
            "id": "example",
            "identifier": [
              {
                "use": "usual",
                "type": {
                  "coding": [
                    {
                      "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                      "code": "MR"
                    }
                  ]
                },
                "system": "urn:oid:1.2.36.146.595.217.0.1",
                "value": "12345",
                "period": {
                  "start": "2001-05-06"
                },
                "assigner": {
                  "display": "Acme Healthcare"
                }
              }
            ],
            "active": true,
            "name": [
              {
                "use": "official",
                "family": "Chalmers",
                "given": [
                  "Peter",
                  "James"
                ]
              },
              {
                "use": "usual",
                "given": [
                  "Jim"
                ]
              },
              {
                "use": "maiden",
                "family": "Windsor",
                "given": [
                  "Peter",
                  "James"
                ],
                "period": {
                  "end": "2002"
                }
              }
            ],
            "telecom": [
              {
                "use": "home"
              },
              {
                "system": "phone",
                "value": "(03) 5555 6473",
                "use": "work",
                "rank": 1
              },
              {
                "system": "phone",
                "value": "(03) 3410 5613",
                "use": "mobile",
                "rank": 2
              },
              {
                "system": "phone",
                "value": "(03) 5555 8834",
                "use": "old",
                "period": {
                  "end": "2014"
                }
              }
            ],
            "gender": "male",
            "birthDate": "1974-12-25",
            "_birthDate": {
              "extension": [
                {
                  "url": "http://hl7.org/fhir/StructureDefinition/patient-birthTime",
                  "valueDateTime": "1974-12-25T14:35:45-05:00"
                }
              ]
            },
            "deceasedBoolean": false,
            "address": [
              {
                "use": "home",
                "type": "both",
                "text": "534 Erewhon St PeasantVille, Rainbow, Vic  3999",
                "line": [
                  "534 Erewhon St"
                ],
                "city": "PleasantVille",
                "district": "Rainbow",
                "state": "Vic",
                "postalCode": "3999",
                "period": {
                  "start": "1974-12-25"
                }
              }
            ],
            "contact": [
              {
                "relationship": [
                  {
                    "coding": [
                      {
                        "system": "http://terminology.hl7.org/CodeSystem/v2-0131",
                        "code": "N"
                      }
                    ]
                  }
                ],
                "name": {
                  "family": "du Marché",
                  "_family": {
                    "extension": [
                      {
                        "url": "http://hl7.org/fhir/StructureDefinition/humanname-own-prefix",
                        "valueString": "VV"
                      }
                    ]
                  },
                  "given": [
                    "Bénédicte"
                  ]
                },
                "telecom": [
                  {
                    "system": "phone",
                    "value": "+33 (237) 998327"
                  }
                ],
                "address": {
                  "use": "home",
                  "type": "both",
                  "line": [
                    "534 Erewhon St"
                  ],
                  "city": "PleasantVille",
                  "district": "Rainbow",
                  "state": "Vic",
                  "postalCode": "3999",
                  "period": {
                    "start": "1974-12-25"
                  }
                },
                "gender": "female",
                "period": {
                  "start": "2012"
                }
              }
            ],
            "managingOrganization": {
              "reference": "Organization/1"
            }
          }
        },
        {
          "name": "terminologyServerUrl",
          "valueString": "https://sqlonfhir-r4.azurewebsites.net/fhir"
        },
        {
          "name": "variables",
          "part": [
            {
              "name": "varValue",
              "valueString": "testMe"
            }
          ]
        }
      ]
    },
    {
      "name": "result",
      "valueString": "Patient.name[0]",
      "part": [
        {
          "name": "string",
          "valueString": "Peter James, Chalmers"
        },
        {
          "name": "string",
          "valueString": "testMe"
        },
        {
          "name": "trace",
          "valueString": "trc",
          "part": [
            {
              "name": "HumanName",
              "valueHumanName": {
                "use": "official",
                "family": "Chalmers",
                "given": [
                  "Peter",
                  "James"
                ]
              }
            }
          ]
        }
      ]
    },
    {
      "name": "result",
      "valueString": "Patient.name[1]",
      "part": [
        {
          "name": "string",
          "valueString": "Jim"
        },
        {
          "name": "string",
          "valueString": "testMe"
        },
        {
          "name": "trace",
          "valueString": "trc",
          "part": [
            {
              "name": "HumanName",
              "valueHumanName": {
                "use": "usual",
                "given": [
                  "Jim"
                ]
              }
            }
          ]
        }
      ]
    },
    {
      "name": "result",
      "valueString": "Patient.name[2]",
      "part": [
        {
          "name": "string",
          "valueString": "Peter James, Windsor"
        },
        {
          "name": "string",
          "valueString": "testMe"
        },
        {
          "name": "trace",
          "valueString": "trc",
          "part": [
            {
              "name": "HumanName",
              "valueHumanName": {
                "use": "maiden",
                "family": "Windsor",
                "given": [
                  "Peter",
                  "James"
                ],
                "period": {
                  "end": "2002"
                }
              }
            }
          ]
        }
      ]
    }
  ]
}
```
