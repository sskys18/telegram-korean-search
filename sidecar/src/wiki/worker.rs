//! Wiki classification worker.
//!
//! This module previously ran inside the Tauri process and emitted
//! progress events through the Tauri event bus. The sidecar will need
//! a store-owned worker that reports progress through the IPC channel
//! instead. The original implementation is preserved in git history
//! at tag `archive/tauri-v0` and will be ported in a later phase once
//! the IPC contract is defined.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::store::wiki_topic::{normalize_topic_title, NewTopic, TopicMessageLink};
use crate::wiki::llm::ClassifiedTopic;

/// Placeholder entry point. Wire up in the IPC phase.
pub fn start_worker() -> (Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    let shutdown = Arc::new(AtomicBool::new(false));
    let handle = std::thread::spawn(|| {
        log::warn!("wiki::worker::start_worker is a stub; no work will be done");
    });
    (shutdown, handle)
}

/// Retained so the topic-processing logic stays in tree for the rewrite.
pub fn process_classified_topic(
    store: &crate::store::Store,
    classified: &ClassifiedTopic,
    chat_id: i64,
    message_id: i64,
    message_timestamp: i64,
) -> Result<(), sqlite::Error> {
    let (category_id, canonical_category) = store
        .resolve_category_with_name(&classified.category, classified.category_ko.as_deref())?;

    let topic_id = match store.find_topic_by_alias(&classified.topic)? {
        Some(id) => {
            if let Some(ref ko) = classified.topic_ko {
                store.set_title_ko_if_absent(id, ko)?;
            }
            let alias = normalize_topic_title(&classified.topic);
            store.add_topic_alias(id, &alias)?;
            id
        }
        None => {
            if let Some(fuzzy_id) = store.find_topic_fuzzy(&classified.topic)? {
                if let Some(ref ko) = classified.topic_ko {
                    store.set_title_ko_if_absent(fuzzy_id, ko)?;
                }
                let alias = normalize_topic_title(&classified.topic);
                store.add_topic_alias(fuzzy_id, &alias)?;
                fuzzy_id
            } else {
                let new_topic = NewTopic {
                    title: classified.topic.clone(),
                    title_ko: classified.topic_ko.clone(),
                    category_id,
                };
                store.create_topic(&new_topic)?
            }
        }
    };

    let link = TopicMessageLink {
        topic_id,
        chat_id,
        message_id,
        relevance: classified.relevance,
        assigned_category: canonical_category,
    };
    store.link_message_to_topic(&link)?;
    store.record_topic_stat(topic_id, message_timestamp, chat_id)?;

    if let Some(new_cat_id) = store.check_category_reconciliation(topic_id)? {
        store.update_topic_category(topic_id, new_cat_id)?;
    }

    Ok(())
}
