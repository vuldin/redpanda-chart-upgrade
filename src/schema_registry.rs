use std::collections::HashMap;
use serde_yaml::Value;
use thiserror::Error;
use crate::{
    schema_version::SchemaVersion,
    transformation_rule::TransformationRule,
    validation::{SchemaDefinition, ValidationReport},
};

/// Registry that manages schema definitions and transformation rules
pub struct SchemaRegistry {
    schemas: HashMap<SchemaVersion, SchemaDefinition>,
    transformation_rules: HashMap<(SchemaVersion, SchemaVersion), Vec<TransformationRule>>,
    migration_paths: HashMap<SchemaVersion, Vec<SchemaVersion>>,
}

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("Schema version not found: {0}")]
    SchemaNotFound(SchemaVersion),

    #[error("No transformation rules found from {0} to {1}")]
    NoTransformationRules(String, String),

    #[error("Rule validation failed: {0}")]
    RuleValidationFailed(String),

    #[error("Schema definition error: {0}")]
    SchemaDefinitionError(String),
}

impl SchemaRegistry {
    pub fn new() -> Self {
        Self {
            schemas: HashMap::new(),
            transformation_rules: HashMap::new(),
            migration_paths: HashMap::new(),
        }
    }

    /// Add a schema definition to the registry
    pub fn add_schema(&mut self, schema: SchemaDefinition) {
        let version = schema.version.clone();
        self.schemas.insert(version, schema);
    }

    /// Add transformation rules between two schema versions
    pub fn add_transformation_rules(
        &mut self,
        source: SchemaVersion,
        target: SchemaVersion,
        rules: Vec<TransformationRule>,
    ) -> Result<(), RegistryError> {
        
        // Validate rules before adding
        self.validate_rules(&rules)?;
        
        self.transformation_rules.insert((source, target), rules);
        Ok(())
    }

    /// Get transformation rules between two versions
    pub fn get_transformation_rules(
        &self,
        source: &SchemaVersion,
        target: &SchemaVersion,
    ) -> Result<Vec<TransformationRule>, RegistryError> {
        
        self.transformation_rules
            .get(&(source.clone(), target.clone()))
            .cloned()
            .ok_or_else(|| RegistryError::NoTransformationRules(
                source.to_string(),
                target.to_string(),
            ))
    }

    /// Get the latest schema version
    pub fn get_latest_version(&self) -> Option<SchemaVersion> {
        self.schemas.keys().max().cloned()
    }

    /// Get all available schema versions
    pub fn get_available_versions(&self) -> Vec<SchemaVersion> {
        let mut versions: Vec<_> = self.schemas.keys().cloned().collect();
        versions.sort();
        versions
    }

    /// Validate a configuration against a specific schema version
    pub fn validate_configuration(
        &self,
        config: &Value,
        version: &SchemaVersion,
    ) -> Result<ValidationReport, RegistryError> {
        
        let schema = self.schemas.get(version)
            .ok_or_else(|| RegistryError::SchemaNotFound(version.clone()))?;

        let mut report = ValidationReport::new();

        // Check required fields
        for required_field in &schema.required_fields {
            if !self.field_exists(config, required_field) {
                report.add_missing_required_field(required_field.clone());
            }
        }

        // Check for deprecated fields
        for deprecated_field in &schema.deprecated_fields {
            if self.field_exists(config, deprecated_field) {
                report.add_deprecated_field(deprecated_field.clone());
            }
        }

        Ok(report)
    }

    /// Load transformation rules from external configuration
    pub fn load_rules_from_config(&mut self, _config_path: &str) -> Result<(), RegistryError> {
        // Placeholder for loading rules from YAML/JSON files
        // In a full implementation, this would parse external rule definitions
        Ok(())
    }

    /// Add a migration path between versions
    pub fn add_migration_path(&mut self, source: SchemaVersion, path: Vec<SchemaVersion>) {
        self.migration_paths.insert(source, path);
    }

    /// Get migration path from source to target version
    pub fn get_migration_path(
        &self,
        source: &SchemaVersion,
        target: &SchemaVersion,
    ) -> Option<Vec<SchemaVersion>> {
        
        if source == target {
            return Some(vec![target.clone()]);
        }

        // Check for direct path
        if let Some(path) = self.migration_paths.get(source) {
            if path.contains(target) {
                // Return path up to and including target
                let target_index = path.iter().position(|v| v == target)?;
                return Some(path[..=target_index].to_vec());
            }
        }

        // For now, return direct migration if no path is defined
        Some(vec![target.clone()])
    }

    /// Validate transformation rules for syntax and completeness
    fn validate_rules(&self, rules: &[TransformationRule]) -> Result<(), RegistryError> {
        for rule in rules {
            // Basic validation - check that rule has required fields
            if rule.rule_id.is_empty() {
                return Err(RegistryError::RuleValidationFailed(
                    "Rule ID cannot be empty".to_string()
                ));
            }
            
            if rule.source_path.is_empty() {
                return Err(RegistryError::RuleValidationFailed(
                    format!("Source path cannot be empty for rule {}", rule.rule_id)
                ));
            }
        }
        Ok(())
    }

    /// Check if a field exists in the configuration using dot notation
    fn field_exists(&self, config: &Value, field_path: &str) -> bool {
        let parts: Vec<&str> = field_path.split('.').collect();
        let mut current = config;

        for part in parts {
            match current {
                Value::Mapping(map) => {
                    if let Some(value) = map.get(&Value::String(part.to_string())) {
                        current = value;
                    } else {
                        return false;
                    }
                }
                _ => return false,
            }
        }

        true
    }
}

impl Default for SchemaRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        transformation_rule::TransformationType,
        validation::FieldType,
    };

    #[test]
    fn test_schema_registry_creation() {
        let registry = SchemaRegistry::new();
        assert!(registry.get_available_versions().is_empty());
        assert!(registry.get_latest_version().is_none());
    }

    #[test]
    fn test_add_schema() {
        let mut registry = SchemaRegistry::new();
        let version = SchemaVersion::new(25, 2, 9);
        let schema = SchemaDefinition::new(version.clone());
        
        registry.add_schema(schema);
        
        let versions = registry.get_available_versions();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0], version);
        assert_eq!(registry.get_latest_version(), Some(version));
    }

    #[test]
    fn test_add_transformation_rules() {
        let mut registry = SchemaRegistry::new();
        let source = SchemaVersion::new(5, 0, 10);
        let target = SchemaVersion::new(25, 2, 9);
        
        let rule = TransformationRule::new(
            "test_rule".to_string(),
            "old.path".to_string(),
            "new.path".to_string(),
            TransformationType::Move,
        );
        
        let result = registry.add_transformation_rules(source.clone(), target.clone(), vec![rule]);
        assert!(result.is_ok());
        
        let retrieved_rules = registry.get_transformation_rules(&source, &target);
        assert!(retrieved_rules.is_ok());
        assert_eq!(retrieved_rules.unwrap().len(), 1);
    }

    #[test]
    fn test_field_exists() {
        let registry = SchemaRegistry::new();
        let config: Value = serde_yaml::from_str(
            r#"
            level1:
              level2:
                field: value
            "#
        ).unwrap();

        assert!(registry.field_exists(&config, "level1.level2.field"));
        assert!(!registry.field_exists(&config, "level1.missing.field"));
        assert!(!registry.field_exists(&config, "missing"));
    }

    #[test]
    fn test_validate_configuration() {
        let mut registry = SchemaRegistry::new();
        let version = SchemaVersion::new(25, 2, 9);
        let mut schema = SchemaDefinition::new(version.clone());
        
        schema.add_required_field("image.tag".to_string(), FieldType::String);
        schema.add_deprecated_field("old.field".to_string());
        
        registry.add_schema(schema);

        let config_with_required: Value = serde_yaml::from_str(
            r#"
            image:
              tag: v25.2.9
            "#
        ).unwrap();

        let config_missing_required: Value = serde_yaml::from_str(
            r#"
            other:
              field: value
            "#
        ).unwrap();

        let report_valid = registry.validate_configuration(&config_with_required, &version).unwrap();
        assert!(report_valid.is_valid);
        assert_eq!(report_valid.missing_required_fields.len(), 0);

        let report_invalid = registry.validate_configuration(&config_missing_required, &version).unwrap();
        assert!(!report_invalid.is_valid);
        assert_eq!(report_invalid.missing_required_fields.len(), 1);
    }
}