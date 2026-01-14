use serde_yaml::Value;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process;
use reqwest;
use clap::Parser;
use regex::Regex;

const LATEST_CHART_VALUES_URL: &str = "https://raw.githubusercontent.com/redpanda-data/redpanda-operator/refs/heads/main/charts/redpanda/chart/values.yaml";
const CONSOLE_CHART_YAML_URL: &str = "https://raw.githubusercontent.com/redpanda-data/redpanda-operator/refs/heads/main/charts/console/chart/Chart.yaml";
const VALUES_SCHEMA_URL: &str = "https://raw.githubusercontent.com/redpanda-data/redpanda-operator/refs/heads/main/charts/redpanda/chart/values.schema.json";

#[derive(Parser, Debug)]
#[command(name = "redpanda-chart-upgrade")]
#[command(about = "Transform legacy Redpanda Helm values to latest chart format")]
struct Args {
    /// Path to the existing deployment's values.yaml file
    values_file: String,

    /// Redpanda version to pin (e.g., v23.2.24) - required if input is missing image.tag
    #[arg(long, value_name = "VERSION")]
    redpanda_version: Option<String>,

    /// Console version to pin (e.g., v3.3.2) - auto-fetched if not provided
    #[arg(long, value_name = "VERSION")]
    console_version: Option<String>,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let file1_path = &args.values_file;

    // Read the existing deployment config file
    let file1 = fs::read_to_string(file1_path).expect("Failed to read the first YAML file");

    // Fetch the latest config file from the URL
    let file2 = reqwest::get(LATEST_CHART_VALUES_URL)
        .await
        .expect("Failed to fetch YAML from URL")
        .text()
        .await
        .expect("Failed to read the YAML content");

    // Parse both YAML files
    let mut data1: Value = serde_yaml::from_str(&file1).expect("Failed to parse the existing deployment config file");
    let data2: Value = serde_yaml::from_str(&file2).expect("Failed to parse the latest config file from the URL");

    // Rename the specified keys in data1
    rename_nested_keys(&mut data1);

    // FIRST: Map old field paths to new field paths (migrate values before removing)
    map_statefulset_to_podtemplate(&mut data1);

    // Migrate external configuration
    migrate_external_config(&mut data1);

    // Migrate Console configuration from v2 to v3 format
    migrate_console_v2_to_v3(&mut data1);

    // SECOND: Clean up deprecated fields after migration (but before merge)
    clean_deprecated_fields(&mut data1);

    // Merge the second YAML file into the first, keeping data1's values
    merge(&mut data1, &data2);

    // Pin versions (validate and add if missing)
    if let Err(e) = pin_versions(&mut data1, args.redpanda_version, args.console_version).await {
        eprintln!("\n❌ Error: {}", e);
        eprintln!("\nUsage: cargo run <values-file> --redpanda-version <VERSION>");
        eprintln!("Example: cargo run values.yaml --redpanda-version v23.2.24");
        process::exit(1);
    }

    // THIRD: Clean up again AFTER merge to remove any empty values added back by merge
    clean_empty_cloud_storage(&mut data1);
    // Note: NOT removing old resource format - schema requires BOTH old and new formats
    // clean_old_resource_format(&mut data1);

    // FOURTH: Filter null values from Console configuration
    filter_console_null_values(&mut data1);

    // FIFTH: Validate and harden tiered storage configuration
    validate_and_fix_tiered_storage(&mut data1);

    // SIXTH: Validate against Helm chart schema
    if let Err(e) = validate_against_schema(&data1).await {
        eprintln!("\n❌ Schema Validation Error: {}", e);
        eprintln!("\nThe transformed values.yaml does not conform to the latest Helm chart schema.");
        eprintln!("This may indicate that the transformation rules need to be updated.");
        eprintln!("Please report this issue with your input values.yaml file.");
        process::exit(1);
    }

    // Serialize the merged YAML to a string
    let updated_yaml = serde_yaml::to_string(&data1).expect("Failed to serialize the updated YAML");

    // Write the merged YAML to a file with a unique name
    let output_file = get_unique_filename("updated-values.yaml");
    let mut file = File::create(&output_file).expect("Failed to create the output file");
    file.write_all(updated_yaml.as_bytes()).expect("Failed to write to the output file");

    println!("\n=== Conversion Complete ===");
    println!("  ✓ Output file: {}", output_file);
}

/// Validates version string format (vX.Y.Z or vX.Y.Z-suffix)
fn validate_version_format(version: &str) -> Result<(), String> {
    let re = Regex::new(r"^v\d+\.\d+\.\d+(-[a-zA-Z0-9.-]+)?$").unwrap();
    if re.is_match(version) {
        Ok(())
    } else {
        Err(format!("Invalid version format '{}'. Expected: vX.Y.Z (e.g., v23.2.24)", version))
    }
}

/// Fetches the appVersion from a Chart.yaml URL
async fn fetch_chart_app_version(chart_yaml_url: &str) -> Result<String, String> {
    let response = reqwest::get(chart_yaml_url)
        .await
        .map_err(|e| format!("Failed to fetch Chart.yaml: {}", e))?
        .text()
        .await
        .map_err(|e| format!("Failed to read Chart.yaml: {}", e))?;

    let chart: Value = serde_yaml::from_str(&response)
        .map_err(|e| format!("Failed to parse Chart.yaml: {}", e))?;

    chart.get("appVersion")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "appVersion not found in Chart.yaml".to_string())
}

/// Pins Redpanda and Console versions in the merged configuration
async fn pin_versions(
    val: &mut Value,
    redpanda_version_arg: Option<String>,
    console_version_arg: Option<String>,
) -> Result<(), String> {
    println!("\n=== Version Pinning ===");

    // Handle Redpanda version
    let current_redpanda = val.get("image").and_then(|img| img.get("tag")).and_then(|tag| tag.as_str()).filter(|s| !s.is_empty());

    if let Some(version) = current_redpanda {
        println!("  ℹ Preserving existing image.tag: {}", version);
        validate_version_format(version)?;
    } else if let Some(version) = redpanda_version_arg {
        validate_version_format(&version)?;
        println!("  ✓ Pinning image.tag to {} (from CLI flag)", version);

        if let Value::Mapping(root_map) = val {
            let image_entry = root_map.entry(Value::String("image".to_string()))
                .or_insert_with(|| Value::Mapping(serde_yaml::Mapping::new()));
            if let Value::Mapping(image_map) = image_entry {
                image_map.insert(Value::String("tag".to_string()), Value::String(version));
            }
        }
    } else {
        return Err("image.tag is missing in input values.yaml. Please provide --redpanda-version flag (e.g., --redpanda-version v23.2.24)".to_string());
    }

    // Handle Console version (only if enabled)
    let console_enabled = val.get("console").and_then(|c| c.get("enabled")).and_then(|e| e.as_bool()).unwrap_or(false);

    if console_enabled {
        let current_console = val.get("console").and_then(|c| c.get("image")).and_then(|img| img.get("tag")).and_then(|tag| tag.as_str()).filter(|s| !s.is_empty());

        if let Some(version) = current_console {
            println!("  ℹ Preserving existing console.image.tag: {}", version);
            validate_version_format(version)?;
        } else {
            let version = if let Some(v) = console_version_arg {
                validate_version_format(&v)?;
                println!("  ✓ Pinning console.image.tag to {} (from CLI flag)", v);
                v
            } else {
                println!("  ℹ Auto-fetching latest Console version...");
                match fetch_chart_app_version(CONSOLE_CHART_YAML_URL).await {
                    Ok(v) => {
                        println!("  ✓ Pinning console.image.tag to {} (auto-discovered)", v);
                        v
                    }
                    Err(e) => {
                        println!("  ⚠ WARNING: Failed to auto-discover Console version: {}", e);
                        return Ok(()); // Continue without Console version
                    }
                }
            };

            if let Value::Mapping(root_map) = val {
                let console_entry = root_map.entry(Value::String("console".to_string()))
                    .or_insert_with(|| Value::Mapping(serde_yaml::Mapping::new()));
                if let Value::Mapping(console_map) = console_entry {
                    let image_entry = console_map.entry(Value::String("image".to_string()))
                        .or_insert_with(|| Value::Mapping(serde_yaml::Mapping::new()));
                    if let Value::Mapping(image_map) = image_entry {
                        image_map.insert(Value::String("tag".to_string()), Value::String(version));
                    }
                }
            }
        }
    } else {
        println!("  ℹ Console is disabled, skipping console version pinning");
    }

    Ok(())
}

// Recursive function to merge YAML values, keeping the first file's values
fn merge(val1: &mut Value, val2: &Value) {
    if let (Value::Mapping(map1), Value::Mapping(map2)) = (val1, val2) {
        for (k, v2) in map2 {
            let entry = map1.entry(k.clone()).or_insert(v2.clone());

            // Avoid moving `entry`, only check its reference
            if let Value::Mapping(_) = entry {
                if let Value::Mapping(_) = v2 {
                    // Recursively merge nested mappings
                    merge(entry, v2);
                }
            }
        }
    }
}

// Function to check for file existence and create a unique filename
fn get_unique_filename(base_name: &str) -> String {
    let mut count = 0;
    let mut file_name = base_name.to_string();

    while Path::new(&file_name).exists() {
        count += 1;
        file_name = format!("updated-values-{}.yaml", count);
    }

    file_name
}

fn map_statefulset_to_podtemplate(val: &mut Value) {
    if let Value::Mapping(map) = val {
        println!("\n=== Field Migration: Old Format → New Format ===");

        // Extract values from ROOT LEVEL that need to be migrated to podTemplate
        let mut root_node_selector = None;
        let mut root_tolerations = None;
        let mut root_affinity = None;

        // Check for root-level fields
        if let Some(ns) = map.get(&Value::String("nodeSelector".to_string())) {
            if !matches!(ns, Value::Mapping(m) if m.is_empty()) {
                println!("  ✓ Migrating root-level nodeSelector → podTemplate.spec.nodeSelector");
                root_node_selector = Some(ns.clone());
            }
        }
        if let Some(tol) = map.get(&Value::String("tolerations".to_string())) {
            if !matches!(tol, Value::Sequence(s) if s.is_empty()) {
                println!("  ✓ Migrating root-level tolerations → podTemplate.spec.tolerations");
                root_tolerations = Some(tol.clone());
            }
        }
        if let Some(aff) = map.get(&Value::String("affinity".to_string())) {
            if !matches!(aff, Value::Mapping(m) if m.is_empty()) {
                println!("  ✓ Migrating root-level affinity → podTemplate.spec.affinity");
                root_affinity = Some(aff.clone());
            }
        }

        // Extract values from statefulset that need to be migrated to podTemplate
        let mut node_selector = None;
        let mut tolerations = None;
        let mut pod_affinity = None;
        let mut pod_anti_affinity = None;
        let mut node_affinity = None;
        let mut security_context = None;
        let mut priority_class_name = None;
        let mut topology_spread_constraints = None;
        let mut termination_grace_period = None;
        let mut statefulset_annotations = None;

        if let Some(Value::Mapping(statefulset_map)) = map.get(&Value::String("statefulset".to_string())) {
            // Extract all the values we need to migrate
            if let Some(ns) = statefulset_map.get(&Value::String("nodeSelector".to_string())) {
                if !matches!(ns, Value::Mapping(m) if m.is_empty()) {
                    println!("  ✓ Migrating statefulset.nodeSelector → podTemplate.spec.nodeSelector");
                    node_selector = Some(ns.clone());
                }
            }
            if let Some(tol) = statefulset_map.get(&Value::String("tolerations".to_string())) {
                if !matches!(tol, Value::Sequence(s) if s.is_empty()) {
                    println!("  ✓ Migrating statefulset.tolerations → podTemplate.spec.tolerations");
                    tolerations = Some(tol.clone());
                }
            }
            if let Some(aff) = statefulset_map.get(&Value::String("podAffinity".to_string())) {
                if !matches!(aff, Value::Mapping(m) if m.is_empty()) {
                    println!("  ✓ Migrating statefulset.podAffinity → podTemplate.spec.affinity.podAffinity");
                    pod_affinity = Some(aff.clone());
                }
            }
            if let Some(anti_aff) = statefulset_map.get(&Value::String("podAntiAffinity".to_string())) {
                if !matches!(anti_aff, Value::Mapping(m) if m.is_empty()) {
                    println!("  ✓ Converting statefulset.podAntiAffinity → podTemplate.spec.affinity.podAntiAffinity");
                    // Convert v5 podAntiAffinity to standard Kubernetes format
                    if let Value::Mapping(anti_aff_map) = anti_aff {
                        if let Some(topology_key) = anti_aff_map.get(&Value::String("topologyKey".to_string())) {
                            let mut converted_anti_aff = serde_yaml::Mapping::new();
                            let mut required_item = serde_yaml::Mapping::new();
                            required_item.insert(Value::String("topologyKey".to_string()), topology_key.clone());

                            // Add standard labelSelector
                            let mut label_selector = serde_yaml::Mapping::new();
                            let mut match_labels = serde_yaml::Mapping::new();
                            match_labels.insert(Value::String("app.kubernetes.io/name".to_string()), Value::String("redpanda".to_string()));
                            match_labels.insert(Value::String("app.kubernetes.io/component".to_string()), Value::String("redpanda-statefulset".to_string()));
                            label_selector.insert(Value::String("matchLabels".to_string()), Value::Mapping(match_labels));
                            required_item.insert(Value::String("labelSelector".to_string()), Value::Mapping(label_selector));

                            converted_anti_aff.insert(
                                Value::String("requiredDuringSchedulingIgnoredDuringExecution".to_string()),
                                Value::Sequence(vec![Value::Mapping(required_item)])
                            );
                            pod_anti_affinity = Some(Value::Mapping(converted_anti_aff));
                        }
                    }
                }
            }
            if let Some(node_aff) = statefulset_map.get(&Value::String("nodeAffinity".to_string())) {
                if !matches!(node_aff, Value::Mapping(m) if m.is_empty()) {
                    println!("  ✓ Migrating statefulset.nodeAffinity → podTemplate.spec.affinity.nodeAffinity");
                    node_affinity = Some(node_aff.clone());
                }
            }
            if let Some(sc) = statefulset_map.get(&Value::String("securityContext".to_string())) {
                println!("  ✓ Migrating statefulset.securityContext → podTemplate.spec.securityContext");
                security_context = Some(sc.clone());
            }
            if let Some(pc) = statefulset_map.get(&Value::String("priorityClassName".to_string())) {
                println!("  ✓ Migrating statefulset.priorityClassName → podTemplate.spec.priorityClassName");
                priority_class_name = Some(pc.clone());
            }
            if let Some(tsc) = statefulset_map.get(&Value::String("topologySpreadConstraints".to_string())) {
                println!("  ✓ Migrating statefulset.topologySpreadConstraints → podTemplate.spec.topologySpreadConstraints");
                topology_spread_constraints = Some(tsc.clone());
            }
            if let Some(tgp) = statefulset_map.get(&Value::String("terminationGracePeriodSeconds".to_string())) {
                println!("  ✓ Migrating statefulset.terminationGracePeriodSeconds → podTemplate.spec.terminationGracePeriodSeconds");
                termination_grace_period = Some(tgp.clone());
            }
            if let Some(ann) = statefulset_map.get(&Value::String("annotations".to_string())) {
                if !matches!(ann, Value::Mapping(m) if m.is_empty()) {
                    println!("  ✓ Migrating statefulset.annotations → statefulset.podTemplate.annotations");
                    statefulset_annotations = Some(ann.clone());
                }
            }
        }

        // Now create or update podTemplate with the extracted values
        if root_node_selector.is_some() || root_tolerations.is_some() || root_affinity.is_some() ||
           node_selector.is_some() || tolerations.is_some() || pod_affinity.is_some() || pod_anti_affinity.is_some() ||
           node_affinity.is_some() || security_context.is_some() || priority_class_name.is_some() ||
           topology_spread_constraints.is_some() || termination_grace_period.is_some() {

            let pod_template_entry = map
                .entry(Value::String("podTemplate".to_string()))
                .or_insert_with(|| Value::Mapping(serde_yaml::Mapping::new()));

            if let Value::Mapping(pod_template_map) = pod_template_entry {
                let spec_entry = pod_template_map
                    .entry(Value::String("spec".to_string()))
                    .or_insert_with(|| Value::Mapping(serde_yaml::Mapping::new()));

                if let Value::Mapping(spec_map) = spec_entry {
                    // Migrate root-level fields first (lower priority - can be overridden)
                    if let Some(ns) = root_node_selector {
                        spec_map.entry(Value::String("nodeSelector".to_string())).or_insert(ns);
                    }
                    if let Some(tol) = root_tolerations {
                        spec_map.entry(Value::String("tolerations".to_string())).or_insert(tol);
                    }
                    if let Some(aff) = root_affinity {
                        spec_map.entry(Value::String("affinity".to_string())).or_insert(aff);
                    }

                    // Migrate statefulset fields (higher priority - override root-level)
                    if let Some(ns) = node_selector {
                        spec_map.insert(Value::String("nodeSelector".to_string()), ns);
                    }
                    if let Some(tol) = tolerations {
                        spec_map.insert(Value::String("tolerations".to_string()), tol);
                    }
                    if let Some(aff) = pod_affinity {
                        // podAffinity goes into affinity.podAffinity
                        let affinity_entry = spec_map
                            .entry(Value::String("affinity".to_string()))
                            .or_insert_with(|| Value::Mapping(serde_yaml::Mapping::new()));
                        if let Value::Mapping(affinity_map) = affinity_entry {
                            affinity_map.insert(Value::String("podAffinity".to_string()), aff);
                        }
                    }
                    if let Some(anti_aff) = pod_anti_affinity {
                        // podAntiAffinity goes into affinity.podAntiAffinity
                        let affinity_entry = spec_map
                            .entry(Value::String("affinity".to_string()))
                            .or_insert_with(|| Value::Mapping(serde_yaml::Mapping::new()));
                        if let Value::Mapping(affinity_map) = affinity_entry {
                            affinity_map.insert(Value::String("podAntiAffinity".to_string()), anti_aff);
                        }
                    }
                    if let Some(node_aff) = node_affinity {
                        // nodeAffinity goes into affinity.nodeAffinity
                        let affinity_entry = spec_map
                            .entry(Value::String("affinity".to_string()))
                            .or_insert_with(|| Value::Mapping(serde_yaml::Mapping::new()));
                        if let Value::Mapping(affinity_map) = affinity_entry {
                            affinity_map.insert(Value::String("nodeAffinity".to_string()), node_aff);
                        }
                    }
                    if let Some(sc) = security_context {
                        spec_map.insert(Value::String("securityContext".to_string()), sc);
                    }
                    if let Some(pc) = priority_class_name {
                        spec_map.insert(Value::String("priorityClassName".to_string()), pc);
                    }
                    if let Some(tsc) = topology_spread_constraints {
                        // Ensure topologySpreadConstraints have labelSelector
                        if let Value::Sequence(constraints) = &tsc {
                            let mut updated_constraints = vec![];
                            for constraint in constraints {
                                if let Value::Mapping(constraint_map) = constraint {
                                    let mut new_constraint = constraint_map.clone();
                                    // Add labelSelector if it doesn't exist
                                    if !new_constraint.contains_key(&Value::String("labelSelector".to_string())) {
                                        let mut label_selector = serde_yaml::Mapping::new();
                                        let mut match_labels = serde_yaml::Mapping::new();
                                        match_labels.insert(Value::String("app.kubernetes.io/name".to_string()), Value::String("redpanda".to_string()));
                                        match_labels.insert(Value::String("app.kubernetes.io/component".to_string()), Value::String("redpanda-statefulset".to_string()));
                                        label_selector.insert(Value::String("matchLabels".to_string()), Value::Mapping(match_labels));
                                        new_constraint.insert(Value::String("labelSelector".to_string()), Value::Mapping(label_selector));
                                        println!("  ✓ Added required labelSelector to topologySpreadConstraint");
                                    }
                                    updated_constraints.push(Value::Mapping(new_constraint));
                                } else {
                                    updated_constraints.push(constraint.clone());
                                }
                            }
                            spec_map.insert(Value::String("topologySpreadConstraints".to_string()), Value::Sequence(updated_constraints));
                        } else {
                            spec_map.insert(Value::String("topologySpreadConstraints".to_string()), tsc);
                        }
                    }
                    if let Some(tgp) = termination_grace_period {
                        spec_map.insert(Value::String("terminationGracePeriodSeconds".to_string()), tgp);
                    }
                }
            }
        }

        // Handle statefulset.annotations → statefulset.podTemplate.annotations migration
        // This is separate because it stays within the statefulset structure
        if let Some(ann) = statefulset_annotations {
            if let Some(Value::Mapping(statefulset_map)) = map.get_mut(&Value::String("statefulset".to_string())) {
                let pod_template_entry = statefulset_map
                    .entry(Value::String("podTemplate".to_string()))
                    .or_insert_with(|| Value::Mapping(serde_yaml::Mapping::new()));

                if let Value::Mapping(pod_template_map) = pod_template_entry {
                    // Merge with existing annotations if they exist, otherwise insert
                    if let Some(Value::Mapping(existing_annotations)) = pod_template_map.get_mut(&Value::String("annotations".to_string())) {
                        // Merge the annotations
                        if let Value::Mapping(new_annotations) = ann {
                            for (key, value) in new_annotations {
                                existing_annotations.entry(key).or_insert(value);
                            }
                        }
                    } else {
                        // No existing annotations, just insert
                        pod_template_map.insert(Value::String("annotations".to_string()), ann);
                    }
                }
            }
        }
    }
}

fn migrate_external_config(val: &mut Value) {
    if let Value::Mapping(root_map) = val {
        if let Some(Value::Mapping(external_map)) = root_map.get_mut(&Value::String("external".to_string())) {
            // Check if external.service.domain exists (old structure)
            if let Some(Value::Mapping(service_map)) = external_map.get(&Value::String("service".to_string())) {
                if let Some(domain_value) = service_map.get(&Value::String("domain".to_string())) {
                    println!("\n=== External Configuration Migration ===");
                    println!("  ✓ Migrating external.service.domain → external.domain");

                    // Move domain up one level to external.domain
                    external_map.insert(Value::String("domain".to_string()), domain_value.clone());
                }
            }
        }
    }
}

fn migrate_console_v2_to_v3(val: &mut Value) {
    if let Value::Mapping(root_map) = val {
        if let Some(Value::Mapping(console_map)) = root_map.get_mut(&Value::String("console".to_string())) {
            // Check if console.console.config exists (v2 structure)
            if let Some(Value::Mapping(inner_console)) = console_map.get(&Value::String("console".to_string())) {
                if let Some(v2_config) = inner_console.get(&Value::String("config".to_string())) {
                    println!("\n=== Console v2 → v3 Migration ===");

                    let mut v3_config = serde_yaml::Mapping::new();

                    if let Value::Mapping(v2_map) = v2_config {
                        // Migrate kafka configuration
                        if let Some(Value::Mapping(kafka_v2)) = v2_map.get(&Value::String("kafka".to_string())) {
                            let mut kafka_v3 = serde_yaml::Mapping::new();

                            // Copy basic fields
                            for key in &["brokers", "tls"] {
                                if let Some(value) = kafka_v2.get(&Value::String(key.to_string())) {
                                    kafka_v3.insert(Value::String(key.to_string()), value.clone());
                                }
                            }

                            // Migrate SASL - preserve structure but note that credentials go under authentication.basic in v3
                            if let Some(Value::Mapping(sasl_v2)) = kafka_v2.get(&Value::String("sasl".to_string())) {
                                let mut sasl_v3 = serde_yaml::Mapping::new();
                                sasl_v3.insert(
                                    Value::String("enabled".to_string()),
                                    sasl_v2.get(&Value::String("enabled".to_string()))
                                        .cloned()
                                        .unwrap_or(Value::Bool(false))
                                );
                                sasl_v3.insert(
                                    Value::String("mechanism".to_string()),
                                    sasl_v2.get(&Value::String("mechanism".to_string()))
                                        .cloned()
                                        .unwrap_or(Value::String("SCRAM-SHA-256".to_string()))
                                );

                                // Preserve username and password for v3
                                if let Some(username) = sasl_v2.get(&Value::String("username".to_string())) {
                                    sasl_v3.insert(Value::String("username".to_string()), username.clone());
                                }
                                if let Some(password) = sasl_v2.get(&Value::String("password".to_string())) {
                                    sasl_v3.insert(Value::String("password".to_string()), password.clone());
                                }

                                kafka_v3.insert(Value::String("sasl".to_string()), Value::Mapping(sasl_v3));
                            }

                            // Move schemaRegistry OUT of kafka to top level in v3
                            if let Some(Value::Mapping(sr_v2)) = kafka_v2.get(&Value::String("schemaRegistry".to_string())) {
                                println!("  ✓ Moving kafka.schemaRegistry → schemaRegistry (top-level)");
                                let mut sr_v3 = serde_yaml::Mapping::new();
                                sr_v3.insert(
                                    Value::String("enabled".to_string()),
                                    sr_v2.get(&Value::String("enabled".to_string()))
                                        .cloned()
                                        .unwrap_or(Value::Bool(false))
                                );
                                sr_v3.insert(
                                    Value::String("urls".to_string()),
                                    sr_v2.get(&Value::String("urls".to_string()))
                                        .cloned()
                                        .unwrap_or(Value::Sequence(vec![]))
                                );

                                // Migrate authentication to new structure
                                if sr_v2.contains_key(&Value::String("username".to_string())) ||
                                   sr_v2.contains_key(&Value::String("password".to_string())) {
                                    println!("  ✓ Wrapping schemaRegistry credentials in authentication.basic");
                                    let mut auth_map = serde_yaml::Mapping::new();
                                    let mut basic_map = serde_yaml::Mapping::new();
                                    basic_map.insert(
                                        Value::String("username".to_string()),
                                        sr_v2.get(&Value::String("username".to_string()))
                                            .cloned()
                                            .unwrap_or(Value::String("".to_string()))
                                    );
                                    basic_map.insert(
                                        Value::String("password".to_string()),
                                        sr_v2.get(&Value::String("password".to_string()))
                                            .cloned()
                                            .unwrap_or(Value::String("".to_string()))
                                    );
                                    auth_map.insert(Value::String("basic".to_string()), Value::Mapping(basic_map));
                                    sr_v3.insert(Value::String("authentication".to_string()), Value::Mapping(auth_map));
                                }

                                // TLS config
                                if let Some(tls) = sr_v2.get(&Value::String("tls".to_string())) {
                                    sr_v3.insert(Value::String("tls".to_string()), tls.clone());
                                }

                                v3_config.insert(Value::String("schemaRegistry".to_string()), Value::Mapping(sr_v3));
                            }

                            if !kafka_v3.is_empty() {
                                v3_config.insert(Value::String("kafka".to_string()), Value::Mapping(kafka_v3));
                            }
                        }

                        // Migrate redpanda adminApi configuration
                        if let Some(Value::Mapping(redpanda_v2)) = v2_map.get(&Value::String("redpanda".to_string())) {
                            if let Some(Value::Mapping(admin_v2)) = redpanda_v2.get(&Value::String("adminApi".to_string())) {
                                println!("  ✓ Migrating redpanda.adminApi with authentication.basic");
                                let mut redpanda_v3 = serde_yaml::Mapping::new();
                                let mut admin_v3 = serde_yaml::Mapping::new();

                                admin_v3.insert(
                                    Value::String("enabled".to_string()),
                                    admin_v2.get(&Value::String("enabled".to_string()))
                                        .cloned()
                                        .unwrap_or(Value::Bool(false))
                                );
                                admin_v3.insert(
                                    Value::String("urls".to_string()),
                                    admin_v2.get(&Value::String("urls".to_string()))
                                        .cloned()
                                        .unwrap_or(Value::Sequence(vec![]))
                                );

                                // Migrate authentication to new structure
                                if admin_v2.contains_key(&Value::String("username".to_string())) ||
                                   admin_v2.contains_key(&Value::String("password".to_string())) {
                                    let mut auth_map = serde_yaml::Mapping::new();
                                    let mut basic_map = serde_yaml::Mapping::new();
                                    basic_map.insert(
                                        Value::String("username".to_string()),
                                        admin_v2.get(&Value::String("username".to_string()))
                                            .cloned()
                                            .unwrap_or(Value::String("".to_string()))
                                    );
                                    basic_map.insert(
                                        Value::String("password".to_string()),
                                        admin_v2.get(&Value::String("password".to_string()))
                                            .cloned()
                                            .unwrap_or(Value::String("".to_string()))
                                    );
                                    auth_map.insert(Value::String("basic".to_string()), Value::Mapping(basic_map));
                                    admin_v3.insert(Value::String("authentication".to_string()), Value::Mapping(auth_map));
                                }

                                // TLS config
                                if let Some(tls) = admin_v2.get(&Value::String("tls".to_string())) {
                                    admin_v3.insert(Value::String("tls".to_string()), tls.clone());
                                }

                                redpanda_v3.insert(Value::String("adminApi".to_string()), Value::Mapping(admin_v3));
                                v3_config.insert(Value::String("redpanda".to_string()), Value::Mapping(redpanda_v3));
                            }
                        }
                    }

                    // Replace console.console.config with console.config
                    if !v3_config.is_empty() {
                        console_map.insert(Value::String("config".to_string()), Value::Mapping(v3_config));
                        console_map.remove(&Value::String("console".to_string()));
                        println!("  ✓ Console v2 → v3 migration complete");
                    }
                }
            }
        }
    }
}

fn clean_deprecated_fields(val: &mut Value) {
    if let Value::Mapping(map) = val {
        println!("\n=== Removing Deprecated Fields ===");

        // Remove root-level deprecated fields
        if map.remove(&Value::String("COMPUTED VALUES".to_string())).is_some() {
            println!("  ✓ Removed: COMPUTED VALUES (deprecated)");
        }
        if map.remove(&Value::String("tolerations".to_string())).is_some() {
            println!("  ✓ Removed: root-level tolerations (migrated to podTemplate.spec)");
        }
        if map.remove(&Value::String("nodeSelector".to_string())).is_some() {
            println!("  ✓ Removed: root-level nodeSelector (migrated to podTemplate.spec)");
        }
        if map.remove(&Value::String("affinity".to_string())).is_some() {
            println!("  ✓ Removed: root-level affinity (migrated to podTemplate.spec)");
        }
        if map.remove(&Value::String("post_upgrade_job".to_string())).is_some() {
            println!("  ✓ Removed: post_upgrade_job (deprecated)");
        }
        if map.remove(&Value::String("imagePullSecrets".to_string())).is_some() {
            println!("  ✓ Removed: root-level imagePullSecrets (deprecated)");
        }
        if map.remove(&Value::String("post_install_job".to_string())).is_some() {
            println!("  ✓ Removed: root-level post_install_job (deprecated)");
        }
        if map.remove(&Value::String("connectors".to_string())).is_some() {
            println!("  ✓ Removed: connectors (deprecated)");
        }
        if map.remove(&Value::String("podManagementPolicy".to_string())).is_some() {
            println!("  ✓ Removed: statefulset.podManagementPolicy (deprecated)");
        }

        // Remove image.pullPolicy
        if let Some(Value::Mapping(image_map)) = map.get_mut(&Value::String("image".to_string())) {
            image_map.remove(&Value::String("pullPolicy".to_string()));
        }

        // Clean up statefulset deprecated fields (now that they've been migrated)
        if let Some(Value::Mapping(statefulset_map)) = map.get_mut(&Value::String("statefulset".to_string())) {
            if statefulset_map.remove(&Value::String("securityContext".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.securityContext (migrated to podTemplate.spec)");
            }
            if statefulset_map.remove(&Value::String("tolerations".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.tolerations (migrated to podTemplate.spec)");
            }
            if statefulset_map.remove(&Value::String("nodeSelector".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.nodeSelector (migrated to podTemplate.spec)");
            }
            if statefulset_map.remove(&Value::String("priorityClassName".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.priorityClassName (migrated to podTemplate.spec)");
            }
            if statefulset_map.remove(&Value::String("startupProbe".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.startupProbe (deprecated)");
            }
            if statefulset_map.remove(&Value::String("livenessProbe".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.livenessProbe (deprecated)");
            }
            if statefulset_map.remove(&Value::String("readinessProbe".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.readinessProbe (deprecated)");
            }
            if statefulset_map.remove(&Value::String("annotations".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.annotations (deprecated)");
            }
            if statefulset_map.remove(&Value::String("topologySpreadConstraints".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.topologySpreadConstraints (migrated to podTemplate.spec)");
            }
            if statefulset_map.remove(&Value::String("extraVolumes".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.extraVolumes (deprecated)");
            }
            if statefulset_map.remove(&Value::String("extraVolumeMounts".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.extraVolumeMounts (deprecated)");
            }
            if statefulset_map.remove(&Value::String("podAffinity".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.podAffinity (migrated to podTemplate.spec.affinity)");
            }
            if statefulset_map.remove(&Value::String("podAntiAffinity".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.podAntiAffinity (migrated to podTemplate.spec.affinity)");
            }
            if statefulset_map.remove(&Value::String("nodeAffinity".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.nodeAffinity (migrated to podTemplate.spec.affinity)");
            }
            if statefulset_map.remove(&Value::String("terminationGracePeriodSeconds".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.terminationGracePeriodSeconds (migrated to podTemplate.spec)");
            }
            if statefulset_map.remove(&Value::String("podManagementPolicy".to_string())).is_some() {
                println!("  ✓ Removed: statefulset.podManagementPolicy (deprecated)");
            }

            // Clean up initContainers deprecated fields
            if let Some(Value::Mapping(init_map)) = statefulset_map.get_mut(&Value::String("initContainers".to_string())) {
                init_map.remove(&Value::String("tuning".to_string()));
                init_map.remove(&Value::String("extraInitContainers".to_string()));
                init_map.remove(&Value::String("setTieredStorageCacheDirOwnership".to_string()));

                // Remove extraVolumeMounts and resources from configurator
                if let Some(Value::Mapping(configurator_map)) = init_map.get_mut(&Value::String("configurator".to_string())) {
                    configurator_map.remove(&Value::String("extraVolumeMounts".to_string()));
                    configurator_map.remove(&Value::String("resources".to_string()));
                }

                // Remove extraVolumeMounts and resources from setDataDirOwnership
                if let Some(Value::Mapping(set_data_map)) = init_map.get_mut(&Value::String("setDataDirOwnership".to_string())) {
                    set_data_map.remove(&Value::String("extraVolumeMounts".to_string()));
                    set_data_map.remove(&Value::String("resources".to_string()));
                }
            }

            // Clean up sideCars deprecated fields
            if let Some(Value::Mapping(sidecars_map)) = statefulset_map.get_mut(&Value::String("sideCars".to_string())) {
                if let Some(Value::Mapping(config_watcher_map)) = sidecars_map.get_mut(&Value::String("configWatcher".to_string())) {
                    config_watcher_map.remove(&Value::String("extraVolumeMounts".to_string()));
                    config_watcher_map.remove(&Value::String("resources".to_string()));
                    config_watcher_map.remove(&Value::String("securityContext".to_string()));
                }
            }
        }

        // Remove kafkaEndpoint from listeners
        if let Some(Value::Mapping(listeners_map)) = map.get_mut(&Value::String("listeners".to_string())) {
            if let Some(Value::Mapping(http_map)) = listeners_map.get_mut(&Value::String("http".to_string())) {
                http_map.remove(&Value::String("kafkaEndpoint".to_string()));
            }
            if let Some(Value::Mapping(sr_map)) = listeners_map.get_mut(&Value::String("schemaRegistry".to_string())) {
                sr_map.remove(&Value::String("kafkaEndpoint".to_string()));
            }
        }

        // Remove external.service.domain (migrated to external.domain)
        if let Some(Value::Mapping(external_map)) = map.get_mut(&Value::String("external".to_string())) {
            if let Some(Value::Mapping(service_map)) = external_map.get_mut(&Value::String("service".to_string())) {
                if service_map.remove(&Value::String("domain".to_string())).is_some() {
                    println!("  ✓ Removed: external.service.domain (migrated to external.domain)");
                }
            }
        }

        // Remove empty licenseSecretRef from enterprise
        if let Some(Value::Mapping(enterprise_map)) = map.get_mut(&Value::String("enterprise".to_string())) {
            if let Some(Value::Mapping(license_ref)) = enterprise_map.get(&Value::String("licenseSecretRef".to_string())) {
                if license_ref.is_empty() {
                    enterprise_map.remove(&Value::String("licenseSecretRef".to_string()));
                }
            }
        }
    }
}

fn clean_empty_cloud_storage(val: &mut Value) {
    if let Value::Mapping(map) = val {
        // Clean up empty cloud storage config when disabled (run after merge)
        if let Some(Value::Mapping(storage_map)) = map.get_mut(&Value::String("storage".to_string())) {
            if let Some(Value::Mapping(tiered_map)) = storage_map.get_mut(&Value::String("tiered".to_string())) {
                if let Some(Value::Mapping(config_map)) = tiered_map.get_mut(&Value::String("config".to_string())) {
                    // Check if cloud_storage_enabled is false
                    let is_enabled = config_map
                        .get(&Value::String("cloud_storage_enabled".to_string()))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    if !is_enabled {
                        // Remove all cloud storage properties when disabled
                        let keys_to_remove = vec![
                            "cloud_storage_access_key",
                            "cloud_storage_api_endpoint",
                            "cloud_storage_azure_container",
                            "cloud_storage_azure_shared_key",
                            "cloud_storage_azure_storage_account",
                            "cloud_storage_bucket",
                            "cloud_storage_cache_size",
                            "cloud_storage_credentials_source",
                            "cloud_storage_enable_remote_read",
                            "cloud_storage_enable_remote_write",
                            "cloud_storage_region",
                            "cloud_storage_secret_key",
                        ];

                        for key in keys_to_remove {
                            config_map.remove(&Value::String(key.to_string()));
                        }
                    }
                }

                // Also remove credentialsSecretRef if storage is disabled
                if let Some(Value::Mapping(config_map)) = tiered_map.get(&Value::String("config".to_string())) {
                    let is_enabled = config_map
                        .get(&Value::String("cloud_storage_enabled".to_string()))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    if !is_enabled {
                        tiered_map.remove(&Value::String("credentialsSecretRef".to_string()));
                    }
                }
            }
        }
    }
}

fn clean_old_resource_format(val: &mut Value) {
    if let Value::Mapping(map) = val {
        // Remove old resource format that may have been added back by merge
        if let Some(Value::Mapping(resources_map)) = map.get_mut(&Value::String("resources".to_string())) {
            // Check if we have the new format (requests/limits)
            let has_new_format = resources_map.contains_key(&Value::String("requests".to_string()))
                && resources_map.contains_key(&Value::String("limits".to_string()));

            // Check if we have old format
            let has_old_format = resources_map.contains_key(&Value::String("cpu".to_string()))
                || resources_map.contains_key(&Value::String("memory".to_string()));

            if has_new_format && has_old_format {
                println!("\n=== Post-Merge Cleanup ===");
                if resources_map.remove(&Value::String("cpu".to_string())).is_some() {
                    println!("  ✓ Removed: resources.cpu (old format - already converted to requests/limits)");
                }
                if resources_map.remove(&Value::String("memory".to_string())).is_some() {
                    println!("  ✓ Removed: resources.memory (old format - already converted to requests/limits)");
                }
            }
        }
    }
}

fn rename_nested_keys(val: &mut Value) {
    if let Value::Mapping(map) = val {
        // Recursively traverse the nested mappings
        for (_, v) in map.iter_mut() {
            rename_nested_keys(v);
        }

        // Convert old resources format to new format with matching requests/limits
        if let Some(Value::Mapping(resources_map)) = map.get(&Value::String("resources".to_string())) {
            let mut cpu_value = None;
            let mut memory_value = None;

            // Extract CPU cores from resources.cpu.cores
            if let Some(Value::Mapping(cpu_map)) = resources_map.get(&Value::String("cpu".to_string())) {
                if let Some(cores) = cpu_map.get(&Value::String("cores".to_string())) {
                    cpu_value = Some(cores.clone());
                }
            }

            // Extract memory from resources.memory.container.max
            if let Some(Value::Mapping(memory_map)) = resources_map.get(&Value::String("memory".to_string())) {
                if let Some(Value::Mapping(container_map)) = memory_map.get(&Value::String("container".to_string())) {
                    if let Some(max) = container_map.get(&Value::String("max".to_string())) {
                        memory_value = Some(max.clone());
                    }
                }
            }

            // If we found old format resources, add new format (but preserve old format - schema requires both)
            if cpu_value.is_some() || memory_value.is_some() {
                println!("\n=== Resource Format Conversion ===");
                if let Some(Value::Mapping(resources_map)) = map.get_mut(&Value::String("resources".to_string())) {
                    // Create new requests and limits mappings
                    let mut requests_map = serde_yaml::Mapping::new();
                    let mut limits_map = serde_yaml::Mapping::new();

                    if let Some(cpu) = &cpu_value {
                        println!("  ✓ Adding resources.requests.cpu & resources.limits.cpu (value: {:?})", cpu);
                        requests_map.insert(Value::String("cpu".to_string()), cpu.clone());
                        limits_map.insert(Value::String("cpu".to_string()), cpu.clone());
                    }

                    if let Some(memory) = &memory_value {
                        println!("  ✓ Adding resources.requests.memory & resources.limits.memory (value: {:?})", memory);
                        requests_map.insert(Value::String("memory".to_string()), memory.clone());
                        limits_map.insert(Value::String("memory".to_string()), memory.clone());
                    }

                    println!("  ℹ Note: Preserving old format (resources.cpu.cores, resources.memory.container.max) - required by schema");
                    println!("  ℹ Note: Requests and limits are set to matching values for production readiness");

                    // Set requests and limits (matching for production readiness)
                    // IMPORTANT: DON'T remove old format - schema requires both formats
                    resources_map.insert(Value::String("requests".to_string()), Value::Mapping(requests_map));
                    resources_map.insert(Value::String("limits".to_string()), Value::Mapping(limits_map));
                }
            }
        }

        // Move keys from "storage.tieredConfig.*" to "storage.tiered.config.*"
        if let Some(Value::Mapping(tiered_config_map)) = map.remove(&Value::String("tieredConfig".to_string())) {
            if let Some(Value::Mapping(tiered_map)) = map.get_mut(&Value::String("tiered".to_string())) {
                let config_entry = tiered_map
                    .entry(Value::String("config".to_string()))
                    .or_insert_with(|| Value::Mapping(serde_yaml::Mapping::new()));

                if let Value::Mapping(ref mut config_map) = config_entry {
                    for (k, v) in tiered_config_map {
                        config_map.insert(k, v);
                    }
                }
            } else {
                let mut new_tiered_map = serde_yaml::Mapping::new();
                let mut new_config_map = serde_yaml::Mapping::new();
                for (k, v) in tiered_config_map {
                    new_config_map.insert(k, v);
                }
                new_tiered_map.insert(Value::String("config".to_string()), Value::Mapping(new_config_map));
                map.insert(Value::String("tiered".to_string()), Value::Mapping(new_tiered_map));
            }
        }

        // Rename "storage.tieredStorageHostPath" -> "storage.tiered.hostPath"
        if let Some(tiered_storage_host_path) = map.remove(&Value::String("tieredStorageHostPath".to_string())) {
            if let Some(Value::Mapping(tiered_map)) = map.get_mut(&Value::String("tiered".to_string())) {
                tiered_map.insert(Value::String("hostPath".to_string()), tiered_storage_host_path);
            }
        }

        // Rename "storage.tieredStoragePersistentVolume" -> "storage.tiered.persistentVolume"
        if let Some(tiered_storage_pv) = map.remove(&Value::String("tieredStoragePersistentVolume".to_string())) {
            if let Some(Value::Mapping(tiered_map)) = map.get_mut(&Value::String("tiered".to_string())) {
                tiered_map.insert(Value::String("persistentVolume".to_string()), tiered_storage_pv);
            }
        }

        // Move and rename keys inside "license_secret_ref" -> "enterprise.licenseSecretRef"
        if let Some(Value::Mapping(mut license_secret_ref_map)) = map.remove(&Value::String("license_secret_ref".to_string())) {
            // Rename "secret_name" -> "name" and "secret_key" -> "key" inside the object
            if let Some(secret_name) = license_secret_ref_map.remove(&Value::String("secret_name".to_string())) {
                license_secret_ref_map.insert(Value::String("name".to_string()), secret_name);
            }
            if let Some(secret_key) = license_secret_ref_map.remove(&Value::String("secret_key".to_string())) {
                license_secret_ref_map.insert(Value::String("key".to_string()), secret_key);
            }

            // Move to "enterprise.licenseSecretRef"
            let enterprise_entry = map
                .entry(Value::String("enterprise".to_string()))
                .or_insert_with(|| Value::Mapping(serde_yaml::Mapping::new()));

            if let Value::Mapping(enterprise_map) = enterprise_entry {
                enterprise_map.insert(Value::String("licenseSecretRef".to_string()), Value::Mapping(license_secret_ref_map));
            }
        }

        // Rename "license_key" -> "enterprise.license"
        if let Some(license_key) = map.remove(&Value::String("license_key".to_string())) {
            let enterprise_entry = map
                .entry(Value::String("enterprise".to_string()))
                .or_insert_with(|| Value::Mapping(serde_yaml::Mapping::new()));

            if let Value::Mapping(enterprise_map) = enterprise_entry {
                enterprise_map.insert(Value::String("license".to_string()), license_key);
            }
        }
    }
}

fn filter_console_null_values(val: &mut Value) {
    if let Value::Mapping(root_map) = val {
        if let Some(Value::Mapping(console_map)) = root_map.get_mut(&Value::String("console".to_string())) {
            println!("\n=== Filtering Console Null Values ===");

            // Filter ingress - remove null className
            if let Some(Value::Mapping(ingress_map)) = console_map.get_mut(&Value::String("ingress".to_string())) {
                if let Some(Value::Null) = ingress_map.get(&Value::String("className".to_string())) {
                    ingress_map.remove(&Value::String("className".to_string()));
                    println!("  ✓ Removed: console.ingress.className (null value)");
                }
            }

            // Filter service - remove null targetPort
            if let Some(Value::Mapping(service_map)) = console_map.get_mut(&Value::String("service".to_string())) {
                if let Some(Value::Null) = service_map.get(&Value::String("targetPort".to_string())) {
                    service_map.remove(&Value::String("targetPort".to_string()));
                    println!("  ✓ Removed: console.service.targetPort (null value)");
                }
            }

            // Filter secret - remove enterprise and login fields (not in v25 schema)
            if let Some(Value::Mapping(secret_map)) = console_map.get_mut(&Value::String("secret".to_string())) {
                if secret_map.remove(&Value::String("enterprise".to_string())).is_some() {
                    println!("  ✓ Removed: console.secret.enterprise (not in v25 schema)");
                }
                if secret_map.remove(&Value::String("login".to_string())).is_some() {
                    println!("  ✓ Removed: console.secret.login (not in v25 schema)");
                }
            }

            // Remove enterprise field from console (not in v25 schema)
            if console_map.remove(&Value::String("enterprise".to_string())).is_some() {
                println!("  ✓ Removed: console.enterprise (not in v25 schema)");
            }
        }
    }
}

// Validate tiered storage configuration
fn validate_and_fix_tiered_storage(val: &mut Value) {
    println!("\n=== Validating Tiered Storage Configuration ===");

    if let Value::Mapping(root_map) = val {
        if let Some(Value::Mapping(storage_map)) = root_map.get_mut(&Value::String("storage".to_string())) {
            if let Some(Value::Mapping(tiered_map)) = storage_map.get_mut(&Value::String("tiered".to_string())) {
                if let Some(Value::Mapping(config_map)) = tiered_map.get_mut(&Value::String("config".to_string())) {

                    // Check if cloud_storage is enabled
                    let is_enabled = config_map
                        .get(&Value::String("cloud_storage_enabled".to_string()))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    if !is_enabled {
                        println!("  ℹ Tiered storage is disabled, skipping validation");
                        return;
                    }

                    // Check if bucket and region are configured
                    let has_bucket = config_map.contains_key(&Value::String("cloud_storage_bucket".to_string()));
                    let has_region = config_map.contains_key(&Value::String("cloud_storage_region".to_string()));

                    if !has_bucket || !has_region {
                        println!("  ℹ Tiered storage enabled but no bucket/region configured");
                        return;
                    }

                    // Validate credentials are configured
                    let has_access_key = config_map.contains_key(&Value::String("cloud_storage_access_key".to_string()));
                    let has_secret_key = config_map.contains_key(&Value::String("cloud_storage_secret_key".to_string()));
                    let has_creds_source = config_map.contains_key(&Value::String("cloud_storage_credentials_source".to_string()));

                    if !has_access_key && !has_creds_source {
                        println!("  ⚠ WARNING: No credentials configured (neither access keys nor credentials_source)");
                        println!("     Either set cloud_storage_access_key/cloud_storage_secret_key or cloud_storage_credentials_source");
                        return;
                    }

                    if has_access_key && !has_secret_key {
                        println!("  ⚠ WARNING: cloud_storage_access_key is set but cloud_storage_secret_key is missing");
                        return;
                    }

                    // Check if API endpoint is configured
                    let has_endpoint = config_map.contains_key(&Value::String("cloud_storage_api_endpoint".to_string()));

                    if !has_endpoint {
                        println!("  ℹ cloud_storage_api_endpoint not set (will be auto-detected from region/bucket)");
                    } else {
                        println!("  ✓ cloud_storage_api_endpoint is explicitly configured");
                    }

                    // Report credentials configuration method
                    if has_access_key {
                        println!("  ✓ Using access key authentication (cloud_storage_credentials_source defaults to 'config_file')");
                    } else if has_creds_source {
                        if let Some(Value::String(source)) = config_map.get(&Value::String("cloud_storage_credentials_source".to_string())) {
                            println!("  ✓ Using cloud_storage_credentials_source: {}", source);
                        }
                    }

                    println!("  ✓ Tiered storage configuration validated");
                }
            }
        }
    }
}

/// Validates the output values against the Helm chart schema
async fn validate_against_schema(val: &Value) -> Result<(), String> {
    println!("\n=== Schema Validation ===");
    println!("  ℹ Fetching latest chart schema...");

    // Fetch the schema
    let schema_response = reqwest::get(VALUES_SCHEMA_URL)
        .await
        .map_err(|e| format!("Failed to fetch schema: {}", e))?
        .text()
        .await
        .map_err(|e| format!("Failed to read schema: {}", e))?;

    let schema_json: serde_json::Value = serde_json::from_str(&schema_response)
        .map_err(|e| format!("Failed to parse schema JSON: {}", e))?;

    // Convert our YAML value to JSON for validation
    let yaml_str = serde_yaml::to_string(val)
        .map_err(|e| format!("Failed to serialize values to YAML: {}", e))?;
    let instance_json: serde_json::Value = serde_yaml::from_str(&yaml_str)
        .map_err(|e| format!("Failed to parse values as JSON: {}", e))?;

    // Compile the schema
    let compiled_schema = jsonschema::JSONSchema::compile(&schema_json)
        .map_err(|e| format!("Failed to compile schema: {}", e))?;

    // Validate
    if let Err(errors) = compiled_schema.validate(&instance_json) {
        println!("  ❌ Schema validation failed:");
        let error_messages: Vec<String> = errors
            .map(|error| {
                let error_msg = format!("    - {}: {}", error.instance_path, error);
                println!("{}", error_msg);
                error_msg
            })
            .collect();

        return Err(format!("Schema validation failed with {} error(s):\n{}",
            error_messages.len(),
            error_messages.join("\n")
        ));
    }

    println!("  ✓ Schema validation passed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_yaml::Value;

    #[test]
    fn test_merge_preserves_existing_values() {
        let mut val1 = serde_yaml::from_str(
            r#"
            image:
              tag: v23.2.24
            storage:
              size: 10Gi
            "#
        ).unwrap();

        let val2 = serde_yaml::from_str(
            r#"
            image:
              tag: v25.2.9
              repository: docker.redpanda.com/redpandadata/redpanda
            storage:
              size: 100Gi
            "#
        ).unwrap();

        merge(&mut val1, &val2);

        // Original values should be preserved
        assert_eq!(
            val1["image"]["tag"].as_str().unwrap(),
            "v23.2.24"
        );
        assert_eq!(
            val1["storage"]["size"].as_str().unwrap(),
            "10Gi"
        );
        // New keys from val2 should be added
        assert_eq!(
            val1["image"]["repository"].as_str().unwrap(),
            "docker.redpanda.com/redpandadata/redpanda"
        );
    }

    #[test]
    fn test_rename_nested_keys_license_migration() {
        let mut val = serde_yaml::from_str(
            r#"
            license_secret_ref:
              secret_name: redpanda-license
              secret_key: redpanda.license
            "#
        ).unwrap();

        rename_nested_keys(&mut val);

        assert!(val.get("license_secret_ref").is_none());
        assert_eq!(
            val["enterprise"]["licenseSecretRef"]["name"].as_str().unwrap(),
            "redpanda-license"
        );
        assert_eq!(
            val["enterprise"]["licenseSecretRef"]["key"].as_str().unwrap(),
            "redpanda.license"
        );
    }

    #[test]
    fn test_rename_nested_keys_tiered_storage_migration() {
        let mut val = serde_yaml::from_str(
            r#"
            storage:
              tieredConfig:
                cloud_storage_enabled: true
                cloud_storage_bucket: test-bucket
                cloud_storage_region: us-central1
            "#
        ).unwrap();

        rename_nested_keys(&mut val);

        assert!(val["storage"].get("tieredConfig").is_none());
        assert_eq!(
            val["storage"]["tiered"]["config"]["cloud_storage_enabled"].as_bool().unwrap(),
            true
        );
        assert_eq!(
            val["storage"]["tiered"]["config"]["cloud_storage_bucket"].as_str().unwrap(),
            "test-bucket"
        );
    }

    #[test]
    fn test_map_statefulset_to_podtemplate() {
        let mut val = serde_yaml::from_str(
            r#"
            statefulset:
              replicas: 3
              nodeSelector:
                nodetype: redpanda-pool1
              tolerations:
                - key: redpanda-pool1
                  operator: Equal
                  value: "true"
                  effect: NoSchedule
            "#
        ).unwrap();

        map_statefulset_to_podtemplate(&mut val);

        // Check nodeSelector was migrated
        assert_eq!(
            val["podTemplate"]["spec"]["nodeSelector"]["nodetype"].as_str().unwrap(),
            "redpanda-pool1"
        );
        // Check tolerations was migrated
        assert_eq!(
            val["podTemplate"]["spec"]["tolerations"][0]["key"].as_str().unwrap(),
            "redpanda-pool1"
        );
        // Check replicas stays in statefulset
        assert_eq!(
            val["statefulset"]["replicas"].as_i64().unwrap(),
            3
        );
    }

    #[test]
    fn test_clean_deprecated_fields() {
        let mut val = serde_yaml::from_str(
            r#"
            statefulset:
              replicas: 3
              nodeSelector:
                nodetype: redpanda-pool1
              tolerations:
                - key: redpanda-pool1
            image:
              pullPolicy: IfNotPresent
              tag: v23.2.24
            post_upgrade_job:
              enabled: true
            "#
        ).unwrap();

        clean_deprecated_fields(&mut val);

        // Deprecated fields should be removed
        assert!(val["statefulset"].get("nodeSelector").is_none());
        assert!(val["statefulset"].get("tolerations").is_none());
        assert!(val["image"].get("pullPolicy").is_none());
        assert!(val.get("post_upgrade_job").is_none());

        // Non-deprecated fields should remain
        assert_eq!(val["statefulset"]["replicas"].as_i64().unwrap(), 3);
        assert_eq!(val["image"]["tag"].as_str().unwrap(), "v23.2.24");
    }

    #[test]
    fn test_clean_old_resource_format() {
        let mut val = serde_yaml::from_str(
            r#"
            resources:
              cpu:
                cores: 2
              memory:
                container:
                  max: 4Gi
              requests:
                cpu: 2
                memory: 4Gi
              limits:
                cpu: 2
                memory: 4Gi
            "#
        ).unwrap();

        clean_old_resource_format(&mut val);

        // Old format should be removed
        assert!(val["resources"].get("cpu").is_none());
        assert!(val["resources"].get("memory").is_none());

        // New format should remain
        assert_eq!(val["resources"]["requests"]["cpu"].as_i64().unwrap(), 2);
        assert_eq!(val["resources"]["limits"]["memory"].as_str().unwrap(), "4Gi");
    }

    #[test]
    fn test_validate_and_fix_tiered_storage_with_complete_config() {
        let mut val = serde_yaml::from_str(
            r#"
            storage:
              tiered:
                config:
                  cloud_storage_enabled: true
                  cloud_storage_bucket: test-bucket
                  cloud_storage_region: us-central1
                  cloud_storage_access_key: GOOG1ETEST
                  cloud_storage_secret_key: test-secret
                  cloud_storage_api_endpoint: storage.googleapis.com
                  cloud_storage_credentials_source: config_file
            "#
        ).unwrap();

        validate_and_fix_tiered_storage(&mut val);

        // Configuration should remain intact
        assert_eq!(
            val["storage"]["tiered"]["config"]["cloud_storage_api_endpoint"].as_str().unwrap(),
            "storage.googleapis.com"
        );
        assert_eq!(
            val["storage"]["tiered"]["config"]["cloud_storage_credentials_source"].as_str().unwrap(),
            "config_file"
        );
        assert_eq!(
            val["storage"]["tiered"]["config"]["cloud_storage_bucket"].as_str().unwrap(),
            "test-bucket"
        );
    }

    #[test]
    fn test_clean_empty_cloud_storage() {
        let mut val = serde_yaml::from_str(
            r#"
            storage:
              tiered:
                config:
                  cloud_storage_enabled: false
                  cloud_storage_bucket: ""
                  cloud_storage_region: ""
            "#
        ).unwrap();

        clean_empty_cloud_storage(&mut val);

        // Empty cloud storage config should be removed when disabled
        if let Some(storage) = val.get("storage") {
            if let Some(tiered) = storage.get("tiered") {
                if let Some(config) = tiered.get("config") {
                    assert!(config.get("cloud_storage_bucket").is_none());
                    assert!(config.get("cloud_storage_region").is_none());
                }
            }
        }
    }

    #[test]
    fn test_get_unique_filename() {
        let filename = get_unique_filename("test.yaml");
        assert!(filename.starts_with("test"));
        assert!(filename.ends_with(".yaml"));
    }

    #[test]
    fn test_resource_format_exists_in_old_format() {
        let val: Value = serde_yaml::from_str(
            r#"
            resources:
              cpu:
                cores: 1
              memory:
                container:
                  max: 2.5Gi
            "#
        ).unwrap();

        // Check old format exists
        assert_eq!(val["resources"]["cpu"]["cores"].as_i64().unwrap(), 1);
        assert_eq!(val["resources"]["memory"]["container"]["max"].as_str().unwrap(), "2.5Gi");
    }

    #[test]
    fn test_clean_old_resource_format_with_new_format_present() {
        let mut val: Value = serde_yaml::from_str(
            r#"
            resources:
              cpu:
                cores: 2
              memory:
                container:
                  max: 4Gi
              requests:
                cpu: 2
                memory: 4Gi
              limits:
                cpu: 2
                memory: 4Gi
            "#
        ).unwrap();

        clean_old_resource_format(&mut val);

        // Old format should be removed
        assert!(val["resources"].get("cpu").is_none());
        assert!(val["resources"].get("memory").is_none());

        // New format should remain
        assert_eq!(val["resources"]["requests"]["cpu"].as_i64().unwrap(), 2);
        assert_eq!(val["resources"]["limits"]["memory"].as_str().unwrap(), "4Gi");
    }

    #[test]
    fn test_validate_version_format_valid() {
        assert!(validate_version_format("v23.2.24").is_ok());
        assert!(validate_version_format("v25.3.1").is_ok());
        assert!(validate_version_format("v25.2.1-beta1").is_ok());
        assert!(validate_version_format("v1.0.0-rc.1").is_ok());
    }

    #[test]
    fn test_validate_version_format_invalid() {
        assert!(validate_version_format("23.2.24").is_err()); // No v prefix
        assert!(validate_version_format("v1.2").is_err()); // Incomplete
        assert!(validate_version_format("latest").is_err()); // Not semver
        assert!(validate_version_format("").is_err()); // Empty
    }

    #[tokio::test]
    async fn test_pin_versions_preserves_existing() {
        let mut val = serde_yaml::from_str(
            r#"
            image:
              tag: v23.2.24
            "#
        ).unwrap();

        pin_versions(&mut val, None, None).await.unwrap();

        assert_eq!(val["image"]["tag"].as_str().unwrap(), "v23.2.24");
    }

    #[tokio::test]
    async fn test_pin_versions_requires_flag_when_missing() {
        let mut val = serde_yaml::from_str(
            r#"
            image:
              repository: docker.redpanda.com
            "#
        ).unwrap();

        let result = pin_versions(&mut val, None, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("--redpanda-version"));
    }

    #[tokio::test]
    async fn test_pin_versions_uses_cli_flag() {
        let mut val = serde_yaml::from_str(
            r#"
            image:
              repository: docker.redpanda.com
            "#
        ).unwrap();

        pin_versions(&mut val, Some("v24.1.1".to_string()), None).await.unwrap();

        assert_eq!(val["image"]["tag"].as_str().unwrap(), "v24.1.1");
    }

    #[tokio::test]
    async fn test_pin_versions_validates_format() {
        let mut val = serde_yaml::from_str(
            r#"
            image:
              repository: docker.redpanda.com
            "#
        ).unwrap();

        let result = pin_versions(&mut val, Some("latest".to_string()), None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid version format"));
    }

    #[tokio::test]
    async fn test_pin_versions_console_only_when_enabled() {
        let mut val = serde_yaml::from_str(
            r#"
            image:
              tag: v23.2.24
            console:
              enabled: true
            "#
        ).unwrap();

        pin_versions(&mut val, None, Some("v3.3.2".to_string())).await.unwrap();

        assert_eq!(val["console"]["image"]["tag"].as_str().unwrap(), "v3.3.2");
    }

    #[tokio::test]
    async fn test_pin_versions_console_disabled() {
        let mut val = serde_yaml::from_str(
            r#"
            image:
              tag: v23.2.24
            console:
              enabled: false
            "#
        ).unwrap();

        pin_versions(&mut val, None, Some("v3.3.2".to_string())).await.unwrap();

        // Console version should not be set when console is disabled
        assert!(val.get("console").and_then(|c| c.get("image")).is_none() ||
                val["console"].get("image").and_then(|i| i.get("tag")).is_none());
    }
}

