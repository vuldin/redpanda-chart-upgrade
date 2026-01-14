use serde::{Deserialize, Serialize};
use serde_yaml::Value;

/// Represents a transformation rule that defines how to convert configuration fields
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformationRule {
    pub rule_id: String,
    pub source_path: String,
    pub target_path: String,
    pub transformation_type: TransformationType,
    pub condition: Option<Condition>,
    pub priority: u32,
}

/// Types of transformations that can be applied
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransformationType {
    /// Move a field from source to target path
    Move,
    /// Copy a field from source to target path
    Copy,
    /// Transform a field value using a custom function
    Transform(String), // Function name for serialization
    /// Merge multiple source fields into a single target field
    Merge(Vec<String>),
    /// Split a source field into multiple target fields
    Split(Vec<String>),
    /// Remove a field
    Remove,
}

/// Condition for conditional transformations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    pub field_path: String,
    pub condition_type: ConditionType,
    pub expected_value: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConditionType {
    FieldExists,
    FieldAbsent,
    ValueEquals,
    ValueNotEquals,
}

/// Represents a transformation that was applied during processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedTransformation {
    pub rule_id: String,
    pub source_path: String,
    pub target_path: String,
    pub old_value: Option<Value>,
    pub new_value: Option<Value>,
    pub transformation_type: TransformationType,
}

/// Represents a field change during transformation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldChange {
    pub path: String,
    pub change_type: ChangeType,
    pub old_value: Option<Value>,
    pub new_value: Option<Value>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChangeType {
    Added,
    Removed,
    Modified,
    Moved,
    Merged,
    Split,
}

impl TransformationRule {
    pub fn new(
        rule_id: String,
        source_path: String,
        target_path: String,
        transformation_type: TransformationType,
    ) -> Self {
        Self {
            rule_id,
            source_path,
            target_path,
            transformation_type,
            condition: None,
            priority: 100,
        }
    }

    pub fn with_condition(mut self, condition: Condition) -> Self {
        self.condition = Some(condition);
        self
    }

    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    /// Check if the rule's condition is satisfied
    pub fn condition_satisfied(&self, config: &Value) -> bool {
        match &self.condition {
            None => true,
            Some(condition) => {
                let field_value = get_nested_value(config, &condition.field_path);
                
                match condition.condition_type {
                    ConditionType::FieldExists => field_value.is_some(),
                    ConditionType::FieldAbsent => field_value.is_none(),
                    ConditionType::ValueEquals => {
                        match (&field_value, &condition.expected_value) {
                            (Some(actual), Some(expected)) => *actual == expected,
                            _ => false,
                        }
                    }
                    ConditionType::ValueNotEquals => {
                        match (&field_value, &condition.expected_value) {
                            (Some(actual), Some(expected)) => *actual != expected,
                            _ => true,
                        }
                    }
                }
            }
        }
    }
}

/// Helper function to get nested value from YAML using dot notation path
fn get_nested_value<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;

    for part in parts {
        match current {
            Value::Mapping(map) => {
                current = map.get(&Value::String(part.to_string()))?;
            }
            _ => return None,
        }
    }

    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transformation_rule_creation() {
        let rule = TransformationRule::new(
            "test_rule".to_string(),
            "old.path".to_string(),
            "new.path".to_string(),
            TransformationType::Move,
        );

        assert_eq!(rule.rule_id, "test_rule");
        assert_eq!(rule.source_path, "old.path");
        assert_eq!(rule.target_path, "new.path");
        assert_eq!(rule.priority, 100);
    }

    #[test]
    fn test_condition_field_exists() {
        let condition = Condition {
            field_path: "test.field".to_string(),
            condition_type: ConditionType::FieldExists,
            expected_value: None,
        };

        let rule = TransformationRule::new(
            "test".to_string(),
            "source".to_string(),
            "target".to_string(),
            TransformationType::Move,
        ).with_condition(condition);

        let config_with_field: Value = serde_yaml::from_str(
            r#"
            test:
              field: value
            "#
        ).unwrap();

        let config_without_field: Value = serde_yaml::from_str(
            r#"
            other:
              field: value
            "#
        ).unwrap();

        assert!(rule.condition_satisfied(&config_with_field));
        assert!(!rule.condition_satisfied(&config_without_field));
    }

    #[test]
    fn test_get_nested_value() {
        let config: Value = serde_yaml::from_str(
            r#"
            level1:
              level2:
                level3: test_value
            "#
        ).unwrap();

        let value = get_nested_value(&config, "level1.level2.level3");
        assert_eq!(value.unwrap().as_str().unwrap(), "test_value");

        let missing = get_nested_value(&config, "level1.missing.level3");
        assert!(missing.is_none());
    }
}