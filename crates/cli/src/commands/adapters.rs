//! Adapter connection and binding commands.

use clap::Subcommand;
use moatless_api_schemas::adapters::CreateAdapterConnectionRequest;
use moatless_api_schemas::bindings::CreateTaskBindingRequest;
use moatless_api_schemas::enums::AdapterKind;

use crate::client::{ClientError, MoatClient};
use crate::output;

#[derive(Subcommand)]
pub enum AdaptersAction {
    /// Manage adapter connections
    Connections {
        #[command(subcommand)]
        action: ConnectionAction,
    },
    /// List supported adapters configured on the backend
    List,
    /// Launch the OAuth install flow for an adapter.
    ///
    /// Opens the backend's install URL in your browser (`--print` copies/pastes
    /// instead). Currently supports `linear`.
    Install {
        #[arg(long)]
        adapter: String,
        /// Print the install URL instead of trying to open it in a browser.
        #[arg(long, default_value_t = false)]
        print: bool,
    },
}

#[derive(Subcommand)]
pub enum ConnectionAction {
    /// List adapter connections
    List {
        #[arg(long)]
        adapter: Option<String>,
    },
    /// Create or update an adapter connection
    Create {
        #[arg(long)]
        adapter: String,
        #[arg(long)]
        external_account_id: String,
        #[arg(long)]
        connection_kind: Option<String>,
        #[arg(long)]
        webhook_secret: Option<String>,
        #[arg(long)]
        transport_config: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum BindingAction {
    /// List task bindings
    List {
        #[arg(long)]
        task: String,
    },
    /// Create a task binding
    Create {
        #[arg(long)]
        task: String,
        #[arg(long)]
        adapter: String,
        #[arg(long)]
        connection_id: String,
        #[arg(long)]
        subject_kind: String,
        #[arg(long)]
        subject_key: String,
        #[arg(long)]
        loop_id: Option<String>,
        #[arg(long)]
        destination_config: Option<String>,
        #[arg(long)]
        metadata: Option<String>,
    },
}

pub async fn run_adapters(
    client: &MoatClient,
    action: AdaptersAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        AdaptersAction::Connections { action } => run_connections(client, action, json).await,
        AdaptersAction::List => {
            let adapters = client.api().list_adapters().await?;
            if json {
                output::print_json_typed(&adapters);
            } else {
                for adapter in adapters {
                    println!("{adapter}");
                }
            }
            Ok(())
        }
        AdaptersAction::Install { adapter, print } => run_install(client, &adapter, print),
    }
}

fn run_install(client: &MoatClient, adapter: &str, print_only: bool) -> Result<(), ClientError> {
    let kind = parse_adapter_kind(adapter)?;
    let path = match kind {
        AdapterKind::Linear => "/api/v1/auth/linear/install",
        other => {
            return Err(ClientError::Other(format!(
                "Install flow not supported for adapter '{}'",
                other.as_str()
            )));
        }
    };
    let url = format!("{}{path}", client.base_url().trim_end_matches('/'));
    if print_only {
        println!("{url}");
        return Ok(());
    }
    println!("Opening {url} in your browser…");
    if let Err(e) = open::that(&url) {
        println!("Could not open browser automatically ({e}). Visit this URL manually:\n  {url}");
    }
    Ok(())
}

async fn run_connections(
    client: &MoatClient,
    action: ConnectionAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        ConnectionAction::List { adapter } => {
            let items = client
                .api()
                .list_adapter_connections(adapter.as_deref())
                .await?;
            if json {
                output::print_json_typed(&items);
            } else {
                for item in items {
                    println!(
                        "{}\t{}\t{}\t{}",
                        item.id, item.adapter_kind, item.connection_kind, item.external_account_id
                    );
                }
            }
        }
        ConnectionAction::Create {
            adapter,
            external_account_id,
            connection_kind,
            webhook_secret,
            transport_config,
        } => {
            let req = CreateAdapterConnectionRequest {
                adapter_kind: parse_adapter_kind(&adapter)?,
                connection_kind,
                external_account_id,
                webhook_secret,
                source_name: None,
                transport_config: parse_json_opt(transport_config.as_deref())?,
                scope: None,
            };
            let created = client.api().create_adapter_connection(&req).await?;
            if json {
                output::print_json_typed(&created);
            } else {
                println!("Connection upserted: {}", created.id);
            }
        }
    }
    Ok(())
}

pub async fn run_bindings(
    client: &MoatClient,
    action: BindingAction,
    json: bool,
) -> Result<(), ClientError> {
    match action {
        BindingAction::List { task } => {
            let items = client.api().list_task_bindings(&task).await?;
            print_or_json(&items, json);
        }
        BindingAction::Create {
            task,
            adapter,
            connection_id,
            subject_kind,
            subject_key,
            loop_id,
            destination_config,
            metadata,
        } => {
            let req = CreateTaskBindingRequest {
                adapter_kind: parse_adapter_kind(&adapter)?,
                connection_id,
                loop_id,
                subject_kind,
                subject_key,
                destination_config: parse_json_opt(destination_config.as_deref())?,
                metadata: parse_json_opt(metadata.as_deref())?,
                origin: Some("api".to_string()),
            };
            let created = client.api().create_task_binding(&task, &req).await?;
            print_or_json(&created, json);
        }
    }
    Ok(())
}

pub(crate) fn parse_adapter_kind(value: &str) -> Result<AdapterKind, ClientError> {
    AdapterKind::parse(value).ok_or_else(|| {
        ClientError::Other(format!(
            "Invalid adapter '{value}'. Expected slack, telegram, github, linear, or webhook"
        ))
    })
}

pub(crate) fn parse_json_opt(
    value: Option<&str>,
) -> Result<Option<serde_json::Value>, ClientError> {
    value
        .map(|raw| {
            serde_json::from_str(raw)
                .map_err(|e| ClientError::Other(format!("Invalid JSON '{raw}': {e}")))
        })
        .transpose()
}

pub(crate) fn print_or_json<T>(value: &T, json: bool)
where
    T: serde::Serialize + std::fmt::Debug,
{
    if json {
        output::print_json_typed(value);
    } else {
        println!("{value:#?}");
    }
}
