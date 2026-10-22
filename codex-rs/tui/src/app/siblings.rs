//! Durable peer groups and sibling creation through the active app-server store.

use std::collections::HashSet;

use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadAttachment;
use codex_app_server_protocol::ThreadAttachmentAddParams;
use codex_app_server_protocol::ThreadAttachmentAddResponse;
use codex_app_server_protocol::ThreadAttachmentListParams;
use codex_app_server_protocol::ThreadAttachmentListResponse;
use codex_app_server_protocol::ThreadAttachmentOwnerListParams;
use codex_app_server_protocol::ThreadAttachmentOwnerListResponse;
use codex_app_server_protocol::ThreadListParams;
use codex_app_server_protocol::ThreadMetadataUpdateParams;
use codex_app_server_protocol::ThreadMetadataUpdateResponse;
use codex_protocol::ThreadId;
use codex_thread_store::sibling_context::SIBLING_CHILDREN_ATTACHMENT;
use codex_thread_store::sibling_context::SIBLING_ROOTS_ATTACHMENT;
use color_eyre::eyre::Result;
use color_eyre::eyre::bail;

use super::App;
use super::session_lifecycle::ThreadAttachPresentation;
use crate::app_event::ThreadTitleDestination;
use crate::app_server_session::AppServerSession;
use crate::app_server_session::ForkGoalContinuation;
use crate::chatwidget::UserMessage;
use crate::tui::Tui;

impl App {
    pub(super) async fn create_sibling(
        &mut self,
        tui: &mut Tui,
        server: &mut AppServerSession,
        initial_user_message: Option<UserMessage>,
    ) -> Result<()> {
        if !server.uses_embedded_app_server() {
            bail!(
                "Live sibling context requires this build's local backend. Restart with codex-latex --no-daemon, then resume this chat."
            );
        }
        let Some(source_id) = self.chat_widget.thread_id() else {
            bail!("Start a conversation before creating a sibling.");
        };
        let source = server.thread_read(source_id, false).await?;
        if source.ephemeral {
            bail!("Sibling context requires a saved conversation.");
        }
        if matches!(
            &source.source,
            codex_app_server_protocol::SessionSource::SubAgent(_)
        ) {
            bail!("Sibling context is available in saved user conversations.");
        }
        server
            .persist_sibling_metadata(
                source_id,
                source.project_id.clone(),
                source
                    .daybreak_enabled
                    .unwrap_or(self.chat_widget.config_ref().daybreak_enabled),
            )
            .await?;
        let parent = source
            .forked_from_id
            .as_deref()
            .map(ThreadId::from_string)
            .transpose()?;
        let (attachment_type, group) = match parent {
            Some(parent) => (SIBLING_CHILDREN_ATTACHMENT, parent.to_string()),
            None => (
                SIBLING_ROOTS_ATTACHMENT,
                server
                    .sibling_attachments(source_id)
                    .await?
                    .into_iter()
                    .find(|attachment| attachment.attachment_type == SIBLING_ROOTS_ATTACHMENT)
                    .map(|attachment| attachment.identity_key)
                    .unwrap_or_else(|| source_id.to_string()),
            ),
        };
        // A parent's marker is copied into future forks, so later branches join the same peer group.
        if let Some(parent) = parent {
            server
                .attach_sibling_group(parent, attachment_type, &group)
                .await?;
            for peer in server.saved_fork_children(parent).await? {
                server
                    .attach_sibling_group(peer, attachment_type, &group)
                    .await?;
            }
        }
        server
            .attach_sibling_group(source_id, attachment_type, &group)
            .await?;
        let mut config = self.chat_widget.config_ref().clone();
        config.ephemeral = false;
        config.model = Some(self.chat_widget.current_model().to_string());
        config.model_reasoning_effort = self.chat_widget.current_reasoning_effort();
        let selected_profile = self.selected_server_profile(source_id);
        let started = match parent {
            Some(parent) => {
                server
                    .fork_thread_at(
                        &self.local_settings,
                        config,
                        parent,
                        None,
                        None,
                        ForkGoalContinuation::DeferUntilNextTurn,
                        selected_profile.as_ref(),
                    )
                    .await?
            }
            None => {
                server
                    .start_thread_with_session_start_source(
                        &self.local_settings,
                        &config,
                        None,
                        None,
                        selected_profile.as_ref(),
                    )
                    .await?
            }
        };
        let sibling_id = started.session.thread_id;
        // A fresh root needs durable metadata before it can own an attachment, even without a prompt.
        server
            .persist_sibling_metadata(
                sibling_id,
                source.project_id.clone(),
                started.session.daybreak_enabled,
            )
            .await?;
        server
            .attach_sibling_group(sibling_id, attachment_type, &group)
            .await?;
        let title_prompt = initial_user_message
            .as_ref()
            .map(|message| super::thread_title::thread_title_prompt(&message.text));
        self.detach_current_thread_for_navigation(server, Some(sibling_id))
            .await;
        self.replace_chat_widget_with_app_server_thread(
            tui,
            started,
            ThreadAttachPresentation::SessionLineage,
            initial_user_message,
        )
        .await?;
        if let Some(prompt) = title_prompt {
            self.generate_thread_title(
                server,
                sibling_id,
                ThreadTitleDestination::Automatic,
                prompt,
            );
        }
        self.chat_widget.add_info_message(
            "Sibling created. Peer context refreshes before each new turn.".to_string(),
            None,
        );
        Ok(())
    }
}

impl AppServerSession {
    async fn persist_sibling_metadata(
        &self,
        thread_id: ThreadId,
        project_id: Option<String>,
        daybreak_enabled: bool,
    ) -> Result<()> {
        self.request_handle()
            .request_typed::<ThreadMetadataUpdateResponse>(ClientRequest::ThreadMetadataUpdate {
                request_id: request_id(),
                params: ThreadMetadataUpdateParams {
                    thread_id: thread_id.to_string(),
                    project_id,
                    daybreak_enabled: Some(daybreak_enabled),
                    git_info: None,
                },
            })
            .await?;
        Ok(())
    }

    pub(crate) async fn sibling_root_peers(&mut self, root: ThreadId) -> Result<Vec<ThreadId>> {
        let Some(group) = self
            .sibling_attachments(root)
            .await?
            .into_iter()
            .find(|attachment| attachment.attachment_type == SIBLING_ROOTS_ATTACHMENT)
        else {
            return Ok(Vec::new());
        };
        let mut peers = Vec::new();
        let mut cursor = None;
        let mut cursors = HashSet::new();
        loop {
            let page = self
                .request_handle()
                .request_typed::<ThreadAttachmentOwnerListResponse>(
                    ClientRequest::ThreadAttachmentOwnerList {
                        request_id: request_id(),
                        params: ThreadAttachmentOwnerListParams {
                            attachment_type: SIBLING_ROOTS_ATTACHMENT.to_string(),
                            identity_key: group.identity_key.clone(),
                            archived: Some(false),
                            cursor,
                            limit: Some(100),
                        },
                    },
                )
                .await?;
            for peer in page.data {
                let id = ThreadId::from_string(&peer.thread_id)?;
                if self.thread_read(id, false).await?.forked_from_id.is_none() {
                    peers.push(id);
                }
            }
            cursor = page.next_cursor;
            if let Some(cursor) = &cursor
                && !cursors.insert(cursor.clone())
            {
                bail!("Sibling root listing repeated a cursor");
            }
            if cursor.is_none() {
                return Ok(peers);
            }
        }
    }

    pub(crate) async fn sibling_attachments(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<ThreadAttachment>> {
        let mut result = Vec::new();
        let mut cursor = None;
        let mut cursors = HashSet::new();
        loop {
            let page = self
                .request_handle()
                .request_typed::<ThreadAttachmentListResponse>(
                    ClientRequest::ThreadAttachmentList {
                        request_id: request_id(),
                        params: ThreadAttachmentListParams {
                            thread_id: thread_id.to_string(),
                            cursor,
                            limit: Some(100),
                        },
                    },
                )
                .await?;
            result.extend(page.data);
            cursor = page.next_cursor;
            if let Some(cursor) = &cursor
                && !cursors.insert(cursor.clone())
            {
                bail!("Sibling attachment listing repeated a cursor");
            }
            if cursor.is_none() {
                return Ok(result);
            }
        }
    }

    pub(crate) async fn attach_sibling_group(
        &self,
        thread_id: ThreadId,
        attachment_type: &str,
        group: &str,
    ) -> Result<()> {
        self.request_handle()
            .request_typed::<ThreadAttachmentAddResponse>(ClientRequest::ThreadAttachmentAdd {
                request_id: request_id(),
                params: ThreadAttachmentAddParams {
                    thread_id: thread_id.to_string(),
                    attachment_type: attachment_type.to_string(),
                    identity_key: group.to_string(),
                    payload: serde_json::json!({"version": 1}),
                },
            })
            .await?;
        Ok(())
    }

    async fn saved_fork_children(&mut self, parent: ThreadId) -> Result<Vec<ThreadId>> {
        let mut result = Vec::new();
        let mut cursor = None;
        let mut cursors = HashSet::new();
        loop {
            let page = self
                .thread_list(ThreadListParams {
                    cursor,
                    limit: Some(100),
                    source_kinds: Some(crate::resume_source_kinds(false)),
                    archived: Some(false),
                    sort_key: None,
                    sort_direction: None,
                    model_providers: Some(Vec::new()),
                    originators: None,
                    section_id: None,
                    project_id: None,
                    cwd: None,
                    use_state_db_only: false,
                    search_term: None,
                    parent_thread_id: None,
                    ancestor_thread_id: None,
                    excluded_thread_ids: None,
                })
                .await?;
            for thread in page.data {
                let id = ThreadId::from_string(&thread.id)?;
                let actual_parent = match thread.forked_from_id {
                    Some(parent) => Some(parent),
                    None => self.thread_read(id, false).await?.forked_from_id,
                };
                if actual_parent.as_deref() == Some(parent.to_string().as_str()) {
                    result.push(id);
                }
            }
            cursor = page.next_cursor;
            if let Some(cursor) = &cursor
                && !cursors.insert(cursor.clone())
            {
                bail!("Sibling thread listing repeated a cursor");
            }
            if cursor.is_none() {
                return Ok(result);
            }
        }
    }
}

fn request_id() -> RequestId {
    RequestId::String(uuid::Uuid::new_v4().to_string())
}
