//! Kubernetes resource-quantity string parsers, shared by both providers.
//!
//! These parse the Kubernetes quantity grammar (`"250m"`, `"128Mi"`) into plain
//! integers. They carry no Kubernetes Rust types — only `&str -> u64` — so they
//! live in the sandbox-control band, where both the k8s and docker providers read
//! them downward.

/// Parse K8s CPU quantity (e.g. "250m", "1", "100n") to millicores.
pub fn parse_cpu_to_millicores(s: &str) -> u64 {
    if let Some(nano) = s.strip_suffix('n') {
        // Nanocores → millicores
        nano.parse::<u64>().unwrap_or(0) / 1_000_000
    } else if let Some(micro) = s.strip_suffix('u') {
        micro.parse::<u64>().unwrap_or(0) / 1_000
    } else if let Some(milli) = s.strip_suffix('m') {
        milli.parse::<u64>().unwrap_or(0)
    } else {
        // Whole cores
        s.parse::<f64>().map_or(0, |v| (v * 1000.0) as u64)
    }
}

/// Parse K8s memory quantity (e.g. "128Mi", "1Gi", "256000Ki") to bytes.
pub fn parse_memory_to_bytes(s: &str) -> u64 {
    if let Some(v) = s.strip_suffix("Ki") {
        v.parse::<u64>().unwrap_or(0) * 1024
    } else if let Some(v) = s.strip_suffix("Mi") {
        v.parse::<u64>().unwrap_or(0) * 1024 * 1024
    } else if let Some(v) = s.strip_suffix("Gi") {
        v.parse::<u64>().unwrap_or(0) * 1024 * 1024 * 1024
    } else if let Some(v) = s.strip_suffix('k') {
        v.parse::<u64>().unwrap_or(0) * 1000
    } else if let Some(v) = s.strip_suffix('M') {
        v.parse::<u64>().unwrap_or(0) * 1_000_000
    } else if let Some(v) = s.strip_suffix('G') {
        v.parse::<u64>().unwrap_or(0) * 1_000_000_000
    } else {
        // Plain bytes
        s.parse::<u64>().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_quantities_parse_to_millicores() {
        assert_eq!(parse_cpu_to_millicores("250m"), 250);
        assert_eq!(parse_cpu_to_millicores("1"), 1000);
        assert_eq!(parse_cpu_to_millicores("2"), 2000);
        assert_eq!(parse_cpu_to_millicores("500000u"), 500);
        assert_eq!(parse_cpu_to_millicores("1000000000n"), 1000);
        // Unparseable pieces degrade to 0 rather than panicking.
        assert_eq!(parse_cpu_to_millicores("garbage"), 0);
    }

    #[test]
    fn memory_quantities_parse_to_bytes() {
        assert_eq!(parse_memory_to_bytes("128Mi"), 128 * 1024 * 1024);
        assert_eq!(parse_memory_to_bytes("1Gi"), 1024 * 1024 * 1024);
        assert_eq!(parse_memory_to_bytes("256000Ki"), 256_000 * 1024);
        assert_eq!(parse_memory_to_bytes("2k"), 2000);
        assert_eq!(parse_memory_to_bytes("5M"), 5_000_000);
        assert_eq!(parse_memory_to_bytes("3G"), 3_000_000_000);
        assert_eq!(parse_memory_to_bytes("4096"), 4096);
        assert_eq!(parse_memory_to_bytes("garbage"), 0);
    }
}
