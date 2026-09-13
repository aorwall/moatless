//! Gathering command input: message text from `--text` or stdin, JSON arguments
//! given inline or as `@file`, and file uploads.

use crate::client::MoatClient;

/// Resolve the outbound message text from `--text` or stdin.
///
/// If `text` is `Some`, returns it as-is; otherwise reads piped stdin, erroring
/// when neither is available.
pub fn read_text_or_stdin(text: Option<String>) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(t) = text {
        return Ok(t);
    }
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return Err("Provide --text or pipe message via stdin".into());
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf.trim().to_string())
}

/// Parse a JSON argument given inline or as `@path/to/file.json`.
///
/// Separate from [`read_text_or_stdin`], which trims its input — right for
/// prose, wrong for a payload.
pub fn read_json_arg<T: serde::de::DeserializeOwned>(
    flag: &str,
    arg: &str,
) -> Result<T, Box<dyn std::error::Error>> {
    let raw = match arg.strip_prefix('@') {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| format!("{flag}: cannot read {path}: {e}"))?,
        None => arg.to_string(),
    };
    serde_json::from_str(&raw).map_err(|e| format!("{flag}: {e}").into())
}

/// Upload each file path via `POST /api/v1/uploads` and return the file IDs.
pub async fn upload_files(
    moat: &MoatClient,
    paths: &[String],
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut file_ids = Vec::with_capacity(paths.len());
    for path_str in paths {
        let path = std::path::Path::new(path_str);
        if !path.exists() {
            return Err(format!("File not found: {path_str}").into());
        }
        let data = std::fs::read(path).map_err(|e| format!("Failed to read {path_str}: {e}"))?;
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unnamed");
        let mime_type = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();
        let upload_resp = moat
            .api()
            .upload_file(filename, &mime_type, data)
            .await
            .map_err(|e| format!("Failed to upload {path_str}: {e}"))?;
        file_ids.push(upload_resp.file_id);
    }
    Ok(file_ids)
}
