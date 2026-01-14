use serde::{Deserialize, Serialize};
use crate::{
    schema_version::SchemaVersion,
    transformation_rule::{AppliedTransformation, FieldChange},
    validation::ValidationReport,
};

/// Reporter for generating transformation reports in various formats
pub struct TransformationReporter {
    output_format: ReportFormat,
}

/// Available output formats for transformation reports
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReportFormat {
    Console,
    Json,
    Yaml,
    Html,
}

/// Comprehensive transformation report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformationReport {
    pub source_version: Option<SchemaVersion>,
    pub target_version: SchemaVersion,
    pub applied_transformations: Vec<AppliedTransformation>,
    pub field_changes: Vec<FieldChange>,
    pub removed_fields: Vec<String>,
    pub added_fields: Vec<String>,
    pub validation_summary: ValidationSummary,
    pub recommendations: Vec<String>,
    pub transformation_summary: TransformationSummary,
}

/// Summary of validation results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationSummary {
    pub total_errors: usize,
    pub total_warnings: usize,
    pub deprecated_fields_count: usize,
    pub missing_required_fields_count: usize,
    pub is_valid: bool,
}

/// Summary of transformation results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformationSummary {
    pub total_transformations: usize,
    pub successful_transformations: usize,
    pub skipped_transformations: usize,
    pub fields_moved: usize,
    pub fields_copied: usize,
    pub fields_removed: usize,
    pub fields_transformed: usize,
}

impl TransformationReporter {
    pub fn new() -> Self {
        Self {
            output_format: ReportFormat::Console,
        }
    }

    pub fn with_format(mut self, format: ReportFormat) -> Self {
        self.output_format = format;
        self
    }

    /// Generate a comprehensive transformation report
    pub fn generate_report(
        &self,
        source_version: Option<SchemaVersion>,
        target_version: SchemaVersion,
        applied_transformations: Vec<AppliedTransformation>,
        validation_report: ValidationReport,
    ) -> TransformationReport {
        
        let field_changes = self.extract_field_changes(&applied_transformations);
        let (removed_fields, added_fields) = self.categorize_field_changes(&field_changes);
        let validation_summary = self.create_validation_summary(&validation_report);
        let transformation_summary = self.create_transformation_summary(&applied_transformations);
        let recommendations = self.generate_recommendations(&validation_report, &applied_transformations);

        TransformationReport {
            source_version,
            target_version,
            applied_transformations,
            field_changes,
            removed_fields,
            added_fields,
            validation_summary,
            recommendations,
            transformation_summary,
        }
    }

    /// Format the report according to the configured output format
    pub fn format_report(&self, report: &TransformationReport) -> Result<String, ReportError> {
        match self.output_format {
            ReportFormat::Console => self.format_console_report(report),
            ReportFormat::Json => self.format_json_report(report),
            ReportFormat::Yaml => self.format_yaml_report(report),
            ReportFormat::Html => self.format_html_report(report),
        }
    }

    /// Extract field changes from applied transformations
    fn extract_field_changes(&self, transformations: &[AppliedTransformation]) -> Vec<FieldChange> {
        let mut changes = Vec::new();
        
        for transformation in transformations {
            let change = FieldChange {
                path: transformation.target_path.clone(),
                change_type: match &transformation.transformation_type {
                    crate::transformation_rule::TransformationType::Move => crate::transformation_rule::ChangeType::Moved,
                    crate::transformation_rule::TransformationType::Copy => crate::transformation_rule::ChangeType::Added,
                    crate::transformation_rule::TransformationType::Remove => crate::transformation_rule::ChangeType::Removed,
                    crate::transformation_rule::TransformationType::Transform(_) => crate::transformation_rule::ChangeType::Modified,
                    crate::transformation_rule::TransformationType::Merge(_) => crate::transformation_rule::ChangeType::Merged,
                    crate::transformation_rule::TransformationType::Split(_) => crate::transformation_rule::ChangeType::Split,
                },
                old_value: transformation.old_value.clone(),
                new_value: transformation.new_value.clone(),
                reason: format!("Applied rule: {}", transformation.rule_id),
            };
            changes.push(change);
        }
        
        changes
    }

    /// Categorize field changes into removed and added fields
    fn categorize_field_changes(&self, changes: &[FieldChange]) -> (Vec<String>, Vec<String>) {
        let mut removed = Vec::new();
        let mut added = Vec::new();

        for change in changes {
            match change.change_type {
                crate::transformation_rule::ChangeType::Removed => removed.push(change.path.clone()),
                crate::transformation_rule::ChangeType::Added => added.push(change.path.clone()),
                crate::transformation_rule::ChangeType::Moved => {
                    // For moved fields, we don't add to removed/added as it's a relocation
                }
                _ => {}
            }
        }

        (removed, added)
    }

    /// Create validation summary from validation report
    fn create_validation_summary(&self, report: &ValidationReport) -> ValidationSummary {
        ValidationSummary {
            total_errors: report.errors.len(),
            total_warnings: report.warnings.len(),
            deprecated_fields_count: report.deprecated_fields.len(),
            missing_required_fields_count: report.missing_required_fields.len(),
            is_valid: report.is_valid,
        }
    }

    /// Create transformation summary from applied transformations
    fn create_transformation_summary(&self, transformations: &[AppliedTransformation]) -> TransformationSummary {
        let mut summary = TransformationSummary {
            total_transformations: transformations.len(),
            successful_transformations: transformations.len(), // All applied transformations are successful
            skipped_transformations: 0, // Would need additional data to track skipped
            fields_moved: 0,
            fields_copied: 0,
            fields_removed: 0,
            fields_transformed: 0,
        };

        for transformation in transformations {
            match transformation.transformation_type {
                crate::transformation_rule::TransformationType::Move => summary.fields_moved += 1,
                crate::transformation_rule::TransformationType::Copy => summary.fields_copied += 1,
                crate::transformation_rule::TransformationType::Remove => summary.fields_removed += 1,
                crate::transformation_rule::TransformationType::Transform(_) => summary.fields_transformed += 1,
                _ => {} // Handle other types as needed
            }
        }

        summary
    }

    /// Generate recommendations based on validation and transformation results
    fn generate_recommendations(
        &self,
        validation_report: &ValidationReport,
        _transformations: &[AppliedTransformation],
    ) -> Vec<String> {
        let mut recommendations = Vec::new();

        if !validation_report.missing_required_fields.is_empty() {
            recommendations.push(
                "Review and add missing required fields to ensure proper configuration".to_string()
            );
        }

        if !validation_report.deprecated_fields.is_empty() {
            recommendations.push(
                "Consider removing deprecated fields to prepare for future schema versions".to_string()
            );
        }

        if validation_report.errors.len() > 0 {
            recommendations.push(
                "Address validation errors before deploying the configuration".to_string()
            );
        }

        if recommendations.is_empty() {
            recommendations.push("Configuration transformation completed successfully".to_string());
        }

        recommendations
    }

    /// Format report for console output
    fn format_console_report(&self, report: &TransformationReport) -> Result<String, ReportError> {
        let mut output = String::new();
        
        output.push_str("=== Schema Transformation Report ===\n\n");
        
        if let Some(ref source) = report.source_version {
            output.push_str(&format!("Source Version: {}\n", source));
        } else {
            output.push_str("Source Version: Unknown\n");
        }
        output.push_str(&format!("Target Version: {}\n\n", report.target_version));
        
        output.push_str(&format!("Transformations Applied: {}\n", report.transformation_summary.total_transformations));
        output.push_str(&format!("Validation Status: {}\n", if report.validation_summary.is_valid { "VALID" } else { "INVALID" }));
        
        if !report.recommendations.is_empty() {
            output.push_str("\nRecommendations:\n");
            for rec in &report.recommendations {
                output.push_str(&format!("  • {}\n", rec));
            }
        }
        
        Ok(output)
    }

    /// Format report as JSON
    fn format_json_report(&self, report: &TransformationReport) -> Result<String, ReportError> {
        serde_json::to_string_pretty(report)
            .map_err(|e| ReportError::SerializationError(e.to_string()))
    }

    /// Format report as YAML
    fn format_yaml_report(&self, report: &TransformationReport) -> Result<String, ReportError> {
        serde_yaml::to_string(report)
            .map_err(|e| ReportError::SerializationError(e.to_string()))
    }

    /// Format report as HTML
    fn format_html_report(&self, report: &TransformationReport) -> Result<String, ReportError> {
        // Basic HTML formatting - could be enhanced with templates
        let mut html = String::new();
        html.push_str("<!DOCTYPE html><html><head><title>Transformation Report</title></head><body>");
        html.push_str("<h1>Schema Transformation Report</h1>");
        
        if let Some(ref source) = report.source_version {
            html.push_str(&format!("<p><strong>Source Version:</strong> {}</p>", source));
        }
        html.push_str(&format!("<p><strong>Target Version:</strong> {}</p>", report.target_version));
        html.push_str(&format!("<p><strong>Transformations:</strong> {}</p>", report.transformation_summary.total_transformations));
        
        html.push_str("</body></html>");
        Ok(html)
    }
}

impl Default for TransformationReporter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("Serialization error: {0}")]
    SerializationError(String),
    
    #[error("Format error: {0}")]
    FormatError(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::ValidationReport;

    #[test]
    fn test_transformation_reporter_creation() {
        let reporter = TransformationReporter::new();
        assert!(matches!(reporter.output_format, ReportFormat::Console));
    }

    #[test]
    fn test_reporter_with_format() {
        let reporter = TransformationReporter::new().with_format(ReportFormat::Json);
        assert!(matches!(reporter.output_format, ReportFormat::Json));
    }

    #[test]
    fn test_generate_report() {
        let reporter = TransformationReporter::new();
        let source_version = Some(SchemaVersion::new(5, 0, 10));
        let target_version = SchemaVersion::new(25, 2, 9);
        let transformations = Vec::new();
        let validation_report = ValidationReport::new();

        let report = reporter.generate_report(
            source_version.clone(),
            target_version.clone(),
            transformations,
            validation_report,
        );

        assert_eq!(report.source_version, source_version);
        assert_eq!(report.target_version, target_version);
        assert!(report.validation_summary.is_valid);
    }

    #[test]
    fn test_format_console_report() {
        let reporter = TransformationReporter::new();
        let report = TransformationReport {
            source_version: Some(SchemaVersion::new(5, 0, 10)),
            target_version: SchemaVersion::new(25, 2, 9),
            applied_transformations: Vec::new(),
            field_changes: Vec::new(),
            removed_fields: Vec::new(),
            added_fields: Vec::new(),
            validation_summary: ValidationSummary {
                total_errors: 0,
                total_warnings: 0,
                deprecated_fields_count: 0,
                missing_required_fields_count: 0,
                is_valid: true,
            },
            recommendations: vec!["Test recommendation".to_string()],
            transformation_summary: TransformationSummary {
                total_transformations: 0,
                successful_transformations: 0,
                skipped_transformations: 0,
                fields_moved: 0,
                fields_copied: 0,
                fields_removed: 0,
                fields_transformed: 0,
            },
        };

        let formatted = reporter.format_console_report(&report).unwrap();
        assert!(formatted.contains("Schema Transformation Report"));
        assert!(formatted.contains("5.0.10"));
        assert!(formatted.contains("25.2.9"));
        assert!(formatted.contains("Test recommendation"));
    }
}