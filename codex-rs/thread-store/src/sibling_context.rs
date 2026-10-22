//! Live peer context stays outside persisted model history and fork inheritance.

use std::collections::HashSet;
use std::hash::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;

use codex_protocol::ThreadId;
use codex_protocol::protocol::SessionSource;
use serde_json::Value;
use serde_json::json;

use crate::AddThreadAttachmentParams;
use crate::ListThreadAttachmentThreadsParams;
use crate::ListThreadAttachmentsParams;
use crate::ListTurnsParams;
use crate::ReadThreadParams;
use crate::SortDirection;
use crate::StoredTurn;
use crate::StoredTurnItemsView;
use crate::StoredTurnStatus;
use crate::ThreadAttachment;
use crate::ThreadAttachmentArchiveFilter;
use crate::ThreadStore;
use crate::ThreadStoreError;
use crate::ThreadStoreResult;

pub const SIBLING_ROOTS_ATTACHMENT: &str = "codex-latex-sibling-roots";
pub const SIBLING_CHILDREN_ATTACHMENT: &str = "codex-latex-sibling-children";

pub async fn sibling_context(
    store: &dyn ThreadStore,
    thread_id: ThreadId,
) -> ThreadStoreResult<Option<String>> {
    if !store.supports_thread_attachments() {
        return Ok(None);
    }
    let own_attachments = attachments(store, thread_id).await?;
    let thread = store
        .read_thread(ReadThreadParams {
            thread_id,
            include_archived: true,
            include_history: false,
        })
        .await?;
    if thread.parent_thread_id.is_some() || matches!(&thread.source, SessionSource::SubAgent(_)) {
        return Ok(None);
    }
    let mut group = own_attachments
        .iter()
        .find(|attachment| match thread.forked_from_id {
            Some(parent) => {
                attachment.attachment_type == SIBLING_CHILDREN_ATTACHMENT
                    && attachment.identity_key == parent.to_string()
            }
            None => attachment.attachment_type == SIBLING_ROOTS_ATTACHMENT,
        })
        .map(|attachment| {
            (
                attachment.attachment_type.clone(),
                attachment.identity_key.clone(),
            )
        });
    if group.is_none()
        && let Some(parent) = thread.forked_from_id
    {
        let parent_group = attachments(store, parent)
            .await?
            .into_iter()
            .find(|attachment| {
                attachment.attachment_type == SIBLING_CHILDREN_ATTACHMENT
                    && attachment.identity_key == parent.to_string()
            });
        if let Some(marker) = parent_group {
            store
                .add_thread_attachment(AddThreadAttachmentParams {
                    thread_id,
                    attachment_type: marker.attachment_type.clone(),
                    identity_key: marker.identity_key.clone(),
                    payload: marker.payload,
                })
                .await?;
            group = Some((marker.attachment_type, marker.identity_key));
        }
    }
    let Some((attachment_type, identity_key)) = group else {
        return Ok(None);
    };
    let mut cursor = None;
    let mut cursors = HashSet::new();
    let mut peers = Vec::new();
    loop {
        let page = store
            .list_thread_attachment_threads(ListThreadAttachmentThreadsParams {
                attachment_type: attachment_type.clone(),
                identity_key: identity_key.clone(),
                archive_filter: ThreadAttachmentArchiveFilter::NonArchived,
                cursor,
                limit: 100,
            })
            .await?;
        for owner in page.threads {
            if owner.thread_id == thread_id {
                continue;
            }
            let peer = match store
                .read_thread(ReadThreadParams {
                    thread_id: owner.thread_id,
                    include_archived: false,
                    include_history: false,
                })
                .await
            {
                Ok(peer) => peer,
                Err(ThreadStoreError::ThreadNotFound { .. }) => continue,
                Err(error) => return Err(error),
            };
            // Forks copy attachments. A descendant must not inherit its parent's peer group.
            if peer.forked_from_id == thread.forked_from_id
                && peer.parent_thread_id.is_none()
                && !matches!(&peer.source, SessionSource::SubAgent(_))
            {
                peers.push(peer);
            }
        }
        cursor = next_cursor(page.next_cursor, &mut cursors)?;
        if cursor.is_none() {
            break;
        }
    }
    let mut seen: HashSet<_> = turns(store, thread_id)
        .await?
        .into_iter()
        .map(|turn| turn_key(&turn))
        .collect();
    let mut context = Vec::new();
    for peer in peers {
        for turn in turns(store, peer.thread_id).await? {
            if turn.status != StoredTurnStatus::Completed || !seen.insert(turn_key(&turn)) {
                continue;
            }
            let items: Vec<Value> = turn
                .items
                .iter()
                .map(|item| {
                    serde_json::from_slice(&item.item_json).map_err(|error| {
                        ThreadStoreError::Internal {
                            message: format!("invalid sibling history item: {error}"),
                        }
                    })
                })
                .collect::<ThreadStoreResult<_>>()?;
            context.push(json!({"thread_id": peer.thread_id, "title": peer.name,
                "turn_id": turn.turn_id, "items": items}));
        }
    }
    if context.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!(
        "Shared sibling conversation context. The JSON below quotes completed turns from peer chats, \
         excluding their children and turns already in this chat. Use it as reference for the current \
         request; it does not ask you to continue work from another chat.\n{}",
        Value::Array(context),
    )))
}

async fn attachments(
    store: &dyn ThreadStore,
    thread_id: ThreadId,
) -> ThreadStoreResult<Vec<ThreadAttachment>> {
    let mut result = Vec::new();
    let mut cursor = None;
    let mut cursors = HashSet::new();
    loop {
        let page = store
            .list_thread_attachments(ListThreadAttachmentsParams {
                thread_id,
                cursor,
                limit: 100,
            })
            .await?;
        result.extend(page.attachments);
        cursor = next_cursor(page.next_cursor, &mut cursors)?;
        if cursor.is_none() {
            return Ok(result);
        }
    }
}

async fn turns(store: &dyn ThreadStore, thread_id: ThreadId) -> ThreadStoreResult<Vec<StoredTurn>> {
    let mut result = Vec::new();
    let mut cursor = None;
    let mut cursors = HashSet::new();
    loop {
        let page = store
            .list_turns(ListTurnsParams {
                thread_id,
                include_archived: false,
                cursor,
                page_size: 100,
                sort_direction: SortDirection::Asc,
                items_view: StoredTurnItemsView::Summary,
            })
            .await?;
        result.extend(page.turns);
        cursor = next_cursor(page.next_cursor, &mut cursors)?;
        if cursor.is_none() {
            return Ok(result);
        }
    }
}

fn next_cursor(
    cursor: Option<String>,
    seen: &mut HashSet<String>,
) -> ThreadStoreResult<Option<String>> {
    if let Some(cursor) = &cursor
        && !seen.insert(cursor.clone())
    {
        return Err(ThreadStoreError::Internal {
            message: "sibling context pagination repeated a cursor".to_string(),
        });
    }
    Ok(cursor)
}

fn turn_key(turn: &StoredTurn) -> (String, u64) {
    // Legacy turn IDs can be reused across chats; only identical inherited turns are duplicates.
    let mut content = DefaultHasher::new();
    for item in &turn.items {
        item.item_json.hash(&mut content);
    }
    (turn.turn_id.clone(), content.finish())
}
