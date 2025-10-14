use serde_yaml::Value;
use std::env;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process;
use reqwest;

const LATEST_CHART_VALUES_URL: &str = "https://raw.githubusercontent.com/redpanda-data/redpanda-operator/refs/heads/main/charts/redpanda/chart/values.yaml";

#[tokio::main]
async fn main() {
    // Get the path to the existing deployment config file
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Provide the path to the existing deployment's values.yaml file:");
        process::exit(1);
    }
    let file1_path = &args[1];

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

    // SECOND: Clean up deprecated fields after migration (but before merge)
    clean_deprecated_fields(&mut data1);

    // Merge the second YAML file into the first, keeping data1's values
    merge(&mut data1, &data2);

    // THIRD: Clean up again AFTER merge to remove any empty values added back by merge
    clean_empty_cloud_storage(&mut data1);
    clean_old_resource_format(&mut data1);

    // FOURTH: Validate and harden tiered storage configuration
    validate_and_fix_tiered_storage(&mut data1);

    // Serialize the merged YAML to a string
    let updated_yaml = serde_yaml::to_string(&data1).expect("Failed to serialize the updated YAML");

    // Write the merged YAML to a file with a unique name
    let output_file = get_unique_filename("updated-values.yaml");
    let mut file = File::create(&output_file).expect("Failed to create the output file");
    file.write_all(updated_yaml.as_bytes()).expect("Failed to write to the output file");

    println!("\n=== Conversion Complete ===");
    println!("  ✓ Output file: {}", output_file);
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
        let mut security_context = None;
        let mut priority_class_name = None;
        let mut topology_spread_constraints = None;
        let mut termination_grace_period = None;

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
        }

        // Now create or update podTemplate with the extracted values
        if root_node_selector.is_some() || root_tolerations.is_some() || root_affinity.is_some() ||
           node_selector.is_some() || tolerations.is_some() || pod_affinity.is_some() ||
           security_context.is_some() || priority_class_name.is_some() ||
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
                    if let Some(sc) = security_context {
                        spec_map.insert(Value::String("securityContext".to_string()), sc);
                    }
                    if let Some(pc) = priority_class_name {
                        spec_map.insert(Value::String("priorityClassName".to_string()), pc);
                    }
                    if let Some(tsc) = topology_spread_constraints {
                        spec_map.insert(Value::String("topologySpreadConstraints".to_string()), tsc);
                    }
                    if let Some(tgp) = termination_grace_period {
                        spec_map.insert(Value::String("terminationGracePeriodSeconds".to_string()), tgp);
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

            // If we found old format resources, convert them
            if cpu_value.is_some() || memory_value.is_some() {
                println!("\n=== Resource Format Conversion ===");
                if let Some(Value::Mapping(resources_map)) = map.get_mut(&Value::String("resources".to_string())) {
                    // Remove old format structures
                    resources_map.remove(&Value::String("cpu".to_string()));
                    resources_map.remove(&Value::String("memory".to_string()));

                    // Create new requests and limits mappings
                    let mut requests_map = serde_yaml::Mapping::new();
                    let mut limits_map = serde_yaml::Mapping::new();

                    if let Some(cpu) = &cpu_value {
                        println!("  ✓ Converting resources.cpu.cores → resources.requests.cpu & resources.limits.cpu (value: {:?})", cpu);
                        requests_map.insert(Value::String("cpu".to_string()), cpu.clone());
                        limits_map.insert(Value::String("cpu".to_string()), cpu.clone());
                    }

                    if let Some(memory) = &memory_value {
                        println!("  ✓ Converting resources.memory.container.max → resources.requests.memory & resources.limits.memory (value: {:?})", memory);
                        requests_map.insert(Value::String("memory".to_string()), memory.clone());
                        limits_map.insert(Value::String("memory".to_string()), memory.clone());
                    }

                    println!("  ℹ Note: Requests and limits are set to matching values for production readiness");

                    // Set requests and limits (matching for production readiness)
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

