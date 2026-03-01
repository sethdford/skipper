//! Provider helper functions.

/// Map a provider name to its conventional environment variable name.
pub fn provider_to_env_var(provider: &str) -> String {
    match provider.to_lowercase().as_str() {
        "groq" => "GROQ_API_KEY".to_string(),
        "anthropic" => "ANTHROPIC_API_KEY".to_string(),
        "openai" => "OPENAI_API_KEY".to_string(),
        "gemini" => "GEMINI_API_KEY".to_string(),
        "google" => "GOOGLE_API_KEY".to_string(),
        "deepseek" => "DEEPSEEK_API_KEY".to_string(),
        "openrouter" => "OPENROUTER_API_KEY".to_string(),
        "together" => "TOGETHER_API_KEY".to_string(),
        "mistral" => "MISTRAL_API_KEY".to_string(),
        "fireworks" => "FIREWORKS_API_KEY".to_string(),
        "perplexity" => "PERPLEXITY_API_KEY".to_string(),
        "cohere" => "COHERE_API_KEY".to_string(),
        "xai" => "XAI_API_KEY".to_string(),
        "brave" => "BRAVE_API_KEY".to_string(),
        "tavily" => "TAVILY_API_KEY".to_string(),
        other => format!("{}_API_KEY", other.to_uppercase()),
    }
}

/// Test an API key by hitting the provider's models/health endpoint.
///
/// Returns true if the key is accepted (status != 401/403).
/// Returns true on timeout/network errors (best-effort — don't block setup).
pub fn test_api_key(provider: &str, env_var: &str) -> bool {
    let key = match std::env::var(env_var) {
        Ok(k) => k,
        Err(_) => return false,
    };

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return true, // can't build client — assume ok
    };

    let result = match provider.to_lowercase().as_str() {
        "groq" => client
            .get("https://api.groq.com/openai/v1/models")
            .bearer_auth(&key)
            .send(),
        "anthropic" => client
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", &key)
            .header("anthropic-version", "2023-06-01")
            .send(),
        "openai" => client
            .get("https://api.openai.com/v1/models")
            .bearer_auth(&key)
            .send(),
        "gemini" | "google" => client
            .get(format!(
                "https://generativelanguage.googleapis.com/v1beta/models?key={key}"
            ))
            .send(),
        "deepseek" => client
            .get("https://api.deepseek.com/models")
            .bearer_auth(&key)
            .send(),
        "openrouter" => client
            .get("https://openrouter.ai/api/v1/models")
            .bearer_auth(&key)
            .send(),
        _ => return true, // unknown provider — skip test
    };

    match result {
        Ok(resp) => {
            let status = resp.status().as_u16();
            status != 401 && status != 403
        }
        Err(_) => true, // network error — don't block setup
    }
}
