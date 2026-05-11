use std::time::Duration;

pub struct LlmFixture {
    child: tokio::process::Child,
}

impl Drop for LlmFixture {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

impl LlmFixture {
    pub async fn start() -> Result<Option<Self>, String> {
        let cmd = match std::env::var("KRIA_EVAL_LLM_CMD") {
            Ok(value) if !value.trim().is_empty() => value,
            _ => {
                eprintln!(
                    "Warning: KRIA_EVAL_LLM_CMD is not set. Skipping LLM fixture startup; using external backend if available."
                );
                return Ok(None);
            }
        };

        let health_url = std::env::var("KRIA_EVAL_LLM_HEALTH_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080/v1/models".to_string());

        let preflight_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .map_err(|error| format!("Failed to build pre-flight HTTP client: {error}"))?;

        if let Ok(response) = preflight_client.get(&health_url).send().await {
            if response.status() == reqwest::StatusCode::OK {
                println!(
                    "🟢 Detected existing LLM backend running at {}. Re-using it.",
                    health_url
                );
                return Ok(None);
            }
        }

        let parts: Vec<String> = cmd
            .split_whitespace()
            .map(|part| part.to_string())
            .collect();
        if parts.is_empty() {
            return Err("KRIA_EVAL_LLM_CMD is empty after parsing".to_string());
        }

        let mut command = tokio::process::Command::new(&parts[0]);
        if parts.len() > 1 {
            command.args(&parts[1..]);
        }
        command
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());

        let mut child = command
            .spawn()
            .map_err(|error| format!("Failed to spawn KRIA_EVAL_LLM_CMD '{cmd}': {error}"))?;

        let client = reqwest::Client::new();

        for attempt in 1..=30 {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!(
                    "LLM process crashed prematurely with status: {status}. Check terminal for GPU OOM or port conflicts."
                ));
            }

            match client.get(&health_url).send().await {
                Ok(response) if response.status() == reqwest::StatusCode::OK => {
                    println!("LLM is ready!");
                    return Ok(Some(Self { child }));
                }
                Ok(response) => {
                    eprintln!(
                        "Waiting for LLM health endpoint (attempt {attempt}/30): status {}",
                        response.status()
                    );
                }
                Err(error) => {
                    eprintln!("Waiting for LLM health endpoint (attempt {attempt}/30): {error}");
                }
            }

            if attempt < 30 {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }

        let _ = child.start_kill();
        Err(format!(
            "HTTP polling timed out waiting for LLM readiness at {health_url} after 30 attempts"
        ))
    }
}
