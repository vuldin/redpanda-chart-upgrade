// Enhanced Schema Transformation System
pub mod schema_version;
pub mod transformation_rule;
pub mod validation;
pub mod transformation_engine;
pub mod schema_registry;
pub mod reporter;

// Re-export core types for convenience
pub use schema_version::SchemaVersion;
pub use transformation_rule::{TransformationRule, TransformationType, AppliedTransformation};
pub use validation::{ValidationReport, ValidationError, ValidationWarning};
pub use transformation_engine::{SchemaTransformationEngine, TransformationResult};
pub use schema_registry::SchemaRegistry;
pub use reporter::{TransformationReporter, TransformationReport};