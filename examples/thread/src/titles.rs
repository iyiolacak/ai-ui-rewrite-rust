//! Thread titles for the example: the first user message, shortened. A real
//! app would persist threads and ask a model here instead.

use ai_ui_core::{AdapterFuture, ThreadListAdapter, ThreadMessage};
use std::sync::Arc;

pub struct FirstMessageTitles;

fn shorten(text: &str) -> String {
    const LIMIT: usize = 28;
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= LIMIT {
        return text;
    }
    let cut: String = text.chars().take(LIMIT).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{}...", cut.trim_end_matches(['?', '.', ',', '!']))
}

impl ThreadListAdapter for FirstMessageTitles {
    fn generate_title(
        &self,
        _remote_id: &str,
        messages: Vec<Arc<ThreadMessage>>,
    ) -> AdapterFuture<Option<String>> {
        let title = messages
            .iter()
            .find(|m| m.role() == ai_ui_core::MessageRole::User)
            .map(|m| shorten(&m.text()))
            .filter(|t| !t.is_empty());
        Box::pin(async move { Ok(title) })
    }
}

#[cfg(test)]
mod tests {
    use super::shorten;

    #[test]
    fn long_text_is_cut_at_a_word() {
        assert_eq!(
            shorten("What's the weather in Tokyo?"),
            "What's the weather in Tokyo?"
        );
        assert_eq!(
            shorten("Explain how message branching works in detail please"),
            "Explain how message..."
        );
    }
}
