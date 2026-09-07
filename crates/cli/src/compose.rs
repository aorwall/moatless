use moatless_cli::{client::MoatClient, config::Config};

pub(super) fn client(config: &Config, verbose: bool, timeout: Option<u64>) -> MoatClient {
    let mut client = MoatClient::new(config, verbose);
    if let Some(seconds) = timeout {
        client.set_http_timeout(std::time::Duration::from_secs(seconds));
    }
    client
}
