//! OperationOutcome response generation.
//!
//! Provides utilities for building FHIR OperationOutcome responses.

use serde_json::Value;

/// Issue severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueSeverity {
    /// Fatal error - processing cannot continue.
    Fatal,
    /// Error - processing has failed.
    Error,
    /// Warning - processing succeeded but with concerns.
    Warning,
    /// Information - informational message.
    Information,
}

impl IssueSeverity {
    /// Returns the FHIR string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueSeverity::Fatal => "fatal",
            IssueSeverity::Error => "error",
            IssueSeverity::Warning => "warning",
            IssueSeverity::Information => "information",
        }
    }
}

/// Issue type codes from FHIR value set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueType {
    /// Invalid content.
    Invalid,
    /// Structural issue.
    Structure,
    /// Required element missing.
    Required,
    /// Value out of range.
    Value,
    /// FHIRPath invariant violation.
    Invariant,
    /// Invalid code / failed terminology binding.
    CodeInvalid,
    /// Resource not found.
    NotFound,
    /// Resource was deleted.
    Deleted,
    /// Multiple matches (ambiguous).
    MultipleMatches,
    /// Conflict with existing state.
    Conflict,
    /// Lock error.
    LockError,
    /// Not supported.
    NotSupported,
    /// Duplicate resource.
    Duplicate,
    /// Processing error.
    Processing,
    /// Transient error.
    Transient,
    /// Security error.
    Security,
    /// Login required.
    Login,
    /// Unknown error.
    Unknown,
    /// Informational message.
    Informational,
    /// Success message.
    Success,
}

impl IssueType {
    /// Returns the FHIR code string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueType::Invalid => "invalid",
            IssueType::Structure => "structure",
            IssueType::Required => "required",
            IssueType::Value => "value",
            IssueType::Invariant => "invariant",
            IssueType::CodeInvalid => "code-invalid",
            IssueType::NotFound => "not-found",
            IssueType::Deleted => "deleted",
            IssueType::MultipleMatches => "multiple-matches",
            IssueType::Conflict => "conflict",
            IssueType::LockError => "lock-error",
            IssueType::NotSupported => "not-supported",
            IssueType::Duplicate => "duplicate",
            IssueType::Processing => "processing",
            IssueType::Transient => "transient",
            IssueType::Security => "security",
            IssueType::Login => "login",
            IssueType::Unknown => "unknown",
            IssueType::Informational => "informational",
            IssueType::Success => "success",
        }
    }
}

/// An issue in an OperationOutcome.
#[derive(Debug, Clone)]
pub struct Issue {
    /// The severity of the issue.
    pub severity: IssueSeverity,
    /// The type/code of the issue.
    pub code: IssueType,
    /// Human-readable description.
    pub details: String,
    /// FHIRPath expression for location.
    pub expression: Option<String>,
}

impl Issue {
    /// Creates a new issue.
    pub fn new(severity: IssueSeverity, code: IssueType, details: impl Into<String>) -> Self {
        Self {
            severity,
            code,
            details: details.into(),
            expression: None,
        }
    }

    /// Creates an error issue.
    pub fn error(code: IssueType, details: impl Into<String>) -> Self {
        Self::new(IssueSeverity::Error, code, details)
    }

    /// Creates a warning issue.
    pub fn warning(code: IssueType, details: impl Into<String>) -> Self {
        Self::new(IssueSeverity::Warning, code, details)
    }

    /// Creates an information issue.
    pub fn information(code: IssueType, details: impl Into<String>) -> Self {
        Self::new(IssueSeverity::Information, code, details)
    }

    /// Sets the expression (location).
    pub fn with_expression(mut self, expression: impl Into<String>) -> Self {
        self.expression = Some(expression.into());
        self
    }

    /// Converts to FHIR JSON.
    pub fn to_json(&self) -> Value {
        let mut issue = serde_json::json!({
            "severity": self.severity.as_str(),
            "code": self.code.as_str(),
            "details": {
                "text": self.details
            }
        });

        if let Some(expr) = &self.expression {
            issue["expression"] = serde_json::json!([expr]);
        }

        issue
    }
}

/// Builder for OperationOutcome resources.
#[derive(Debug, Default)]
pub struct OperationOutcomeBuilder {
    issues: Vec<Issue>,
}

impl OperationOutcomeBuilder {
    /// Creates a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an issue.
    pub fn add_issue(mut self, issue: Issue) -> Self {
        self.issues.push(issue);
        self
    }

    /// Adds an error issue.
    pub fn error(self, code: IssueType, details: impl Into<String>) -> Self {
        self.add_issue(Issue::error(code, details))
    }

    /// Adds a warning issue.
    pub fn warning(self, code: IssueType, details: impl Into<String>) -> Self {
        self.add_issue(Issue::warning(code, details))
    }

    /// Adds an information issue.
    pub fn information(self, code: IssueType, details: impl Into<String>) -> Self {
        self.add_issue(Issue::information(code, details))
    }

    /// Builds the OperationOutcome resource.
    pub fn build(self) -> Value {
        let issues: Vec<Value> = self.issues.iter().map(|i| i.to_json()).collect();

        serde_json::json!({
            "resourceType": "OperationOutcome",
            "issue": issues
        })
    }

    /// Returns true if there are any error or fatal issues.
    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|i| matches!(i.severity, IssueSeverity::Error | IssueSeverity::Fatal))
    }

    /// Returns true if there are any issues.
    pub fn has_issues(&self) -> bool {
        !self.issues.is_empty()
    }
}

/// Creates a simple error OperationOutcome.
pub fn error_outcome(code: IssueType, message: &str) -> Value {
    OperationOutcomeBuilder::new().error(code, message).build()
}

/// Creates a simple success OperationOutcome.
pub fn success_outcome(message: &str) -> Value {
    OperationOutcomeBuilder::new()
        .information(IssueType::Success, message)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_issue_to_json() {
        let issue = Issue::error(IssueType::NotFound, "Resource not found");
        let json = issue.to_json();

        assert_eq!(json["severity"], "error");
        assert_eq!(json["code"], "not-found");
        assert_eq!(json["details"]["text"], "Resource not found");
    }

    #[test]
    fn test_issue_with_expression() {
        let issue =
            Issue::error(IssueType::Required, "Name is required").with_expression("Patient.name");
        let json = issue.to_json();

        assert_eq!(json["expression"][0], "Patient.name");
    }

    #[test]
    fn test_builder() {
        let outcome = OperationOutcomeBuilder::new()
            .error(IssueType::Invalid, "Invalid resource")
            .warning(IssueType::Processing, "Resource processed with warnings")
            .build();

        assert_eq!(outcome["resourceType"], "OperationOutcome");
        assert_eq!(outcome["issue"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_error_outcome() {
        let outcome = error_outcome(IssueType::NotFound, "Not found");
        assert_eq!(outcome["issue"][0]["severity"], "error");
    }

    #[test]
    fn test_success_outcome() {
        let outcome = success_outcome("Operation completed");
        assert_eq!(outcome["issue"][0]["severity"], "information");
        assert_eq!(outcome["issue"][0]["code"], "success");
    }
}
