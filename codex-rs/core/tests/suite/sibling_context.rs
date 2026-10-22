use std::sync::Arc;

use anyhow::Result;
use codex_core::ForkSnapshot;
use codex_core::NewThread;
use codex_core::TurnInputRequest;
use codex_history::InitialHistory;
use codex_history::ResumedHistory;
use codex_protocol::ThreadId;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ThreadHistoryMode;
use codex_protocol::user_input::UserInput;
use codex_thread_store::AddThreadAttachmentParams;
use codex_thread_store::LoadThreadHistoryParams;
use codex_thread_store::sibling_context::SIBLING_CHILDREN_ATTACHMENT;
use codex_thread_store::sibling_context::SIBLING_ROOTS_ATTACHMENT;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_sibling_context_excludes_children_and_is_not_inherited() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = start_mock_server().await;
    let responses = mount_sse_sequence(
        &server,
        (0..7)
            .map(|index| {
                let id = format!("response-{index}");
                sse(vec![
                    ev_response_created(&id),
                    ev_assistant_message(&id, "Done"),
                    ev_completed(&id),
                ])
            })
            .collect(),
    )
    .await;
    let first = test_codex()
        .with_history_mode(ThreadHistoryMode::Paginated)
        .build_with_auto_env(&server)
        .await?;
    first.submit_text_turn("ALPHA_ROOT_ONLY").await?;
    let second = test_codex()
        .with_history_mode(ThreadHistoryMode::Paginated)
        .with_home(Arc::clone(&first.home))
        .with_thread_store(Arc::clone(&first.thread_store))
        .build_with_auto_env(&server)
        .await?;
    second.submit_text_turn("BETA_OWN_HISTORY").await?;
    assert!(first.thread_store.supports_thread_attachments());
    let group = first.session_configured.thread_id.to_string();
    for thread_id in [
        first.session_configured.thread_id,
        second.session_configured.thread_id,
    ] {
        first
            .thread_store
            .add_thread_attachment(AddThreadAttachmentParams {
                thread_id,
                attachment_type: SIBLING_ROOTS_ATTACHMENT.to_string(),
                identity_key: group.clone(),
                payload: serde_json::json!({"version": 1}),
            })
            .await?;
    }
    second.submit_text_turn("READ_PEER_CONTEXT").await?;
    assert!(
        responses.requests()[2]
            .body_json()
            .to_string()
            .contains("ALPHA_ROOT_ONLY")
    );
    first
        .submit_text_turn("ALPHA_LATEST_COMPLETED_TURN")
        .await?;
    let history = second
        .thread_store
        .load_latest_model_context(LoadThreadHistoryParams {
            thread_id: second.session_configured.thread_id,
            include_archived: false,
        })
        .await?;
    let persisted = serde_json::to_string(&history.items)?;
    assert!(
        !persisted.contains("ALPHA_ROOT_ONLY"),
        "peer context was persisted into own history"
    );
    let child = second
        .thread_manager
        .fork_thread_from_history(
            ForkSnapshot::Interrupted,
            second.start_thread_options().await,
            InitialHistory::Resumed(ResumedHistory {
                history_revision: history.revision,
                conversation_id: history.thread_id,
                history: Arc::new(history.items),
                rollout_path: None,
            }),
        )
        .await?;
    child
        .thread
        .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
            text: "CHILD_PRIVATE_CONTEXT".to_string(),
            text_elements: Vec::new(),
        }]))
        .await?;
    wait_for_event(&child.thread, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let child_request = responses.requests()[4].body_json().to_string();
    assert!(child_request.contains("BETA_OWN_HISTORY"));
    assert!(!child_request.contains("ALPHA_ROOT_ONLY"));
    first.submit_text_turn("CHECK_CHILD_EXCLUSION").await?;
    let first_request = responses.requests()[5].body_json().to_string();
    assert!(first_request.contains("BETA_OWN_HISTORY"));
    assert!(!first_request.contains("CHILD_PRIVATE_CONTEXT"));
    second.submit_text_turn("CHECK_LATEST_PEER_TURN").await?;
    let latest = responses.requests()[6].body_json().to_string();
    assert!(latest.contains("ALPHA_LATEST_COMPLETED_TURN"));
    assert!(!latest.contains("CHILD_PRIVATE_CONTEXT"));
    child.thread.shutdown_and_wait().await?;
    second.codex.shutdown_and_wait().await?;
    first.codex.shutdown_and_wait().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nonroot_siblings_share_completed_turns_without_parent_or_grandchild_leaks() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = start_mock_server().await;
    let responses = mount_sse_sequence(
        &server,
        (0..7)
            .map(|index| {
                let id = format!("nested-{index}");
                sse(vec![
                    ev_response_created(&id),
                    ev_assistant_message(&id, "Done"),
                    ev_completed(&id),
                ])
            })
            .collect(),
    )
    .await;
    let fixture = test_codex()
        .with_history_mode(ThreadHistoryMode::Paginated)
        .build_with_auto_env(&server)
        .await?;
    fixture.submit_text_turn("COMMON_ANCESTOR_CONTEXT").await?;
    let parent = fixture.session_configured.thread_id;
    let left = fork(&fixture, parent).await?;
    submit(&left, "LEFT_SIBLING_CONTEXT").await?;
    let right = fork(&fixture, parent).await?;
    submit(&right, "RIGHT_SIBLING_CONTEXT").await?;
    // Existing children can join on their next turn without restarting or inheriting a fresh marker.
    for thread_id in [parent, left.thread_id] {
        fixture
            .thread_store
            .add_thread_attachment(AddThreadAttachmentParams {
                thread_id,
                attachment_type: SIBLING_CHILDREN_ATTACHMENT.to_string(),
                identity_key: parent.to_string(),
                payload: serde_json::json!({"version": 1}),
            })
            .await?;
    }
    submit(&right, "READ_LEFT_CONTEXT").await?;
    let peer_request = responses.requests()[3].body_json().to_string();
    assert!(peer_request.contains("LEFT_SIBLING_CONTEXT"));
    fixture.submit_text_turn("PARENT_STAYS_SEPARATE").await?;
    let parent_request = responses.requests()[4].body_json().to_string();
    assert!(!parent_request.contains("LEFT_SIBLING_CONTEXT"));
    assert!(!parent_request.contains("RIGHT_SIBLING_CONTEXT"));
    let grandchild = fork(&fixture, left.thread_id).await?;
    submit(&grandchild, "GRANDCHILD_PRIVATE_CONTEXT").await?;
    let grandchild_request = responses.requests()[5].body_json().to_string();
    assert!(grandchild_request.contains("LEFT_SIBLING_CONTEXT"));
    assert!(!grandchild_request.contains("RIGHT_SIBLING_CONTEXT"));
    submit(&left, "READ_RIGHT_CONTEXT").await?;
    let latest = responses.requests()[6].body_json().to_string();
    assert!(latest.contains("RIGHT_SIBLING_CONTEXT"));
    assert!(!latest.contains("GRANDCHILD_PRIVATE_CONTEXT"));
    grandchild.thread.shutdown_and_wait().await?;
    left.thread.shutdown_and_wait().await?;
    right.thread.shutdown_and_wait().await?;
    fixture.codex.shutdown_and_wait().await?;
    Ok(())
}

async fn fork(fixture: &TestCodex, thread_id: ThreadId) -> Result<NewThread> {
    let history = fixture
        .thread_store
        .load_latest_model_context(LoadThreadHistoryParams {
            thread_id,
            include_archived: false,
        })
        .await?;
    Ok(fixture
        .thread_manager
        .fork_thread_from_history(
            ForkSnapshot::Interrupted,
            fixture.start_thread_options().await,
            InitialHistory::Resumed(ResumedHistory {
                history_revision: history.revision,
                conversation_id: history.thread_id,
                history: Arc::new(history.items),
                rollout_path: None,
            }),
        )
        .await?)
}

async fn submit(thread: &NewThread, text: &str) -> Result<()> {
    thread
        .thread
        .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
            text: text.to_string(),
            text_elements: Vec::new(),
        }]))
        .await?;
    wait_for_event(&thread.thread, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    Ok(())
}
