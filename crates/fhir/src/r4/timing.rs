use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Timing {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub modifierExtension: Option<Vec<Extension>>,
    pub event: Option<Vec<dateTime>>,
    pub repeat: Option<Element>,
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub bounds: Option<TimingBounds>,
    pub count: Option<positiveInt>,
    pub countMax: Option<positiveInt>,
    pub duration: Option<decimal>,
    pub durationMax: Option<decimal>,
    pub durationUnit: Option<code>,
    pub frequency: Option<positiveInt>,
    pub frequencyMax: Option<positiveInt>,
    pub period: Option<decimal>,
    pub periodMax: Option<decimal>,
    pub periodUnit: Option<code>,
    pub dayOfWeek: Option<Vec<code>>,
    pub timeOfDay: Option<Vec<time>>,
    pub when: Option<Vec<code>>,
    pub offset: Option<unsignedInt>,
    pub code: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimingBounds {
    Duration(Duration),
    Range(Range),
    Period(Period),
}

