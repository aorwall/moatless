use codex_sdk::{CodexClient, CodexMessage, CodexOptions};

#[tokio::main]
async fn main() -> codex_sdk::Result<()> {
    let mut client = CodexClient::new(CodexOptions::default()).await?;

    println!("Thread: {}", client.thread_id());

    // First turn.
    client.send("What is 2 + 2?").await?;

    while let Some(msg) = client.recv().await {
        match msg? {
            CodexMessage::AgentMessageDelta(delta) => {
                print!("{}", delta.delta);
            }
            CodexMessage::TurnCompleted(_) => {
                println!();
                break;
            }
            _ => {}
        }
    }

    // Second turn.
    client.send("Now multiply that by 3").await?;

    while let Some(msg) = client.recv().await {
        match msg? {
            CodexMessage::AgentMessageDelta(delta) => {
                print!("{}", delta.delta);
            }
            CodexMessage::TurnCompleted(_) => {
                println!();
                break;
            }
            _ => {}
        }
    }

    client.cancel();
    Ok(())
}
