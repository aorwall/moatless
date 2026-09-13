//! `moat api` — make a request and report how the API answered.
//!
//! Every other command hides the transport; this one exposes it, for the two
//! cases where the status *is* the answer: a script branching on a rejection,
//! and an E2E assertion about how the API refuses something. It never fails the
//! process on a 4xx or 5xx — only a request that could not be made at all does.

use std::io::Read;

use clap::Args;

use crate::client::{ClientError, MoatClient};

#[derive(Args)]
pub struct ApiArgs {
    /// HTTP method: GET, POST, PUT, PATCH, DELETE.
    pub method: String,

    /// Path relative to the API base, e.g. `/api/v1/tasks/{id}`.
    pub path: String,

    /// JSON body. Pass `-` to read it from stdin.
    #[arg(long)]
    pub body: Option<String>,

    /// Extra header, as `Name: value`. Repeatable.
    #[arg(long = "header")]
    pub headers: Vec<String>,
}

pub async fn run(client: &MoatClient, args: ApiArgs) -> Result<(), ClientError> {
    let body = match args.body.as_deref() {
        None => None,
        Some("-") => Some(parse_json(&read_stdin()?)?),
        Some(literal) => Some(parse_json(literal)?),
    };

    let headers = args
        .headers
        .iter()
        .map(|raw| parse_header(raw))
        .collect::<Result<Vec<_>, _>>()?;

    let response = client
        .raw_request(&args.method, &args.path, body.as_ref(), &headers)
        .await?;

    // Always the same shape, so a caller can parse one thing regardless of how
    // the request went.
    let output = serde_json::json!({
        "status": response.status,
        "body": response.body,
    });
    println!("{}", serde_json::to_string(&output).unwrap_or_default());
    Ok(())
}

fn read_stdin() -> Result<String, ClientError> {
    let mut buffer = String::new();
    std::io::stdin()
        .read_to_string(&mut buffer)
        .map_err(|e| ClientError::Api {
            status: 400,
            body: format!("Failed to read the body from stdin: {e}"),
        })?;
    Ok(buffer)
}

fn parse_json(raw: &str) -> Result<serde_json::Value, ClientError> {
    serde_json::from_str(raw).map_err(|e| ClientError::Api {
        status: 400,
        body: format!("--body is not valid JSON: {e}"),
    })
}

fn parse_header(raw: &str) -> Result<(String, String), ClientError> {
    let (name, value) = raw.split_once(':').ok_or_else(|| ClientError::Api {
        status: 400,
        body: format!("--header '{raw}' is not in 'Name: value' form"),
    })?;
    Ok((name.trim().to_string(), value.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_header_pair() {
        let Ok((name, value)) = parse_header("X-Forwarded-Host: preview.example.com") else {
            panic!("a well-formed header should parse");
        };
        assert_eq!(name, "X-Forwarded-Host");
        assert_eq!(value, "preview.example.com");
    }

    #[test]
    fn rejects_a_header_without_a_colon() {
        assert!(parse_header("X-Broken").is_err());
    }

    #[test]
    fn rejects_a_body_that_is_not_json() {
        assert!(parse_json("{not json").is_err());
    }
}
