use serde::{Deserialize, Serialize};
use crate::schema_version::SchemaVersion;

/// Comprehensive validation report for configurations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub is_valid: bool,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationWarning>,
    pub deprecated_fields: Vec<String>,
    pub missing_required_fields: Vec<String>,
}

impl ValidationReport {
    pub fn new() -> Self {
        Self {
            is_valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
            deprecated_fields: Vec::new(),
            missing_required_fields: Vec::new(),
        }
    }

    pub fn add_error(&mut self, error: ValidationError) {
        self.is_valid = false;
        self.errors.push(error);
    }

    pub fn add_warning(&mut self, warning: ValidationWarning) {
        self.warnings.push(warning);
    }

    pub fn add_deprecated_field(&mut self, field_path: String) {
        self.deprecated_fields.push(field_path);
    }

    pub fn add_missing_required_field(&mut self, field_path: String) {
        self.is_valid = false;
        self.missing_required_fields.push(field_path);
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty() || !self.missing_required_fields.is_empty()
    }

    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty() || !self.deprecated_fields.is_empty()
    }
}

impl Default for ValidationReport {
    fn default() -> Self {
        Self::new()
    }
}

/// Validation error with detailed information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub field_path: String,
    pub error_type: ValidationErrorType,
    pub message: String,
    pub suggested_fix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidationErrorType {
    MissingRequiredField,
    InvalidFieldType,
    InvalidFieldValue,
    StructureViolation,
    SchemaViolation,
}

/// Validation warning for non-critical issues
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationWarning {
    pub field_path: String,
    pub warning_type: ValidationWarningType,
    pub message: String,
    pub recommendation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidationWarningType {
    DeprecatedField,
    SuboptimalConfiguration,
    MissingOptionalField,
    PotentialIssue,
}

/// Schema definition for validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaDefinition {
    pub version: SchemaVersion,
    pub required_fields: Vec<String>,
    pub deprecated_fields: Vec<String>,
    pub field_types: std::collections::HashMap<String, FieldType>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FieldType {
    String,
    Integer,
    Float,
    Boolean,
    Array,
    Object,
    Any,
}

impl ValidationError {
    pub fn new(
        field_path: String,
        error_type: ValidationErrorType,
        message: String,
    ) -> Self {
        Self {
            field_path,
            error_type,
            message,
            suggested_fix: None,
        }
    }

    pub fn with_suggested_fix(mut self, fix: String) -> Self {
        self.suggested_fix = Some(fix);
        self
    }
}

impl ValidationWarning {
    pub fn new(
        field_path: String,
        warning_type: ValidationWarningType,
        message: String,
    ) -> Self {
        Self {
            field_path,
            warning_type,
            message,
            recommendation: None,
        }
    }

    pub fn with_recommendation(mut self, recommendation: String) -> Self {
        self.recommendation = Some(recommendation);
        self
    }
}

impl SchemaDefinition {
    pub fn new(version: SchemaVersion) -> Self {
        Self {
            version,
            required_fields: Vec::new(),
            deprecated_fields: Vec::new(),
            field_types: std::collections::HashMap::new(),
        }
    }

    pub fn add_required_field(&mut self, field_path: String, field_type: FieldType) {
        self.required_fields.push(field_path.clone());
        self.field_types.insert(field_path, field_type);
    }

    pub fn add_deprecated_field(&mut self, field_path: String) {
        self.deprecated_fields.push(field_path);
    }

    pub fn add_field_type(&mut self, field_path: String, field_type: FieldType) {
        self.field_types.insert(field_path, field_type);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_report_creation() {
        let mut report = ValidationReport::new();
        assert!(report.is_valid);
        assert!(!report.has_errors());
        assert!(!report.has_warnings());
    }

    #[test]
    fn test_validation_report_add_error() {
        let mut report = ValidationReport::new();
        let error = ValidationError::new(
            "test.field".to_string(),
            ValidationErrorType::MissingRequiredField,
            "Field is required".to_string(),
        );

        report.add_error(error);
        assert!(!report.is_valid);
        assert!(report.has_errors());
        assert_eq!(report.errors.len(), 1);
    }

    #[test]
    fn test_validation_report_add_warning() {
        let mut report = ValidationReport::new();
        let warning = ValidationWarning::new(
            "test.field".to_string(),
            ValidationWarningType::DeprecatedField,
            "Field is deprecated".to_string(),
        );

        report.add_warning(warning);
        assert!(report.is_valid); // Warnings don't affect validity
        assert!(report.has_warnings());
        assert_eq!(report.warnings.len(), 1);
    }

    #[test]
    fn test_schema_definition_creation() {
        let version = SchemaVersion::new(25, 2, 9);
        let mut schema = SchemaDefinition::new(version.clone());
        
        schema.add_required_field("image.tag".to_string(), FieldType::String);
        schema.add_deprecated_field("old.field".to_string());

        assert_eq!(schema.version, version);
        assert_eq!(schema.required_fields.len(), 1);
        assert_eq!(schema.deprecated_fields.len(), 1);
        assert!(schema.field_types.contains_key("image.tag"));
    }

    #[test]
    fn test_validation_error_with_suggested_fix() {
        let error = ValidationError::new(
            "test.field".to_string(),
            ValidationErrorType::MissingRequiredField,
            "Field is required".to_string(),
        ).with_suggested_fix("Add the field with a valid value".to_string());

        assert!(error.suggested_fix.is_some());
        assert_eq!(error.suggested_fix.unwrap(), "Add the field with a valid value");
    }
}