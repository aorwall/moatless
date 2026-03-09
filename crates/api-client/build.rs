#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::Path;

fn main() {
    let spec_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../openapi-specs.json");
    let spec_path = spec_path.canonicalize().unwrap_or_else(|e| {
        panic!(
            "Cannot find openapi-specs.json at {}: {e}",
            spec_path.display()
        );
    });
    println!("cargo::rerun-if-changed={}", spec_path.display());

    let spec = std::fs::read_to_string(&spec_path).expect("read openapi-specs.json");
    let mut spec: serde_json::Value =
        serde_json::from_str(&spec).expect("parse openapi-specs.json");

    // Downconvert OpenAPI 3.1.0 → 3.0.3 for progenitor/openapiv3 compatibility
    downconvert_31_to_30(&mut spec);

    // Remove endpoints with multipart/form-data (unsupported by progenitor)
    strip_multipart_endpoints(&mut spec);

    let spec_str = serde_json::to_string(&spec).expect("re-serialize spec");
    let spec: openapiv3::OpenAPI =
        serde_json::from_str(&spec_str).expect("parse downconverted spec as OpenAPI");

    let mut generator = progenitor::Generator::default();
    let tokens = generator.generate_tokens(&spec).expect("generate client");
    let ast = syn::parse2(tokens).expect("parse generated tokens");
    let content = prettyplease::unparse(&ast);

    let out_file = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("codegen.rs");
    std::fs::write(out_file, content).expect("write codegen.rs");
}

/// Remove paths that use multipart/form-data (unsupported by progenitor).
fn strip_multipart_endpoints(spec: &mut serde_json::Value) {
    if let Some(paths) = spec.get_mut("paths").and_then(|p| p.as_object_mut()) {
        let to_remove: Vec<String> = paths
            .iter()
            .filter(|(_, path_item)| {
                let json = serde_json::to_string(path_item).unwrap_or_default();
                json.contains("multipart/form-data")
            })
            .map(|(k, _)| k.clone())
            .collect();
        for key in to_remove {
            paths.remove(&key);
        }
    }
}

/// Convert OpenAPI 3.1.0 idioms to 3.0.3 equivalents:
/// - `"openapi": "3.1.0"` → `"3.0.3"`
/// - `"type": ["string", "null"]` → `"type": "string", "nullable": true`
/// - `"oneOf": [{"type": "null"}, {"$ref": "..."}]` → `"$ref": "...", "nullable": true`
/// - `"const": "value"` → `"enum": ["value"]`
fn downconvert_31_to_30(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            // Fix version at top level
            if let Some(v) = map.get_mut("openapi") {
                if v.as_str() == Some("3.1.0") {
                    *v = serde_json::Value::String("3.0.3".to_string());
                }
            }

            // Convert "type": ["string", "null"] → "type": "string", "nullable": true
            if let Some(type_val) = map.get("type") {
                if let Some(arr) = type_val.as_array() {
                    let types: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                    if types.len() == 2 && types.contains(&"null") {
                        let real_type = types.iter().find(|&&t| t != "null").unwrap_or(&"string");
                        map.insert(
                            "type".to_string(),
                            serde_json::Value::String(real_type.to_string()),
                        );
                        map.insert("nullable".to_string(), serde_json::Value::Bool(true));
                    } else if types.len() == 1 {
                        // Single-element array: just unwrap
                        map.insert(
                            "type".to_string(),
                            serde_json::Value::String(types[0].to_string()),
                        );
                    }
                }
            }

            // Convert "oneOf": [{"type": "null"}, {"$ref": "..."}] → nullable $ref
            if let Some(one_of) = map.get("oneOf") {
                if let Some(arr) = one_of.as_array() {
                    if arr.len() == 2 {
                        let null_idx = arr.iter().position(|v| {
                            v.get("type").and_then(|t| t.as_str()) == Some("null")
                        });
                        if let Some(ni) = null_idx {
                            let other_idx = 1 - ni;
                            let other = &arr[other_idx];
                            if let Some(ref_val) = other.get("$ref") {
                                // Replace oneOf with direct $ref + nullable
                                let ref_val = ref_val.clone();
                                map.remove("oneOf");
                                // Wrap in allOf for nullable ref (3.0.x pattern)
                                map.insert(
                                    "allOf".to_string(),
                                    serde_json::json!([{"$ref": ref_val}]),
                                );
                                map.insert("nullable".to_string(), serde_json::Value::Bool(true));
                            }
                        }
                    }
                }
            }

            // Convert "const": "value" → "enum": ["value"]
            if let Some(const_val) = map.remove("const") {
                map.insert(
                    "enum".to_string(),
                    serde_json::Value::Array(vec![const_val]),
                );
            }

            // Recurse into all values
            for v in map.values_mut() {
                downconvert_31_to_30(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                downconvert_31_to_30(v);
            }
        }
        _ => {}
    }
}
