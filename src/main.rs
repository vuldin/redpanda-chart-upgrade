use serde_yaml::Value;
//use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process;

fn main() {
    // Read the first YAML file from the command-line argument.
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Please provide the path to the first YAML file as an argument.");
        process::exit(1);
    }
    let file1_path = &args[1];

    // Read and parse both YAML files.
    let file1 = fs::read_to_string(file1_path).expect("Failed to read first YAML file");
    let file2 = fs::read_to_string("test/config2.yaml").expect("Failed to read second YAML file");

    let mut data1: Value = serde_yaml::from_str(&file1).expect("Failed to parse first YAML file");
    let data2: Value = serde_yaml::from_str(&file2).expect("Failed to parse second YAML file");

    // Print the differences between the two YAML files.
    println!("Differences between the two YAML files:");
    print_diffs(&data1, &data2, 0);

    // Merge the second YAML file into the first, keeping data1's values.
    merge_keep_first(&mut data1, &data2);

    // Print the updated first YAML file.
    println!("\nUpdated version of the first YAML file:");
    let updated_yaml = serde_yaml::to_string(&data1).expect("Failed to serialize updated YAML");
    println!("{}", updated_yaml);
}

// Recursive function to print differences between two YAML values.
fn print_diffs(val1: &Value, val2: &Value, indent: usize) {
    match (val1, val2) {
        (Value::Mapping(map1), Value::Mapping(map2)) => {
            for (k, v1) in map1 {
                if let Some(v2) = map2.get(k) {
                    print_diffs(v1, v2, indent + 2);
                } else {
                    println!(
                        "{}Key '{}' is only in the first file.",
                        " ".repeat(indent),
                        k.as_str().unwrap_or("<unknown key>")
                    );
                }
            }
            for k in map2.keys() {
                if !map1.contains_key(k) {
                    println!(
                        "{}Key '{}' is only in the second file.",
                        " ".repeat(indent),
                        k.as_str().unwrap_or("<unknown key>")
                    );
                }
            }
        }
        _ => {
            if val1 != val2 {
                println!(
                    "{}Key has different values: '{:?}' vs '{:?}'.",
                    " ".repeat(indent),
                    val1,
                    val2
                );
            }
        }
    }
}


// Recursive function to merge two YAML values, keeping data1's values if they exist.
/*
fn merge_keep_first(val1: &mut Value, val2: &Value) {
    if let (Value::Mapping(map1), Value::Mapping(map2)) = (val1, val2) {
        for (k, v2) in map2 {
            let entry = map1.entry(k.clone()).or_insert(v2.clone());

            // Handle nested mappings (recursion) without moving `entry`
            if let (Value::Mapping(_), Value::Mapping(_)) = (entry, v2) {
                merge_keep_first(entry, v2);
            }
        }
    }
}
fn merge_keep_first(val1: &mut Value, val2: &Value) {
    if let (Value::Mapping(map1), Value::Mapping(map2)) = (val1, val2) {
        for (k, v2) in map2 {
            let entry = map1.entry(k.clone()).or_insert(v2.clone());

            // Use references instead of moving the values
            if let (Value::Mapping(ref mut entry_map), Value::Mapping(v2_map)) = (entry, v2) {
                // Recursively merge nested mappings
                merge_keep_first(entry, v2);
            }
        }
    }
}
*/
fn merge_keep_first(val1: &mut Value, val2: &Value) {
    if let (Value::Mapping(map1), Value::Mapping(map2)) = (val1, val2) {
        for (k, v2) in map2 {
            let entry = map1.entry(k.clone()).or_insert(v2.clone());

            // Avoid moving `entry`, only check its reference
            if let Value::Mapping(_) = entry {
                if let Value::Mapping(_) = v2 {
                    // Recursively merge nested mappings
                    merge_keep_first(entry, v2);
                }
            }
        }
    }
}

