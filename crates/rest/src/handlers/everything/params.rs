use helios_fhir::FhirVersion;

use crate::error::RestError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EverythingParams {
    pub start: Option<String>,
    pub end: Option<String>,
    pub since: Option<String>,
    pub types: Option<Vec<String>>,
    pub count: Option<usize>,
    pub cursor: Option<String>,
}

fn is_fhir_date(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    match b.len() {
        4 => digits(0..4),
        7 => digits(0..4) && b[4] == b'-' && digits(5..7),
        10 => digits(0..4) && b[4] == b'-' && digits(5..7) && b[7] == b'-' && digits(8..10),
        _ => false,
    }
}

fn bad(param: &str, value: &str, why: &str) -> RestError {
    RestError::BadRequest {
        message: format!("Invalid value '{value}' for parameter '{param}': {why}"),
    }
}

impl EverythingParams {
    pub fn from_pairs(
        pairs: &[(String, String)],
        version: FhirVersion,
        max_page_size: usize,
    ) -> Result<Self, RestError> {
        let mut out = Self {
            start: None,
            end: None,
            since: None,
            types: None,
            count: None,
            cursor: None,
        };
        let known_types = crate::fhir_types::get_resource_type_names_for_version(version);
        let mut types: Vec<String> = Vec::new();
        let mut saw_type = false;

        for (name, value) in pairs {
            match name.as_str() {
                "start" | "end" => {
                    if !is_fhir_date(value) {
                        return Err(bad(
                            name,
                            value,
                            "expected a FHIR date (YYYY, YYYY-MM or YYYY-MM-DD)",
                        ));
                    }
                    let slot = if name == "start" {
                        &mut out.start
                    } else {
                        &mut out.end
                    };
                    *slot = Some(value.clone());
                }
                "_since" => {
                    chrono::DateTime::parse_from_rfc3339(value)
                        .map_err(|_| bad(name, value, "expected an RFC 3339 instant"))?;
                    out.since = Some(value.clone());
                }
                "_type" => {
                    saw_type = true;
                    for t in value.split(',').map(str::trim).filter(|t| !t.is_empty()) {
                        if !known_types.contains(&t) {
                            return Err(bad(name, t, "unknown resource type"));
                        }
                        if !types.iter().any(|x| x == t) {
                            types.push(t.to_string());
                        }
                    }
                }
                "_count" => {
                    let n: usize = value
                        .parse()
                        .map_err(|_| bad(name, value, "expected a positive integer"))?;
                    if n == 0 {
                        return Err(bad(name, value, "must be at least 1"));
                    }
                    out.count = Some(n.min(max_page_size));
                }
                "_cursor" => out.cursor = Some(value.clone()),
                "_format" | "_pretty" => {}
                other => {
                    return Err(RestError::BadRequest {
                        message: format!("Unknown parameter '{other}' for $everything"),
                    });
                }
            }
        }
        if saw_type {
            out.types = Some(types);
        }
        Ok(out)
    }

    /// Canonical string of the scope-defining inputs; hashed into the cursor.
    ///
    /// Binds the cursor to the tenant and FHIR version of the request that
    /// issued it: a cursor decoded under a different tenant or version would
    /// otherwise resume a walk against the wrong compartment data or the
    /// wrong version's segment/search-parameter shape.
    pub fn fingerprint_input(
        &self,
        patient_id: Option<&str>,
        tenant_id: &str,
        version: FhirVersion,
    ) -> String {
        format!(
            "pid={}|start={}|end={}|since={}|types={}|count={}|tenant={}|ver={:?}",
            patient_id.unwrap_or(""),
            self.start.as_deref().unwrap_or(""),
            self.end.as_deref().unwrap_or(""),
            self.since.as_deref().unwrap_or(""),
            self.types.as_ref().map(|t| t.join(",")).unwrap_or_default(),
            self.count.map(|c| c.to_string()).unwrap_or_default(),
            tenant_id,
            version,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn empty_input_is_all_defaults() {
        let p = EverythingParams::from_pairs(&[], FhirVersion::R4, 1000).unwrap();
        assert_eq!(
            p,
            EverythingParams {
                start: None,
                end: None,
                since: None,
                types: None,
                count: None,
                cursor: None
            }
        );
    }

    #[test]
    fn parses_every_parameter() {
        let p = EverythingParams::from_pairs(
            &pairs(&[
                ("start", "2020"),
                ("end", "2021-06-30"),
                ("_since", "2021-02-03T04:05:06Z"),
                ("_type", "Observation,Encounter"),
                ("_type", "Condition"),
                ("_count", "50"),
                ("_cursor", "abc"),
            ]),
            FhirVersion::R4,
            1000,
        )
        .unwrap();
        assert_eq!(p.start.as_deref(), Some("2020"));
        assert_eq!(p.end.as_deref(), Some("2021-06-30"));
        assert_eq!(p.since.as_deref(), Some("2021-02-03T04:05:06Z"));
        assert_eq!(
            p.types,
            Some(vec![
                "Observation".into(),
                "Encounter".into(),
                "Condition".into()
            ])
        );
        assert_eq!(p.count, Some(50));
        assert_eq!(p.cursor.as_deref(), Some("abc"));
    }

    #[test]
    fn type_list_is_deduped_preserving_order() {
        let p = EverythingParams::from_pairs(
            &pairs(&[("_type", "Encounter,Observation,Encounter")]),
            FhirVersion::R4,
            1000,
        )
        .unwrap();
        assert_eq!(
            p.types,
            Some(vec!["Encounter".into(), "Observation".into()])
        );
    }

    #[test]
    fn count_is_clamped_to_max_page_size() {
        let p = EverythingParams::from_pairs(&pairs(&[("_count", "5000")]), FhirVersion::R4, 1000)
            .unwrap();
        assert_eq!(p.count, Some(1000));
    }

    #[test]
    fn rejects_unknown_parameter() {
        let e = EverythingParams::from_pairs(&pairs(&[("_include", "x")]), FhirVersion::R4, 1000)
            .unwrap_err();
        assert!(matches!(e, RestError::BadRequest { .. }), "{e:?}");
    }

    #[test]
    fn rejects_unknown_resource_type() {
        let e = EverythingParams::from_pairs(
            &pairs(&[("_type", "Observation,Bogus")]),
            FhirVersion::R4,
            1000,
        )
        .unwrap_err();
        assert!(matches!(e, RestError::BadRequest { message } if message.contains("Bogus")));
    }

    #[test]
    fn rejects_bad_dates_and_counts() {
        for (k, v) in [
            ("start", "20-01"),
            ("end", "2020-13-01x"),
            ("_since", "2020-01-01"),
            ("_count", "0"),
            ("_count", "abc"),
        ] {
            let e =
                EverythingParams::from_pairs(&pairs(&[(k, v)]), FhirVersion::R4, 1000).unwrap_err();
            assert!(
                matches!(e, RestError::BadRequest { .. }),
                "{k}={v} should be rejected"
            );
        }
    }

    #[test]
    fn fingerprint_input_is_canonical() {
        let a = EverythingParams::from_pairs(
            &pairs(&[("_type", "Observation"), ("_count", "5")]),
            FhirVersion::R4,
            1000,
        )
        .unwrap();
        let b = EverythingParams::from_pairs(
            &pairs(&[
                ("_count", "5"),
                ("_type", "Observation"),
                ("_cursor", "zzz"),
            ]),
            FhirVersion::R4,
            1000,
        )
        .unwrap();
        assert_eq!(
            a.fingerprint_input(Some("p1"), "acme", FhirVersion::R4),
            b.fingerprint_input(Some("p1"), "acme", FhirVersion::R4)
        );
        assert_ne!(
            a.fingerprint_input(Some("p1"), "acme", FhirVersion::R4),
            a.fingerprint_input(Some("p2"), "acme", FhirVersion::R4)
        );
        assert_ne!(
            a.fingerprint_input(Some("p1"), "acme", FhirVersion::R4),
            a.fingerprint_input(None, "acme", FhirVersion::R4)
        );
        assert_ne!(
            a.fingerprint_input(Some("p1"), "acme", FhirVersion::R4),
            a.fingerprint_input(Some("p1"), "other-tenant", FhirVersion::R4),
            "a different tenant must change the fingerprint"
        );
        #[cfg(feature = "R4B")]
        assert_ne!(
            a.fingerprint_input(Some("p1"), "acme", FhirVersion::R4),
            a.fingerprint_input(Some("p1"), "acme", FhirVersion::R4B),
            "a different FHIR version must change the fingerprint"
        );
    }
}
