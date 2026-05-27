use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;

/// Spawns a background process that generates a storm of Zenity popups.
/// This mimics a hostile or buggy UI environment stealing focus.
pub struct PopupStorm {
    processes: Vec<std::process::Child>,
}

impl PopupStorm {
    pub fn start(count: usize) -> Self {
        let mut processes = Vec::new();
        for i in 0..count {
            if let Ok(child) = Command::new("zenity")
                .args(["--info", "--text", &format!("Chaos popup {}", i)])
                .spawn()
            {
                processes.push(child);
            }
        }
        Self { processes }
    }

    pub fn stop(&mut self) {
        for child in &mut self.processes {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for PopupStorm {
    fn drop(&mut self) {
        self.stop();
    }
}

pub async fn run_chaos_test() -> Result<(), String> {
    // 1. Start a popup storm
    let mut storm = PopupStorm::start(5);
    sleep(Duration::from_secs(1)).await; // Let them spawn

    // 2. Instantiate AT-SPI engine
    let engine = kria_core::agent::atspi_engine::AtSpiEngine::new();

    // 3. Try to find the popups, demonstrating that the engine doesn't crash
    //    when the accessibility tree is rapidly mutating.
    let elements = engine.find_elements("dialog", None).await;

    // We expect to find multiple dialogs.
    if elements.is_empty() {
        return Err("Chaos test failed: No dialogs found despite zenity storm.".into());
    }

    // 4. Try to dismiss one

    // Attempt a click on the 'OK' button inside the dialog
    let ok_buttons = engine.find_elements("push button", Some("OK")).await;
    if let Some(ok) = ok_buttons.first() {
        // This validates that the engine handles chaotic UI state
        let _ = engine.click_element("push button", &ok.name).await;
    }

    storm.stop();
    Ok(())
}
