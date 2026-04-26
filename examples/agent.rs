// examples/agent.rs
use ai_agent::{Agent, EnvConfig};
use futures::StreamExt;
use tokio::runtime::Builder;

fn main() {
    // Load config from .env file and environment variables
    let config = EnvConfig::load();

    println!("Config loaded:");
    println!("  AI_BASE_URL: {:?}", config.base_url);
    println!("  AI_AUTH_TOKEN: {:?}", config.auth_token.as_ref().map(|_| "***"));
    println!("  AI_MODEL: {:?}", config.model);

    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create runtime");
    let local_set = tokio::task::LocalSet::new();

    local_set.block_on(&runtime, async {
        // Pass model from config, or fallback to "sonnet"
        let model = config.model.as_deref().unwrap_or("sonnet");
        let agent = Agent::new(model)
            .disallowed_tools(vec![
                String::from("SendUserMessage"),
                String::from("StructuredOutput"),
                String::from("AskUserQuestion"),
            ])
            .max_turns(10);

        let (sub, _guard) = agent.subscribe();
        tokio::pin!(sub);

        let query = agent.query("Say hello in one sentence");
        tokio::pin!(query);

        tokio::select! {
            result = &mut query => {
                match result {
                    Ok(r) => {
                        println!("Response: {}", r.text);
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            Some(event) = sub.next() => {
                eprintln!("[event] {:?}", std::mem::discriminant(&event));
            }
        }
    });
}
