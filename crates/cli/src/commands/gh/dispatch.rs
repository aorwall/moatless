//! Where a `moat gh` invocation goes: the typed proxy, `gh api`, or real `gh`.

use super::repo::origin_gh_repo;
use super::typed::{self, TypedInvocation};
use super::{api, process, stack};
use crate::client::MoatClient;
use crate::config::Config;

pub async fn run(
    client: &MoatClient,
    cfg: &Config,
    args: Vec<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match typed::parse_typed(&args) {
        Some(TypedInvocation::PrCreate(a)) => typed::run_pr_create(client, cfg, a, json).await,
        Some(TypedInvocation::Comment(a)) => typed::run_comment(client, cfg, a, json).await,
        Some(TypedInvocation::PrReply(a)) => typed::run_pr_reply(client, cfg, a, json).await,
        Some(TypedInvocation::Stack(a)) => stack::run_stack(client, cfg, a, json).await,
        None if args.first().is_some_and(|a| a == "api") => {
            let gh_repo = origin_gh_repo(client).await;
            api::run_api(client, cfg, &args, gh_repo.as_deref()).await
        }
        None => {
            let gh_repo = origin_gh_repo(client).await;
            process::passthrough(&args, gh_repo.as_deref())
        }
    }
}
