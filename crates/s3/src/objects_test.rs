use std::error::Error;
use std::time::Duration;

use aws_sdk_s3::config::{Credentials, Region, retry::RetryConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::{
    S3Error, delete_objects_by_prefix, get_object, get_object_opt, head_object_modified_at,
};

async fn endpoint(
    exchanges: Vec<(&'static str, String)>,
) -> (aws_sdk_s3::Client, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(10), async move {
            for (expected, response) in exchanges {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let header_end = loop {
                    let byte = socket.read_u8().await.unwrap();
                    request.push(byte);
                    if request.ends_with(b"\r\n\r\n") {
                        break request.len();
                    }
                };
                let headers = String::from_utf8_lossy(&request).to_lowercase();
                assert!(headers.contains(expected), "{headers}");
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .map_or(0, |length| length.trim().parse::<usize>().unwrap());
                request.resize(header_end + length, 0);
                socket.read_exact(&mut request[header_end..]).await.unwrap();
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        })
        .await
        .unwrap();
    });
    let config = aws_sdk_s3::Config::builder()
        .behavior_version_latest()
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new("test", "test", None, None, "test"))
        .endpoint_url(format!("http://{address}"))
        .force_path_style(true)
        .retry_config(RetryConfig::standard().with_max_attempts(1))
        .build();
    (aws_sdk_s3::Client::from_conf(config), server)
}

fn response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn absent_objects_are_distinct_from_failed_reads() {
    let (client, server) = endpoint(vec![
        (
            "get /bucket/missing",
            response("404 Not Found", "<Error><Code>NoSuchKey</Code></Error>"),
        ),
        ("head /bucket/missing", response("404 Not Found", "")),
        (
            "get /bucket/denied",
            response("403 Forbidden", "<Error><Code>AccessDenied</Code></Error>"),
        ),
        ("head /bucket/denied", response("403 Forbidden", "")),
        ("get /bucket/present", response("200 OK", "payload")),
    ])
    .await;
    assert!(
        get_object_opt(&client, "bucket", "missing")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        head_object_modified_at(&client, "bucket", "missing")
            .await
            .unwrap()
            .is_none()
    );
    let error = get_object_opt(&client, "bucket", "denied")
        .await
        .unwrap_err();
    assert!(matches!(&error, S3Error::GetObject(source) if source.as_service_error().is_some()));
    assert!(error.source().is_some());
    assert!(matches!(
        head_object_modified_at(&client, "bucket", "denied").await,
        Err(S3Error::HeadObject(_))
    ));
    assert_eq!(
        get_object(&client, "bucket", "present").await.unwrap(),
        (b"payload".to_vec(), "application/octet-stream".to_string())
    );
    server.await.unwrap();
}

#[tokio::test]
async fn pagination_retains_every_batch_failure() {
    let (client, server) = endpoint(vec![
        ("get /bucket/?", response("200 OK", "<ListBucketResult><IsTruncated>true</IsTruncated><NextContinuationToken>next</NextContinuationToken><Contents><Key>first</Key></Contents></ListBucketResult>")),
        ("post /bucket/?delete", response("200 OK", "<DeleteResult/>")),
        ("continuation-token=next", response("200 OK", "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>second</Key></Contents><Contents><Key>third</Key></Contents></ListBucketResult>")),
        ("post /bucket/?delete", response("200 OK", "<DeleteResult><Error><Key>second</Key><Code>AccessDenied</Code><Message>denied</Message></Error><Error><Key>third</Key><Code>InternalError</Code><Message>retry</Message></Error></DeleteResult>")),
    ]).await;
    let error = delete_objects_by_prefix(&client, "bucket", "prefix/")
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "S3 delete_objects left 2 of 2 keys: second: denied"
    );
    let S3Error::BatchDelete {
        attempted,
        failures,
    } = error
    else {
        panic!("expected partial deletion");
    };
    assert_eq!(attempted, 2);
    assert_eq!(failures.len(), 2);
    assert_eq!(failures[0].key(), Some("second"));
    assert_eq!(failures[0].code(), Some("AccessDenied"));
    assert_eq!(failures[1].key(), Some("third"));
    assert_eq!(failures[1].code(), Some("InternalError"));
    server.await.unwrap();
}

#[test]
fn body_and_credentials_failures_keep_their_sources() {
    let body = S3Error::ReadBody(std::io::Error::other("body interrupted").into());
    assert!(body.source().unwrap().source().is_some());
    let credentials = S3Error::Credentials(Box::new(std::io::Error::other("provider unavailable")));
    assert_eq!(
        credentials.source().unwrap().to_string(),
        "provider unavailable"
    );
}

#[tokio::test]
async fn metadata_and_truncated_body_failures_remain_errors() {
    let (client, server) = endpoint(vec![
        ("head /bucket/missing-date", response("200 OK", "")),
        ("head /bucket/dated", "HTTP/1.1 200 OK\r\nLast-Modified: Wed, 21 Oct 2015 07:28:00 GMT\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()),
        ("get /bucket/truncated", "HTTP/1.1 200 OK\r\nContent-Length: 50\r\nConnection: close\r\n\r\nshort".to_string()),
    ]).await;
    assert!(
        matches!(head_object_modified_at(&client, "bucket", "missing-date").await, Err(S3Error::MissingLastModified { key }) if key == "missing-date")
    );
    assert_eq!(
        head_object_modified_at(&client, "bucket", "dated")
            .await
            .unwrap()
            .unwrap()
            .timestamp(),
        1_445_412_480
    );
    let error = get_object(&client, "bucket", "truncated")
        .await
        .unwrap_err();
    assert!(matches!(&error, S3Error::ReadBody(_)));
    assert!(error.source().unwrap().source().is_some());
    server.await.unwrap();
}
