use serde_yaml::Value;
use thiserror::Error;
use crate::{
    schema_version::SchemaVersion,
    transformation_rule::{AppliedTransformation, TransformationRule},
    validation::ValidationReport,
    schema_registry::SchemaRegistry,
    reporter::TransformationReporter,
};

/// Main transformation engine that orchestrates the schema transformation process
pub struct SchemaTransformationEngine {
    registry: SchemaRegistry,
    reporter: TransformationReporter,
}

/// Result of a transformation operation
#[derive(Debug, Clone)]
pub struct TransformationResult {
    pub transformed_config: Value,
    pub applied_transformations: Vec<AppliedTransformation>,
    pub validation_report: ValidationReport,
    pub warnings: Vec<TransformationWarning>,
    pub source_version: Option<SchemaVersion>,
    pub target_version: SchemaVersion,
}

/// Warning generated during transformation
#[derive(Debug, Clone)]
pub struct TransformationWarning {
    pub message: String,
    pub field_path: Option<String>,
    pub warning_type: TransformationWarningType,
}

#[derive(Debug, Clone)]
pub enum TransformationWarningType {
    PartialTransformation,
    ConditionalSkipped,
    ValueNotTransformed,
    DeprecatedFieldFound,
}

/// Errors that can occur during transformation
#[derive(Debug, Error)]
pub enum TransformationError {
    #[error("Schema version detection failed: {0}")]
    VersionDetectionFailed(String),

    #[error("No migration path found from {0} to {1}")]
    NoMigrationPath(String, String),

    #[error("Transformation rule failed: {0} - {1}")]
    RuleApplicationFailed(String, String),

    #[error("Configuration validation failed")]
    ValidationFailed(ValidationReport),

    #[error("Schema registry error: {0}")]
    RegistryError(String),

    #[error("YAML parsing error: {0}")]
    YamlError(#[from] serde_yaml::Error),
}

impl From<crate::schema_registry::RegistryError> for TransformationError {
    fn from(err: crate::schema_registry::RegistryError) -> Self {
        TransformationError::RegistryError(err.to_string())
    }
}

impl SchemaTransformationEngine {
    pub fn new(registry: SchemaRegistry) -> Self {
        Self {
            registry,
            reporter: TransformationReporter::new(),
        }
    }

    /// Transform configuration to the latest schema version
    pub fn transform(&mut self, config: Value) -> Result<TransformationResult, TransformationError> {
        let latest_version = self.registry.get_latest_version()
            .ok_or_else(|| TransformationError::RegistryError("No schema versions available".to_string()))?;
        
        self.transform_with_target_version(config, latest_version)
    }

    /// Transform configuration to a specific target version
    pub fn transform_with_target_version(
        &mut self, 
        mut config: Value, 
        target_version: SchemaVersion
    ) -> Result<TransformationResult, TransformationError> {
        
        // Detect source version
        let source_version = self.detect_version(&config)?;
        
        // If already at target version, just validate
        if let Some(ref source) = source_version {
            if source == &target_version {
                let validation_report = self.registry.validate_configuration(&config, &target_version)?;
                return Ok(TransformationResult {
                    transformed_config: config,
                    applied_transformations: Vec::new(),
                    validation_report,
                    warnings: Vec::new(),
                    source_version,
                    target_version,
                });
            }
        }

        // Get migration path
        let migration_path = if let Some(ref source) = source_version {
            self.resolve_migration_path(source.clone(), target_version.clone())?
        } else {
            // If we can't detect source version, try direct transformation to target
            vec![target_version.clone()]
        };

        // Apply transformations along the migration path
        let mut applied_transformations = Vec::new();
        let mut warnings = Vec::new();
        let mut current_version = source_version.clone();

        for target in migration_path {
            if let Some(ref current) = current_version {
                let rules = self.registry.get_transformation_rules(current, &target)?;
                let (new_transformations, new_warnings) = self.apply_transformation_rules(&mut config, &rules)?;
                applied_transformations.extend(new_transformations);
                warnings.extend(new_warnings);
            }
            current_version = Some(target);
        }

        // Final validation
        let validation_report = self.registry.validate_configuration(&config, &target_version)?;

        Ok(TransformationResult {
            transformed_config: config,
            applied_transformations,
            validation_report,
            warnings,
            source_version,
            target_version,
        })
    }

    /// Detect the schema version of the input configuration
    fn detect_version(&self, _config: &Value) -> Result<Option<SchemaVersion>, TransformationError> {
        // This is a placeholder - actual implementation would use pattern matching
        // For now, we'll return None to indicate unknown version
        Ok(None)
    }

    /// Resolve the migration path from source to target version
    fn resolve_migration_path(
        &self, 
        source: SchemaVersion, 
        target: SchemaVersion
    ) -> Result<Vec<SchemaVersion>, TransformationError> {
        
        if source == target {
            return Ok(vec![target]);
        }

        // For now, implement direct migration
        // In a full implementation, this would calculate the optimal path
        Ok(vec![target])
    }

    /// Apply a set of transformation rules to the configuration
    fn apply_transformation_rules(
        &mut self,
        config: &mut Value,
        rules: &[TransformationRule],
    ) -> Result<(Vec<AppliedTransformation>, Vec<TransformationWarning>), TransformationError> {
        
        let mut applied = Vec::new();
        let mut warnings = Vec::new();

        // Sort rules by priority (higher priority first)
        let mut sorted_rules = rules.to_vec();
        sorted_rules.sort_by(|a, b| b.priority.cmp(&a.priority));

        for rule in sorted_rules {
            if rule.condition_satisfied(config) {
                match self.apply_single_rule(config, &rule) {
                    Ok(Some(transformation)) => {
                        applied.push(transformation);
                    }
                    Ok(None) => {
                        // Rule was skipped (e.g., field not found)
                        warnings.push(TransformationWarning {
                            message: format!("Rule {} was skipped", rule.rule_id),
                            field_path: Some(rule.source_path.clone()),
                            warning_type: TransformationWarningType::ConditionalSkipped,
                        });
                    }
                    Err(e) => {
                        return Err(TransformationError::RuleApplicationFailed(
                            rule.rule_id.clone(),
                            e.to_string(),
                        ));
                    }
                }
            }
        }

        Ok((applied, warnings))
    }

    /// Apply a single transformation rule
    fn apply_single_rule(
        &self,
        _config: &mut Value,
        _rule: &TransformationRule,
    ) -> Result<Option<AppliedTransformation>, Box<dyn std::error::Error>> {
        
        // This is a placeholder implementation
        // In the full implementation, this would handle all transformation types
        Ok(None)
    }
}

impl TransformationWarning {
    pub fn new(message: String, warning_type: TransformationWarningType) -> Self {
        Self {
            message,
            field_path: None,
            warning_type,
        }
    }

    pub fn with_field_path(mut self, field_path: String) -> Self {
        self.field_path = Some(field_path);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transformation_engine_creation() {
        let registry = SchemaRegistry::new();
        let engine = SchemaTransformationEngine::new(registry);
        // Basic creation test - more comprehensive tests would require mock data
    }

    #[test]
    fn test_transformation_warning_creation() {
        let warning = TransformationWarning::new(
            "Test warning".to_string(),
            TransformationWarningType::PartialTransformation,
        ).with_field_path("test.field".to_string());

        assert_eq!(warning.message, "Test warning");
        assert_eq!(warning.field_path, Some("test.field".to_string()));
    }
}