//! The ten example ViewDefinitions from HL7/sql-on-fhir PR #410, the
//! resources they run over, and the rows the in-process `helios-sof` engine
//! (`sof-cli`) produces for them. Shared by the SQLite and PostgreSQL in-DB
//! runner tests, which must return exactly these rows.
//!
//! [`extra_cases`] adds hand-checked views for the corners the examples only
//! touch indirectly (a singleton intermediate, flattening `forEachOrNull`,
//! a filtered `forEachOrNull ... .first()`, and untyped string columns).

#![allow(dead_code)]

use serde_json::Value;

/// One view and its expected rows (order-insensitive).
pub struct Case {
    pub name: &'static str,
    pub view: Value,
    pub expected: Vec<Value>,
}

fn parse(s: &str) -> Value {
    serde_json::from_str(s).expect("fixture JSON")
}

/// Resources every case runs over, as `(resourceType, resource)`.
pub fn resources() -> Vec<(String, Value)> {
    [
        r#"{"resourceType":"Patient","id":"p1","gender":"female","name":[{"use":"official","family":"Smith","given":["Jane","Q"]},{"use":"nickname","given":["JJ"]}],"address":[{"use":"home","line":["1 Main St","Apt 2"],"city":"Boston","postalCode":"02101"},{"use":"work","line":["9 Elm"],"city":"Cambridge","postalCode":"02139"}],"contact":[{"name":{"family":"Smith"},"address":{"line":["5 Oak"],"city":"Salem","postalCode":"01970"}},{"name":{"family":"Doe"}}]}"#,
        r#"{"resourceType":"Patient","id":"p2","gender":"male","name":[{"use":"usual","family":"Jones","given":["Bob"]}]}"#,
        r#"{"resourceType":"Observation","id":"bp1","status":"final","code":{"coding":[{"system":"http://loinc.org","code":"85354-9"}]},"subject":{"reference":"Patient/p1"},"effectiveDateTime":"2024-01-02T10:00:00Z","component":[{"code":{"coding":[{"system":"http://loinc.org","code":"8480-6"}]},"valueQuantity":{"value":120,"unit":"mmHg","system":"http://unitsofmeasure.org","code":"mm[Hg]"}},{"code":{"coding":[{"system":"http://loinc.org","code":"8462-4"}]},"valueQuantity":{"value":80,"unit":"mmHg","system":"http://unitsofmeasure.org","code":"mm[Hg]"}}]}"#,
        r#"{"resourceType":"Observation","id":"bp2","status":"final","code":{"coding":[{"system":"http://loinc.org","code":"85354-9"}]},"subject":{"reference":"Patient/p2"},"effectiveDateTime":"2024-02-02","component":[{"code":{"coding":[{"system":"http://loinc.org","code":"8480-6"}]},"valueQuantity":{"value":135.5,"unit":"mmHg","system":"http://unitsofmeasure.org","code":"mm[Hg]"}}]}"#,
        r#"{"resourceType":"Observation","id":"hr1","status":"final","code":{"coding":[{"system":"http://loinc.org","code":"8867-4"}]},"subject":{"reference":"Patient/p1"},"valueQuantity":{"value":70}}"#,
        r#"{"resourceType":"Condition","id":"c1","subject":{"reference":"Patient/p1"},"encounter":{"reference":"Encounter/e1"},"onsetDateTime":"2023-05-01","code":{"coding":[{"system":"http://snomed.info/sct","code":"44054006"},{"system":"http://hl7.org/fhir/sid/icd-10-cm","code":"E11.9"}]},"category":[{"coding":[{"code":"problem-list-item"}]}],"clinicalStatus":{"coding":[{"code":"active"}]}}"#,
        r#"{"resourceType":"Condition","id":"c2","subject":{"reference":"Patient/p2"}}"#,
        r#"{"resourceType":"Encounter","id":"e1","status":"finished","class":{"code":"AMB"},"subject":{"reference":"Patient/p1"},"serviceProvider":{"reference":"Organization/o1"},"period":{"start":"2023-05-01T09:00:00Z","end":"2023-05-01T10:00:00Z"},"type":[{"coding":[{"system":"http://snomed.info/sct","code":"185349003"}]}],"participant":[{"individual":{"reference":"Practitioner/pr1"}}],"location":[{"location":{"reference":"Location/l1"}}],"episodeOfCare":[{"reference":"EpisodeOfCare/eoc1"}]}"#,
        r#"{"resourceType":"Encounter","id":"e2","status":"planned","class":{"code":"AMB"}}"#,
        r#"{"resourceType":"QuestionnaireResponse","id":"qr1","status":"completed","questionnaire":"http://example.org/Questionnaire/q1","subject":{"reference":"Patient/p1"},"authored":"2024-03-01","item":[{"linkId":"1","text":"Name","answer":[{"valueString":"Jane"}]},{"linkId":"2","text":"Group","item":[{"linkId":"2.1","text":"Age","answer":[{"valueInteger":42,"item":[{"linkId":"2.1.1","text":"Smoker","answer":[{"valueBoolean":false}]}]}]},{"linkId":"2.2","text":"DOB","answer":[{"valueDate":"1982-01-01"}]}]},{"linkId":"3","text":"Unanswered"}]}"#,
        r#"{"resourceType":"CodeSystem","id":"cs1","url":"http://example.org/cs1","status":"active","content":"complete","concept":[{"code":"A","concept":[{"code":"A1"},{"code":"A2","concept":[{"code":"A2a"}]}]},{"code":"B"}]}"#,
        // Not read by the PR #410 views; carries keys outside FHIR.
        r#"{"resourceType":"Basic","id":"b1","code":{"text":"x"},"zip":"02101","num":"2.1","count":42,"flag":true}"#,
    ]
    .iter()
    .map(|s| {
        let v = parse(s);
        (v["resourceType"].as_str().expect("type").to_string(), v)
    })
    .collect()
}

fn case(name: &'static str, view: &str, expected: &str) -> Case {
    Case {
        name,
        view: parse(view),
        expected: parse(expected).as_array().expect("rows").clone(),
    }
}

/// The PR #410 example views with the `sof-cli` reference rows.
pub fn pr410_cases() -> Vec<Case> {
    vec![
        case(
            "CodeSystemHierarchy",
            r#"{"resourceType":"ViewDefinition","resource":"CodeSystem","status":"active","select":[{"column":[{"path":"getResourceKey()","name":"id","description":"CodeSystem identifier"}]},{"column":[{"path":"code","name":"parent_code","description":"Code of the parent concept in the hierarchy"}],"select":[{"column":[{"path":"code","name":"code","description":"Code of the child concept"}],"forEach":"concept"}],"repeat":["concept"]}]}"#,
            r#"[{"id":"cs1","parent_code":"A","code":"A1"},{"id":"cs1","parent_code":"A","code":"A2"},{"id":"cs1","parent_code":"A2","code":"A2a"}]"#,
        ),
        case(
            "ConditionFlat",
            r#"{"resourceType":"ViewDefinition","resource":"Condition","status":"active","select":[{"column":[{"path":"getResourceKey()","name":"id"},{"path":"subject.getReferenceKey(Patient)","name":"patient_id"},{"path":"encounter.getReferenceKey(Encounter)","name":"encounter_id"},{"path":"onset.ofType(dateTime)","name":"onset_datetime","type":"dateTime"}]},{"column":[{"path":"system","name":"system","type":"uri"},{"path":"code","name":"code"}],"forEachOrNull":"code.coding"},{"column":[{"path":"code","name":"category"}],"forEachOrNull":"category.coding"},{"column":[{"path":"code","name":"clinical_status"}],"forEachOrNull":"clinicalStatus.coding"},{"column":[{"path":"code","name":"verification_status"}],"forEachOrNull":"verificationStatus.coding"}]}"#,
            r#"[{"id":"c1","patient_id":"p1","encounter_id":"e1","onset_datetime":"2023-05-01","system":"http://snomed.info/sct","code":"44054006","category":"problem-list-item","clinical_status":"active","verification_status":null},{"id":"c1","patient_id":"p1","encounter_id":"e1","onset_datetime":"2023-05-01","system":"http://hl7.org/fhir/sid/icd-10-cm","code":"E11.9","category":"problem-list-item","clinical_status":"active","verification_status":null},{"id":"c2","patient_id":"p2","encounter_id":null,"onset_datetime":null,"system":null,"code":null,"category":null,"clinical_status":null,"verification_status":null}]"#,
        ),
        case(
            "EncounterFlat",
            r#"{"resourceType":"ViewDefinition","resource":"Encounter","status":"active","select":[{"column":[{"path":"getResourceKey()","name":"id"},{"path":"status","name":"status"},{"path":"subject.getReferenceKey(Patient)","name":"patient_id"},{"path":"serviceProvider.getReferenceKey(Organization)","name":"service_org_id"},{"path":"period.start","name":"period_start"},{"path":"period.end","name":"period_end"}]},{"column":[{"path":"system","name":"type_sys"},{"path":"code","name":"type_code"}],"forEachOrNull":"type.coding"},{"column":[{"path":"individual.getReferenceKey(Practitioner)","name":"practitioner_id"}],"forEachOrNull":"participant"},{"column":[{"path":"location.getReferenceKey(Location)","name":"location_id"}],"forEachOrNull":"location"},{"column":[{"path":"getReferenceKey(EpisodeOfCare)","name":"episode_of_care_id"}],"forEachOrNull":"episodeOfCare"}]}"#,
            r#"[{"id":"e1","status":"finished","patient_id":"p1","service_org_id":"o1","period_start":"2023-05-01T09:00:00Z","period_end":"2023-05-01T10:00:00Z","type_sys":"http://snomed.info/sct","type_code":"185349003","practitioner_id":"pr1","location_id":"l1","episode_of_care_id":"eoc1"},{"id":"e2","status":"planned","patient_id":null,"service_org_id":null,"period_start":null,"period_end":null,"type_sys":null,"type_code":null,"practitioner_id":null,"location_id":null,"episode_of_care_id":null}]"#,
        ),
        case(
            "PatientAddresses",
            r#"{"resourceType":"ViewDefinition","resource":"Patient","status":"active","select":[{"column":[{"path":"getResourceKey()","name":"patient_id"}]},{"column":[{"path":"line.join('\n')","name":"street","description":"The full street address, including newlines if present."},{"path":"use","name":"use"},{"path":"city","name":"city"},{"path":"postalCode","name":"zip"}],"forEach":"address"}]}"#,
            r#"[{"patient_id":"p1","street":"1 Main St\nApt 2","use":"home","city":"Boston","zip":"02101"},{"patient_id":"p1","street":"9 Elm","use":"work","city":"Cambridge","zip":"02139"}]"#,
        ),
        case(
            "PatientAndContactAddressUnion",
            r#"{"resourceType":"ViewDefinition","resource":"Patient","status":"active","select":[{"column":[{"path":"getResourceKey()","name":"resource_id"}],"unionAll":[{"column":[{"path":"line.join('\n')","name":"street"},{"path":"city","name":"city"},{"path":"postalCode","name":"zip"},{"path":"true","name":"is_patient"}],"forEach":"address"},{"column":[{"path":"line.join('\n')","name":"street"},{"path":"city","name":"city"},{"path":"postalCode","name":"zip"},{"path":"false","name":"is_patient"}],"forEach":"contact.address"}]}]}"#,
            r#"[{"resource_id":"p1","street":"1 Main St\nApt 2","city":"Boston","zip":"02101","is_patient":true},{"resource_id":"p1","street":"9 Elm","city":"Cambridge","zip":"02139","is_patient":true},{"resource_id":"p1","street":"5 Oak","city":"Salem","zip":"01970","is_patient":false}]"#,
        ),
        case(
            "PatientDemographics",
            r#"{"resourceType":"ViewDefinition","resource":"Patient","status":"active","select":[{"column":[{"path":"getResourceKey()","name":"id"},{"path":"gender","name":"gender"}]},{"column":[{"path":"given.join(' ')","name":"given_name","description":"A single given name field with all names joined together."},{"path":"family","name":"family_name"}],"forEach":"name.where(use = 'official').first()"}]}"#,
            r#"[{"id":"p1","gender":"female","given_name":"Jane Q","family_name":"Smith"}]"#,
        ),
        case(
            "PatientNamesWithIndex",
            r#"{"resourceType":"ViewDefinition","resource":"Patient","status":"active","select":[{"column":[{"path":"getResourceKey()","name":"patient_id"}]},{"column":[{"path":"%rowIndex","name":"name_index","description":"The 0-based position of this name in the patient's name array.","type":"integer"},{"path":"use","name":"use"},{"path":"family","name":"family"},{"path":"given.join(' ')","name":"given"}],"forEach":"name"}]}"#,
            r#"[{"patient_id":"p1","name_index":0,"use":"official","family":"Smith","given":"Jane Q"},{"patient_id":"p1","name_index":1,"use":"nickname","family":null,"given":"JJ"},{"patient_id":"p2","name_index":0,"use":"usual","family":"Jones","given":"Bob"}]"#,
        ),
        case(
            "QuestionnaireResponseItems",
            r#"{"resourceType":"ViewDefinition","resource":"QuestionnaireResponse","status":"active","select":[{"column":[{"path":"getResourceKey()","name":"id","description":"Unique questionnaire response identifier"},{"path":"questionnaire","name":"questionnaire","description":"Canonical URL of the questionnaire"},{"path":"subject.getReferenceKey(Patient)","name":"patient_id","description":"Patient identifier"},{"path":"authored","name":"authored","description":"Date and time the response was authored"}]},{"column":[{"path":"linkId","name":"item_link_id","description":"Unique identifier for this item within the questionnaire"},{"path":"text","name":"item_text","description":"Question text"}],"select":[{"column":[{"path":"value.ofType(string)","name":"answer_value_string","description":"String answer value"},{"path":"value.ofType(integer)","name":"answer_value_integer","description":"Integer answer value"},{"path":"value.ofType(boolean)","name":"answer_value_boolean","description":"Boolean answer value"},{"path":"value.ofType(date)","name":"answer_value_date","description":"Date answer value"}],"forEachOrNull":"answer"}],"repeat":["item","answer.item"]}]}"#,
            r#"[{"id":"qr1","questionnaire":"http://example.org/Questionnaire/q1","patient_id":"p1","authored":"2024-03-01","item_link_id":"1","item_text":"Name","answer_value_string":"Jane","answer_value_integer":null,"answer_value_boolean":null,"answer_value_date":null},{"id":"qr1","questionnaire":"http://example.org/Questionnaire/q1","patient_id":"p1","authored":"2024-03-01","item_link_id":"2","item_text":"Group","answer_value_string":null,"answer_value_integer":null,"answer_value_boolean":null,"answer_value_date":null},{"id":"qr1","questionnaire":"http://example.org/Questionnaire/q1","patient_id":"p1","authored":"2024-03-01","item_link_id":"2.1","item_text":"Age","answer_value_string":null,"answer_value_integer":42,"answer_value_boolean":null,"answer_value_date":null},{"id":"qr1","questionnaire":"http://example.org/Questionnaire/q1","patient_id":"p1","authored":"2024-03-01","item_link_id":"2.1.1","item_text":"Smoker","answer_value_string":null,"answer_value_integer":null,"answer_value_boolean":false,"answer_value_date":null},{"id":"qr1","questionnaire":"http://example.org/Questionnaire/q1","patient_id":"p1","authored":"2024-03-01","item_link_id":"2.2","item_text":"DOB","answer_value_string":null,"answer_value_integer":null,"answer_value_boolean":null,"answer_value_date":"1982-01-01"},{"id":"qr1","questionnaire":"http://example.org/Questionnaire/q1","patient_id":"p1","authored":"2024-03-01","item_link_id":"3","item_text":"Unanswered","answer_value_string":null,"answer_value_integer":null,"answer_value_boolean":null,"answer_value_date":null}]"#,
        ),
        case(
            "ShareablePatientDemographics",
            r#"{"resourceType":"ViewDefinition","resource":"Patient","status":"active","select":[{"column":[{"path":"getResourceKey()","name":"id","type":"string"},{"path":"gender","name":"gender","type":"code"}]},{"column":[{"path":"given.join(' ')","name":"given_name","description":"A single given name field with all names joined together.","type":"string"},{"path":"family","name":"family_name","type":"string"}],"forEach":"name.where(use = 'official').first()"}]}"#,
            r#"[{"id":"p1","gender":"female","given_name":"Jane Q","family_name":"Smith"}]"#,
        ),
        case(
            "UsCoreBloodPressures",
            r#"{"resourceType":"ViewDefinition","resource":"Observation","status":"active","constant":[{"name":"systolic_bp","valueCode":"8480-6"},{"name":"diastolic_bp","valueCode":"8462-4"},{"name":"bp_code","valueCode":"85354-9"}],"select":[{"column":[{"path":"getResourceKey()","name":"id"},{"path":"subject.getReferenceKey(Patient)","name":"patient_id","description":"Can be used to join to patient tables created by other views."},{"path":"effective.ofType(dateTime)","name":"effective_date_time"}]},{"column":[{"path":"value.ofType(Quantity).system","name":"sbp_quantity_system"},{"path":"value.ofType(Quantity).code","name":"sbp_quantity_code"},{"path":"value.ofType(Quantity).unit","name":"sbp_quantity_unit"},{"path":"value.ofType(Quantity).value","name":"sbp_quantity_value"}],"forEach":"component.where(code.coding.exists(system='http://loinc.org' and code=%systolic_bp)).first()"},{"column":[{"path":"value.ofType(Quantity).system","name":"dbp_quantity_system"},{"path":"value.ofType(Quantity).code","name":"dbp_quantity_code"},{"path":"value.ofType(Quantity).unit","name":"dbp_quantity_unit"},{"path":"value.ofType(Quantity).value","name":"dbp_quantity_value"}],"forEach":"component.where(code.coding.exists(system='http://loinc.org' and code=%diastolic_bp)).first()"}],"where":[{"path":"code.coding.exists(system='http://loinc.org' and code=%bp_code)"}]}"#,
            r#"[{"id":"bp1","patient_id":"p1","effective_date_time":"2024-01-02T10:00:00Z","sbp_quantity_system":"http://unitsofmeasure.org","sbp_quantity_code":"mm[Hg]","sbp_quantity_unit":"mmHg","sbp_quantity_value":120,"dbp_quantity_system":"http://unitsofmeasure.org","dbp_quantity_code":"mm[Hg]","dbp_quantity_unit":"mmHg","dbp_quantity_value":80}]"#,
        ),
    ]
}

/// [`extra_cases`] that only apply to the in-DB runners (see each case).
pub const IN_DB_ONLY: &[&str] = &["UntypedUnknownFields"];

/// Looks a case up by name in [`pr410_cases`] and [`extra_cases`].
pub fn case_named(name: &str) -> Case {
    pr410_cases()
        .into_iter()
        .chain(extra_cases())
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no case named {name}"))
}

/// Hand-checked views over [`resources`].
pub fn extra_cases() -> Vec<Case> {
    vec![
        // `Condition.code` is a singleton object: the unnest must visit the
        // object, not iterate its members.
        case(
            "ForEachThroughSingleton",
            r#"{"resourceType":"ViewDefinition","resource":"Condition","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"forEach":"code.coding","column":[{"path":"code","name":"code"}]}]}"#,
            r#"[{"id":"c1","code":"44054006"},{"id":"c1","code":"E11.9"}]"#,
        ),
        // p1's second contact has no address: the flattened collection still
        // has one element, so no extra null row; p2 has none at all.
        case(
            "ForEachOrNullFlattens",
            r#"{"resourceType":"ViewDefinition","resource":"Patient","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"forEachOrNull":"contact.address","column":[{"path":"city","name":"city"}]}]}"#,
            r#"[{"id":"p1","city":"Salem"},{"id":"p2","city":null}]"#,
        ),
        case(
            "ForEachOrNullWhereFirst",
            r#"{"resourceType":"ViewDefinition","resource":"Patient","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"forEachOrNull":"name.where(use = 'official' or use = 'nickname')[1]","column":[{"path":"use","name":"use"}]}]}"#,
            r#"[{"id":"p1","use":"nickname"},{"id":"p2","use":null}]"#,
        ),
        // Fields outside the FHIR element tables have no type to decode by;
        // a string that reads as a number must still come back a string.
        // In-DB runners only: they read the stored JSON, while the in-process
        // engine deserializes typed resources and so never sees these keys.
        case(
            "UntypedUnknownFields",
            r#"{"resourceType":"ViewDefinition","resource":"Basic","status":"active","select":[{"column":[{"path":"id","name":"id"},{"path":"zip","name":"zip"},{"path":"num","name":"num"},{"path":"count","name":"count"},{"path":"flag","name":"flag"}]}]}"#,
            r#"[{"id":"b1","zip":"02101","num":"2.1","count":42,"flag":true}]"#,
        ),
        // `repeat` seeded from each `forEach` element: the descendants of
        // concept A only (B has none, so its row drops).
        case(
            "RepeatInsideForEach",
            r#"{"resourceType":"ViewDefinition","resource":"CodeSystem","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"forEach":"concept","select":[{"column":[{"path":"code","name":"top"}]},{"repeat":["concept"],"column":[{"path":"code","name":"descendant"}]}]}]}"#,
            r#"[{"id":"cs1","top":"A","descendant":"A1"},{"id":"cs1","top":"A","descendant":"A2"},{"id":"cs1","top":"A","descendant":"A2a"}]"#,
        ),
        // The inner `repeat` restarts from every node the outer one visits.
        case(
            "RepeatInsideRepeat",
            r#"{"resourceType":"ViewDefinition","resource":"CodeSystem","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"repeat":["concept"],"select":[{"column":[{"path":"code","name":"parent"}]},{"repeat":["concept"],"column":[{"path":"code","name":"child"}]}]}]}"#,
            r#"[{"id":"cs1","parent":"A","child":"A1"},{"id":"cs1","parent":"A","child":"A2"},{"id":"cs1","parent":"A","child":"A2a"},{"id":"cs1","parent":"A2","child":"A2a"}]"#,
        ),
        // Sibling `repeat`s multiply: 3 x 3 descendants of concept A.
        case(
            "SiblingRepeatsInsideForEach",
            r#"{"resourceType":"ViewDefinition","resource":"CodeSystem","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"forEach":"concept","select":[{"repeat":["concept"],"column":[{"path":"code","name":"a"}]},{"repeat":["concept"],"column":[{"path":"code","name":"b"}]}]}]}"#,
            r#"[{"id":"cs1","a":"A1","b":"A1"},{"id":"cs1","a":"A1","b":"A2"},{"id":"cs1","a":"A1","b":"A2a"},{"id":"cs1","a":"A2","b":"A1"},{"id":"cs1","a":"A2","b":"A2"},{"id":"cs1","a":"A2","b":"A2a"},{"id":"cs1","a":"A2a","b":"A1"},{"id":"cs1","a":"A2a","b":"A2"},{"id":"cs1","a":"A2a","b":"A2a"}]"#,
        ),
        // `unionAll` nested in a `select` under `forEach`; the branch without
        // iteration runs once per name, the other once per given.
        case(
            "UnionAllInsideForEach",
            r#"{"resourceType":"ViewDefinition","resource":"Patient","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"forEach":"name","select":[{"unionAll":[{"column":[{"path":"family","name":"value"}]},{"forEach":"given","column":[{"path":"$this","name":"value","type":"string"}]}]}]}]}"#,
            r#"[{"id":"p1","value":"Smith"},{"id":"p1","value":"Jane"},{"id":"p1","value":"Q"},{"id":"p1","value":null},{"id":"p1","value":"JJ"},{"id":"p2","value":"Jones"},{"id":"p2","value":"Bob"}]"#,
        ),
        // The descent survives a `unionAll` in the same clause: one row per
        // visited node per branch.
        case(
            "UnionAllInsideRepeat",
            r#"{"resourceType":"ViewDefinition","resource":"CodeSystem","status":"active","select":[{"repeat":["concept"],"unionAll":[{"column":[{"path":"'code'","name":"kind"},{"path":"code","name":"value"}]},{"column":[{"path":"'first-child'","name":"kind"},{"path":"concept.code.first()","name":"value"}]}]}]}"#,
            r#"[{"kind":"code","value":"A"},{"kind":"code","value":"A1"},{"kind":"code","value":"A2"},{"kind":"code","value":"A2a"},{"kind":"code","value":"B"},{"kind":"first-child","value":"A1"},{"kind":"first-child","value":null},{"kind":"first-child","value":"A2a"},{"kind":"first-child","value":null},{"kind":"first-child","value":null}]"#,
        ),
        // A `select` and a `unionAll` in one `forEach` clause: the union
        // branches multiply the sibling `select`'s rows per name.
        case(
            "SelectPlusUnionInForEach",
            r#"{"resourceType":"ViewDefinition","resource":"Patient","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"forEach":"name","select":[{"column":[{"path":"family","name":"fam"}]}],"unionAll":[{"column":[{"path":"'a'","name":"u"}]},{"forEach":"given","column":[{"path":"$this","name":"u","type":"string"}]}]}]}"#,
            r#"[{"id":"p1","fam":"Smith","u":"a"},{"id":"p1","fam":"Smith","u":"Jane"},{"id":"p1","fam":"Smith","u":"Q"},{"id":"p1","fam":null,"u":"a"},{"id":"p1","fam":null,"u":"JJ"},{"id":"p2","fam":"Jones","u":"a"},{"id":"p2","fam":"Jones","u":"Bob"}]"#,
        ),
        // A trailing-`[N]` forEach next to a `repeat` (the index filter is applied in the main query).
        case(
            "IndexedForEachSiblingRepeat",
            r#"{"resourceType":"ViewDefinition","resource":"CodeSystem","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"forEach":"concept[0]","column":[{"path":"code","name":"top"}]},{"repeat":["concept"],"column":[{"path":"code","name":"d"}]}]}"#,
            r#"[{"id":"cs1","top":"A","d":"A"},{"id":"cs1","top":"A","d":"A1"},{"id":"cs1","top":"A","d":"A2"},{"id":"cs1","top":"A","d":"A2a"},{"id":"cs1","top":"A","d":"B"}]"#,
        ),
        // A trailing-`[N]` forEach nested under a `repeat`.
        case(
            "RepeatNestedIndexedForEach",
            r#"{"resourceType":"ViewDefinition","resource":"CodeSystem","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"repeat":["concept"],"select":[{"forEach":"concept[0]","column":[{"path":"code","name":"c0"}]}]}]}"#,
            r#"[{"id":"cs1","c0":"A1"},{"id":"cs1","c0":"A2a"}]"#,
        ),
        // An EMPTY `forEachOrNull` keeps one null row through a nested `repeat`.
        case(
            "EmptyForEachOrNullNestedRepeat",
            r#"{"resourceType":"ViewDefinition","resource":"CodeSystem","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"forEachOrNull":"concept.where(code='zzz')","select":[{"column":[{"path":"code","name":"top"}]},{"repeat":["concept"],"column":[{"path":"code","name":"desc"}]}]}]}"#,
            r#"[{"id":"cs1","top":null,"desc":null}]"#,
        ),
        // ...and through a `repeat` that itself holds a `forEach`.
        case(
            "EmptyForEachOrNullRepeatForEach",
            r#"{"resourceType":"ViewDefinition","resource":"CodeSystem","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"forEachOrNull":"concept.where(code='zzz')","select":[{"column":[{"path":"code","name":"top"}]},{"repeat":["concept"],"select":[{"forEach":"concept","column":[{"path":"code","name":"c"}]}]}]}]}"#,
            r#"[{"id":"cs1","top":null,"c":null}]"#,
        ),
        // A NON-empty `forEachOrNull` element without descendants still drops its row.
        case(
            "NonEmptyForEachOrNullLeafRepeat",
            r#"{"resourceType":"ViewDefinition","resource":"CodeSystem","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"forEachOrNull":"concept.where(code='B')","select":[{"column":[{"path":"code","name":"top"}]},{"repeat":["concept"],"column":[{"path":"code","name":"desc"}]}]}]}"#,
            r#"[]"#,
        ),
        // An empty `forEachOrNull` yields ONE null row, not one per union branch.
        case(
            "EmptyForEachOrNullNestedUnionAll",
            r#"{"resourceType":"ViewDefinition","resource":"Patient","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"forEachOrNull":"contact.where(name.family='zzz')","select":[{"unionAll":[{"column":[{"path":"name.family","name":"v"}]},{"column":[{"path":"'x'","name":"v"}]}]}]}]}"#,
            r#"[{"id":"p1","v":null},{"id":"p2","v":null}]"#,
        ),
        // ...and through a plain nested `forEach` (no `repeat` involved).
        case(
            "EmptyForEachOrNullNestedForEach",
            r#"{"resourceType":"ViewDefinition","resource":"CodeSystem","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"forEachOrNull":"concept.where(code='zzz')","select":[{"column":[{"path":"code","name":"top"}]},{"forEach":"concept","column":[{"path":"code","name":"c"}]}]}]}"#,
            r#"[{"id":"cs1","top":null,"c":null}]"#,
        ),
        case(
            "EmptyForEachOrNullNestedIndexedForEach",
            r#"{"resourceType":"ViewDefinition","resource":"CodeSystem","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"forEachOrNull":"concept.where(code='zzz')","select":[{"column":[{"path":"code","name":"top"}]},{"forEach":"concept[0]","column":[{"path":"code","name":"c0"}]}]}]}"#,
            r#"[{"id":"cs1","top":null,"c0":null}]"#,
        ),
        case(
            "EmptyForEachOrNullRepeatIndexedForEach",
            r#"{"resourceType":"ViewDefinition","resource":"CodeSystem","status":"active","select":[{"column":[{"path":"id","name":"id"}]},{"forEachOrNull":"concept.where(code='zzz')","select":[{"column":[{"path":"code","name":"top"}]},{"repeat":["concept"],"select":[{"forEach":"concept[0]","column":[{"path":"code","name":"c0"}]}]}]}]}"#,
            r#"[{"id":"cs1","top":null,"c0":null}]"#,
        ),
    ]
}
