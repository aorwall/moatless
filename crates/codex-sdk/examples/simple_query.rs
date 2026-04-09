use codex_sdk::{CodexMessage, CodexOptions, query};

#[tokio::main]
async fn main() -> codex_sdk::Result<()> {
    let mut handle = query(
        "List the files in the current directory",
        CodexOptions::default(),
    )
    .await?;

    while let Some(msg) = handle.messages.recv().await {
        match msg? {
            CodexMessage::AgentMessageDelta(delta) => {
                print!("{}", delta.delta);
            }
            CodexMessage::TurnCompleted(_) => {
                println!("\n--- Turn completed ---");
                break;
            }
            _ => {}
        }
    }

    Ok(())
}
