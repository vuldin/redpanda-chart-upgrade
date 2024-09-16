use serde_yaml::Value;
use std::env;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process;
use reqwest;

const LATEST_CHART_VALUES_URL: &str = "https://raw.githubusercontent.com/redpanda-data/helm-charts/main/charts/redpanda/values.yaml";

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

    // Print the differences between the two YAML files
    println!("Differences between the two files:");
    print_diffs(&data1, &data2, 0);

    // Merge the second YAML file into the first, keeping data1's values
    merge(&mut data1, &data2);

    // Serialize the merged YAML to a string
    let updated_yaml = serde_yaml::to_string(&data1).expect("Failed to serialize the updated YAML");

    // Write the merged YAML to a file with a unique name
    let output_file = get_unique_filename("updated-values.yaml");
    let mut file = File::create(&output_file).expect("Failed to create the output file");
    file.write_all(updated_yaml.as_bytes()).expect("Failed to write to the output file");

    println!("\nMerged YAML written to: {}", output_file);
}

// Recursive function to print differences between two YAML values
fn print_diffs(val1: &Value, val2: &Value, indent: usize) {
    match (val1, val2) {
        (Value::Mapping(map1), Value::Mapping(map2)) => {
            for (k, v1) in map1 {
                if let Some(v2) = map2.get(k) {
                    print_diffs(v1, v2, indent + 2);
                } else {
                    println!(
                        "{}Key '{}' is only in the existing deployment config.",
                        " ".repeat(indent),
                        k.as_str().unwrap_or("<unknown key>")
                    );
                }
            }
            for k in map2.keys() {
                if !map1.contains_key(k) {
                    println!(
                        "{}Key '{}' is only in the latest config.",
                        " ".repeat(indent),
                        k.as_str().unwrap_or("<unknown key>")
                    );
                }
            }
        }
        _ => {
            if val1 != val2 {
                println!(
                    "{}Key has different values. existing: '{:?}' vs latest: '{:?}'.",
                    " ".repeat(indent),
                    val1,
                    val2
                );
            }
        }
    }
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

fn rename_nested_keys(val: &mut Value) {
    if let Value::Mapping(map) = val {
        // Recursively traverse the nested mappings
        for (_, v) in map.iter_mut() {
            rename_nested_keys(v);
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

