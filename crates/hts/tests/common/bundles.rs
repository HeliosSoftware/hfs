//! FHIR Bundle fixtures for each FHIR version.
#![allow(dead_code)] // items are shared across test binaries; not all are used in each
//!
//! All bundles use `http://hts.test/` URLs to avoid clashing with real
//! registries.  The anatomy CodeSystem and limbs ValueSet are shared across
//! versions; ConceptMap serialisation differs between R4/R4B and R5/R6.
//!
//! Hierarchy tested by subsumption:
//!   body
//!   ├─ limb
//!   │   ├─ arm
//!   │   └─ leg
//!   └─ head
//!
//! Translation mapping (arm → 40983000, leg → 61685007) is present in all
//! version-specific ConceptMap fixtures.

// ── Shared anatomy CodeSystem ──────────────────────────────────────────────────

pub const ANATOMY_CS_URL: &str = "http://hts.test/anatomy";
pub const LIMBS_VS_URL: &str = "http://hts.test/vs/limbs";

// ── Implicit ValueSet bundle ───────────────────────────────────────────────────

/// URL of the implicit ValueSet declared on `IMPLICIT_CS_URL` via `.valueSet`.
pub const IMPLICIT_VS_URL: &str = "http://hts.test/vs/implicit-anatomy";

/// A CodeSystem that declares an implicit ValueSet via its `valueSet` property.
/// No explicit ValueSet resource exists for `IMPLICIT_VS_URL`.
pub fn implicit_vs_bundle() -> &'static str {
    r#"{
  "resourceType": "Bundle",
  "type": "collection",
  "entry": [
    {
      "resource": {
        "resourceType": "CodeSystem",
        "id": "anatomy-implicit",
        "url": "http://hts.test/anatomy-implicit",
        "valueSet": "http://hts.test/vs/implicit-anatomy",
        "version": "1.0",
        "name": "AnatomyImplicitCS",
        "status": "active",
        "content": "complete",
        "concept": [
          { "code": "bone", "display": "Bone" },
          { "code": "muscle", "display": "Muscle" },
          { "code": "nerve", "display": "Nerve" }
        ]
      }
    }
  ]
}"#
}

/// R4 Bundle: anatomy CodeSystem + limbs ValueSet + R4-format ConceptMap.
///
/// ConceptMap uses `sourceUri`/`targetUri` and `equivalence` in targets,
/// as specified in FHIR R4 / R4B.
pub const CM_URL_R4: &str = "http://hts.test/cm/anatomy-r4";

pub fn r4_bundle() -> &'static str {
    r#"{
  "resourceType": "Bundle",
  "type": "collection",
  "entry": [
    {
      "resource": {
        "resourceType": "CodeSystem",
        "id": "anatomy",
        "url": "http://hts.test/anatomy",
        "version": "1.0",
        "name": "AnatomyCS",
        "status": "active",
        "content": "complete",
        "concept": [
          {
            "code": "body",
            "display": "Body",
            "concept": [
              {
                "code": "limb",
                "display": "Limb",
                "concept": [
                  { "code": "arm",  "display": "Arm"  },
                  { "code": "leg",  "display": "Leg"  }
                ]
              },
              { "code": "head", "display": "Head" }
            ]
          }
        ]
      }
    },
    {
      "resource": {
        "resourceType": "ValueSet",
        "id": "limbs",
        "url": "http://hts.test/vs/limbs",
        "version": "1.0",
        "name": "LimbsVS",
        "status": "active",
        "compose": {
          "include": [
            {
              "system": "http://hts.test/anatomy",
              "concept": [
                { "code": "limb" },
                { "code": "arm"  },
                { "code": "leg"  }
              ]
            }
          ]
        }
      }
    },
    {
      "resource": {
        "resourceType": "ConceptMap",
        "id": "anatomy-r4",
        "url": "http://hts.test/cm/anatomy-r4",
        "version": "1.0",
        "name": "AnatomyMapR4",
        "status": "active",
        "sourceUri": "http://hts.test/anatomy",
        "targetUri": "http://snomed.info/sct",
        "group": [
          {
            "source": "http://hts.test/anatomy",
            "target": "http://snomed.info/sct",
            "element": [
              {
                "code": "arm",
                "target": [
                  {
                    "code": "40983000",
                    "display": "Structure of upper extremity",
                    "equivalence": "equivalent"
                  }
                ]
              },
              {
                "code": "leg",
                "target": [
                  {
                    "code": "61685007",
                    "display": "Structure of lower extremity",
                    "equivalence": "equivalent"
                  }
                ]
              }
            ]
          }
        ]
      }
    }
  ]
}"#
}

/// R4B Bundle: identical CodeSystem + ValueSet; ConceptMap is the same format
/// as R4 (FHIR R4B kept the same ConceptMap structure).
pub const CM_URL_R4B: &str = "http://hts.test/cm/anatomy-r4b";

pub fn r4b_bundle() -> &'static str {
    r#"{
  "resourceType": "Bundle",
  "type": "collection",
  "entry": [
    {
      "resource": {
        "resourceType": "CodeSystem",
        "id": "anatomy-r4b",
        "url": "http://hts.test/anatomy",
        "version": "1.0",
        "name": "AnatomyCS",
        "status": "active",
        "content": "complete",
        "concept": [
          {
            "code": "body",
            "display": "Body",
            "concept": [
              {
                "code": "limb",
                "display": "Limb",
                "concept": [
                  { "code": "arm",  "display": "Arm"  },
                  { "code": "leg",  "display": "Leg"  }
                ]
              },
              { "code": "head", "display": "Head" }
            ]
          }
        ]
      }
    },
    {
      "resource": {
        "resourceType": "ValueSet",
        "id": "limbs-r4b",
        "url": "http://hts.test/vs/limbs",
        "version": "1.0",
        "name": "LimbsVS",
        "status": "active",
        "compose": {
          "include": [
            {
              "system": "http://hts.test/anatomy",
              "concept": [
                { "code": "limb" },
                { "code": "arm"  },
                { "code": "leg"  }
              ]
            }
          ]
        }
      }
    },
    {
      "resource": {
        "resourceType": "ConceptMap",
        "id": "anatomy-r4b",
        "url": "http://hts.test/cm/anatomy-r4b",
        "version": "1.0",
        "name": "AnatomyMapR4B",
        "status": "active",
        "sourceUri": "http://hts.test/anatomy",
        "targetUri": "http://snomed.info/sct",
        "group": [
          {
            "source": "http://hts.test/anatomy",
            "target": "http://snomed.info/sct",
            "element": [
              {
                "code": "arm",
                "target": [
                  {
                    "code": "40983000",
                    "display": "Structure of upper extremity",
                    "equivalence": "equivalent"
                  }
                ]
              },
              {
                "code": "leg",
                "target": [
                  {
                    "code": "61685007",
                    "display": "Structure of lower extremity",
                    "equivalence": "equivalent"
                  }
                ]
              }
            ]
          }
        ]
      }
    }
  ]
}"#
}

/// R5 Bundle: uses `sourceScope`/`targetScope` and `relationship` in targets
/// as specified in FHIR R5.
pub const CM_URL_R5: &str = "http://hts.test/cm/anatomy-r5";

pub fn r5_bundle() -> &'static str {
    r#"{
  "resourceType": "Bundle",
  "type": "collection",
  "entry": [
    {
      "resource": {
        "resourceType": "CodeSystem",
        "id": "anatomy-r5",
        "url": "http://hts.test/anatomy",
        "version": "1.0",
        "name": "AnatomyCS",
        "status": "active",
        "content": "complete",
        "concept": [
          {
            "code": "body",
            "display": "Body",
            "concept": [
              {
                "code": "limb",
                "display": "Limb",
                "concept": [
                  { "code": "arm",  "display": "Arm"  },
                  { "code": "leg",  "display": "Leg"  }
                ]
              },
              { "code": "head", "display": "Head" }
            ]
          }
        ]
      }
    },
    {
      "resource": {
        "resourceType": "ValueSet",
        "id": "limbs-r5",
        "url": "http://hts.test/vs/limbs",
        "version": "1.0",
        "name": "LimbsVS",
        "status": "active",
        "compose": {
          "include": [
            {
              "system": "http://hts.test/anatomy",
              "concept": [
                { "code": "limb" },
                { "code": "arm"  },
                { "code": "leg"  }
              ]
            }
          ]
        }
      }
    },
    {
      "resource": {
        "resourceType": "ConceptMap",
        "id": "anatomy-r5",
        "url": "http://hts.test/cm/anatomy-r5",
        "version": "1.0",
        "name": "AnatomyMapR5",
        "status": "active",
        "sourceScope": "http://hts.test/anatomy",
        "targetScope": "http://snomed.info/sct",
        "group": [
          {
            "source": "http://hts.test/anatomy",
            "target": "http://snomed.info/sct",
            "element": [
              {
                "code": "arm",
                "target": [
                  {
                    "code": "40983000",
                    "display": "Structure of upper extremity",
                    "relationship": "equivalent"
                  }
                ]
              },
              {
                "code": "leg",
                "target": [
                  {
                    "code": "61685007",
                    "display": "Structure of lower extremity",
                    "relationship": "equivalent"
                  }
                ]
              }
            ]
          }
        ]
      }
    }
  ]
}"#
}

/// R6 Bundle: same ConceptMap structure as R5 (FHIR R6 retained the same
/// `relationship` field and `sourceScope`/`targetScope` layout).
pub const CM_URL_R6: &str = "http://hts.test/cm/anatomy-r6";

pub fn r6_bundle() -> &'static str {
    r#"{
  "resourceType": "Bundle",
  "type": "collection",
  "entry": [
    {
      "resource": {
        "resourceType": "CodeSystem",
        "id": "anatomy-r6",
        "url": "http://hts.test/anatomy",
        "version": "1.0",
        "name": "AnatomyCS",
        "status": "active",
        "content": "complete",
        "concept": [
          {
            "code": "body",
            "display": "Body",
            "concept": [
              {
                "code": "limb",
                "display": "Limb",
                "concept": [
                  { "code": "arm",  "display": "Arm"  },
                  { "code": "leg",  "display": "Leg"  }
                ]
              },
              { "code": "head", "display": "Head" }
            ]
          }
        ]
      }
    },
    {
      "resource": {
        "resourceType": "ValueSet",
        "id": "limbs-r6",
        "url": "http://hts.test/vs/limbs",
        "version": "1.0",
        "name": "LimbsVS",
        "status": "active",
        "compose": {
          "include": [
            {
              "system": "http://hts.test/anatomy",
              "concept": [
                { "code": "limb" },
                { "code": "arm"  },
                { "code": "leg"  }
              ]
            }
          ]
        }
      }
    },
    {
      "resource": {
        "resourceType": "ConceptMap",
        "id": "anatomy-r6",
        "url": "http://hts.test/cm/anatomy-r6",
        "version": "1.0",
        "name": "AnatomyMapR6",
        "status": "active",
        "sourceScope": "http://hts.test/anatomy",
        "targetScope": "http://snomed.info/sct",
        "group": [
          {
            "source": "http://hts.test/anatomy",
            "target": "http://snomed.info/sct",
            "element": [
              {
                "code": "arm",
                "target": [
                  {
                    "code": "40983000",
                    "display": "Structure of upper extremity",
                    "relationship": "equivalent"
                  }
                ]
              },
              {
                "code": "leg",
                "target": [
                  {
                    "code": "61685007",
                    "display": "Structure of lower extremity",
                    "relationship": "equivalent"
                  }
                ]
              }
            ]
          }
        ]
      }
    }
  ]
}"#
}
