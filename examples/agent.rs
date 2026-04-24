use ai_agent::{Agent, EnvConfig};
use tokio::runtime::Runtime;

fn main() {
    // Load config from .env file and environment variables
    let config = EnvConfig::load();

    println!("Config loaded:");
    println!("  AI_BASE_URL: {:?}", config.base_url);
    println!("  AI_AUTH_TOKEN: {:?}", config.auth_token.as_ref().map(|_| "***"));
    println!("  AI_MODEL: {:?}", config.model);

    let runtime = Runtime::new().expect("Failed to create runtime");

    // Pass model from config, or fallback to "sonnet"
    let model = config.model.as_deref().unwrap_or("sonnet");
    let agent = Agent::new(model).max_turns(10);

    match runtime.block_on(agent.query("Say hello in one sentence")) {
        Ok(result) => {
            println!("Response: {}", result.text);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}