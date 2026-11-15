//! The example's tools. They run inside the app, on the UI thread's executor,
//! so a tool can await Slint timers (here, to simulate latency).

use crate::delay::Delay;
use ai_ui_core::{Tool, ToolRegistry};
use serde_json::json;
use std::time::Duration;

/// `latency` simulates the time real tools take; zero for offscreen renders.
pub fn registry(latency: Duration) -> ToolRegistry {
    let weather = Tool::new(json!({
        "type": "object",
        "properties": { "city": { "type": "string" } },
        "required": ["city"]
    }))
    .description("Get the current weather for a city")
    .execute(move |args, _| async move {
        Delay::new(latency).await;
        let city = args
            .get("city")
            .and_then(|c| c.as_str())
            .unwrap_or("Tokyo")
            .to_owned();
        // Deterministic fake data, so the demo is repeatable.
        let temp = 12 + (city.len() as i64 * 3) % 17;
        Ok(json!({ "city": city, "temp_c": temp, "sky": "partly cloudy" }))
    });

    let email = Tool::new(json!({
        "type": "object",
        "properties": { "to": { "type": "string" }, "subject": { "type": "string" } },
        "required": ["to", "subject"]
    }))
    .description("Send an email")
    .requires_approval()
    .approval_prompt(|_, args| {
        format!(
            "Send an email to {} with the subject '{}'?",
            args.get("to").and_then(|v| v.as_str()).unwrap_or("someone"),
            args.get("subject").and_then(|v| v.as_str()).unwrap_or("")
        )
    })
    .execute(move |args, _| async move {
        Delay::new(latency).await;
        Ok(json!({ "sent": true, "to": args.get("to").cloned().unwrap_or_default() }))
    });

    ToolRegistry::new()
        .with("get_weather", weather)
        .with("send_email", email)
}
