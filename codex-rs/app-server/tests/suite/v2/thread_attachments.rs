//! Public RPC coverage for durable thread attachment membership and notifications.

use super::connection_handling_websocket::DEFAULT_READ_TIMEOUT;
use super::connection_handling_websocket::connect_websocket;
use super::connection_handling_websocket::read_jsonrpc_message;
use super::connection_handling_websocket::read_notification_for_method;
use super::connection_handling_websocket::read_response_and_notification_for_method;
use super::connection_handling_websocket::read_response_for_id;
use super::connection_handling_websocket::send_initialize_request;
use super::connection_handling_websocket::send_request;
use super::connection_handling_websocket::spawn_websocket_server;
use anyhow::Context;
use anyhow::Result;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use app_test_support::create_fake_rollout;
use app_test_support::create_mock_responses_server_repeating_assistant;
use app_test_support::to_response;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::JSONRPCError;
use codex_app_server_protocol::JSONRPCMessage;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadArchiveParams;
use codex_app_server_protocol::ThreadArchiveResponse;
use codex_app_server_protocol::ThreadAttachment;
use codex_app_server_protocol::ThreadAttachmentAddOutcome;
use codex_app_server_protocol::ThreadAttachmentAddParams;
use codex_app_server_protocol::ThreadAttachmentAddResponse;
use codex_app_server_protocol::ThreadAttachmentListParams;
use codex_app_server_protocol::ThreadAttachmentListResponse;
use codex_app_server_protocol::ThreadAttachmentOperation;
use codex_app_server_protocol::ThreadAttachmentOwner;
use codex_app_server_protocol::ThreadAttachmentOwnerListParams;
use codex_app_server_protocol::ThreadAttachmentOwnerListResponse;
use codex_app_server_protocol::ThreadAttachmentRemoveParams;
use codex_app_server_protocol::ThreadAttachmentRemoveResponse;
use codex_app_server_protocol::ThreadAttachmentUpdatedNotification;
use codex_app_server_protocol::ThreadListResponse;
use codex_app_server_protocol::ThreadLoadedListParams;
use codex_app_server_protocol::ThreadLoadedListResponse;
use codex_features::Feature;
use codex_thread_store::sibling_context::SIBLING_ROOTS_ATTACHMENT;
use pretty_assertions::assert_eq;
use serde_json::json;
use tempfile::TempDir;
use tokio::time::timeout;

#[tokio::test]
async fn shared_backend_loads_live_sibling_context_across_clients() -> Result<()> {
    let responses = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    MockResponsesConfig::new(&responses.uri())
        .enable_feature(Feature::Sqlite)
        .write(codex_home.path())?;
    let (mut process, address) = spawn_websocket_server(codex_home.path()).await?;
    let mut first = connect_websocket(address).await?;
    let mut second = connect_websocket(address).await?;
    let mut threads = Vec::new();
    for (client, marker) in [
        (&mut first, "ALPHA_PEER_ONLY"),
        (&mut second, "BETA_OWN_HISTORY"),
    ] {
        send_request(
            client,
            "initialize",
            1,
            Some(json!({
                "clientInfo": {"name": "sibling_client", "version": "1.0.0"},
                "capabilities": {"experimentalApi": true}
            })),
        )
        .await?;
        let initialized = read_response_for_id(client, 1).await?;
        assert_eq!(initialized.result["supportsSiblingContext"], true);
        send_request(
            client,
            "thread/start",
            2,
            Some(json!({"historyMode": "paginated"})),
        )
        .await?;
        let started = read_response_for_id(client, 2).await?;
        let thread = started.result["thread"]["id"]
            .as_str()
            .context("thread id")?
            .to_owned();
        send_sibling_test_turn(client, &thread, 3, marker).await?;
        threads.push(thread);
    }
    for (client, thread) in [(&mut first, &threads[0]), (&mut second, &threads[1])] {
        send_request(
            client,
            "thread/attachment/add",
            4,
            Some(json!({
                "threadId": thread,
                "attachmentType": SIBLING_ROOTS_ATTACHMENT,
                "identityKey": threads[0],
                "payload": {"version": 1}
            })),
        )
        .await?;
        read_response_for_id(client, 4).await?;
    }
    first.close(None).await?;
    send_sibling_test_turn(&mut second, &threads[1], 5, "READ_LIVE_PEER_CONTEXT").await?;
    let requests = responses
        .received_requests()
        .await
        .context("recorded model requests")?;
    let latest = requests
        .iter()
        .rev()
        .find(|request| request.url.path().ends_with("/responses"))
        .context("model request")?;
    let body: serde_json::Value = serde_json::from_slice(&latest.body)?;
    let input = body["input"].to_string();
    assert!(input.contains("ALPHA_PEER_ONLY"), "{input}");
    assert!(input.contains("BETA_OWN_HISTORY"), "{input}");
    send_request(
        &mut second,
        "thread/read",
        6,
        Some(json!({"threadId": threads[1], "includeTurns": true})),
    )
    .await?;
    let saved = read_response_for_id(&mut second, 6).await?;
    assert!(
        !saved.result["thread"]["turns"]
            .to_string()
            .contains("ALPHA_PEER_ONLY")
    );
    second.close(None).await?;
    process.kill().await?;
    Ok(())
}

#[tokio::test]
async fn thread_attachments_support_unloaded_listing_and_idempotent_attachment() -> Result<()> {
    let responses = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    MockResponsesConfig::new(&responses.uri())
        .enable_feature(Feature::Sqlite)
        .write(codex_home.path())?;
    let first_thread = create_fake_rollout(
        codex_home.path(),
        "2025-01-06T08-00-00",
        "2025-01-06T08:00:00Z",
        "First thread",
        Some("mock_provider"),
        /*git_info*/ None,
    )?;
    let unrelated_thread = create_fake_rollout(
        codex_home.path(),
        "2025-01-06T10-00-00",
        "2025-01-06T10:00:00Z",
        "Unrelated thread",
        Some("mock_provider"),
        /*git_info*/ None,
    )?;
    let mut server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .build_initialized()
        .await?;
    let request_id = server
        .send_raw_request("thread/list", Some(json!({ "limit": 20 })))
        .await?;
    let _: ThreadListResponse =
        timeout(DEFAULT_READ_TIMEOUT, server.read_response(request_id)).await??;

    let loaded: ThreadLoadedListResponse = server
        .request(|request_id| ClientRequest::ThreadLoadedList {
            request_id,
            params: ThreadLoadedListParams::default(),
        })
        .await?;
    assert_eq!(loaded.data, Vec::<String>::new());

    let first_params = ThreadAttachmentAddParams {
        thread_id: first_thread.clone(),
        attachment_type: "pull_request".to_string(),
        identity_key: r#"["github.com","openai","codex",123]"#.to_string(),
        payload: json!({ "url": "https://github.com/openai/codex/pull/123" }),
    };
    let first: ThreadAttachmentAddResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentAdd {
            request_id,
            params: first_params.clone(),
        })
        .await?;
    assert_eq!(first.outcome, ThreadAttachmentAddOutcome::Created);
    let first_attachment = first.attachment;
    let created_notification: ThreadAttachmentUpdatedNotification = timeout(
        DEFAULT_READ_TIMEOUT,
        server.read_notification("thread/attachment/updated"),
    )
    .await??;
    assert_eq!(
        created_notification,
        ThreadAttachmentUpdatedNotification {
            thread_id: first_thread.clone(),
            attachment_type: first_attachment.attachment_type.clone(),
            identity_key: first_attachment.identity_key.clone(),
            attachment_id: first_attachment.id.clone(),
            operation: ThreadAttachmentOperation::Created,
        }
    );

    let duplicate: ThreadAttachmentAddResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentAdd {
            request_id,
            params: first_params,
        })
        .await?;
    assert_eq!(
        duplicate,
        ThreadAttachmentAddResponse {
            outcome: ThreadAttachmentAddOutcome::Existing,
            attachment: first_attachment.clone(),
        }
    );

    let second: ThreadAttachmentAddResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentAdd {
            request_id,
            params: ThreadAttachmentAddParams {
                thread_id: first_thread.clone(),
                attachment_type: "pull_request".to_string(),
                identity_key: r#"["github.com","openai","codex",456]"#.to_string(),
                payload: json!({ "url": "https://github.com/openai/codex/pull/456" }),
            },
        })
        .await?;
    let second_attachment = second.attachment;

    let unrelated: ThreadAttachmentAddResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentAdd {
            request_id,
            params: ThreadAttachmentAddParams {
                thread_id: unrelated_thread.clone(),
                attachment_type: "pull_request".to_string(),
                identity_key: r#"["github.com","openai","codex",789]"#.to_string(),
                payload: json!({ "url": "https://github.com/openai/codex/pull/789" }),
            },
        })
        .await?;
    assert_eq!(unrelated.outcome, ThreadAttachmentAddOutcome::Created);

    // Neither the same type with another key nor the same key with another type matches.
    let _: ThreadAttachmentAddResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentAdd {
            request_id,
            params: ThreadAttachmentAddParams {
                thread_id: unrelated_thread.clone(),
                attachment_type: "document".to_string(),
                identity_key: first_attachment.identity_key.clone(),
                payload: json!({}),
            },
        })
        .await?;
    let reverse_params = ThreadAttachmentOwnerListParams {
        attachment_type: first_attachment.attachment_type.clone(),
        identity_key: first_attachment.identity_key.clone(),
        archived: None,
        cursor: None,
        limit: None,
    };
    let matched: ThreadAttachmentOwnerListResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentOwnerList {
            request_id,
            params: reverse_params.clone(),
        })
        .await?;
    let active_match = ThreadAttachmentOwner {
        thread_id: first_thread.clone(),
        archived: false,
    };
    assert_eq!(
        matched,
        ThreadAttachmentOwnerListResponse {
            data: vec![active_match.clone()],
            next_cursor: None,
        }
    );
    let _: ThreadAttachmentAddResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentAdd {
            request_id,
            params: ThreadAttachmentAddParams {
                thread_id: unrelated_thread.clone(),
                attachment_type: first_attachment.attachment_type.clone(),
                identity_key: first_attachment.identity_key.clone(),
                payload: first_attachment.payload.clone(),
            },
        })
        .await?;
    let _: ThreadArchiveResponse = server
        .request(|request_id| ClientRequest::ThreadArchive {
            request_id,
            params: ThreadArchiveParams {
                thread_id: unrelated_thread.clone(),
            },
        })
        .await?;
    let all_matches: ThreadAttachmentOwnerListResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentOwnerList {
            request_id,
            params: reverse_params.clone(),
        })
        .await?;
    let archived_match = ThreadAttachmentOwner {
        thread_id: unrelated_thread,
        archived: true,
    };
    let mut expected_matches = vec![active_match, archived_match.clone()];
    expected_matches.sort_by(|left, right| left.thread_id.cmp(&right.thread_id));
    assert_eq!(
        all_matches,
        ThreadAttachmentOwnerListResponse {
            data: expected_matches,
            next_cursor: None,
        }
    );
    let archived_matches: ThreadAttachmentOwnerListResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentOwnerList {
            request_id,
            params: ThreadAttachmentOwnerListParams {
                archived: Some(true),
                ..reverse_params
            },
        })
        .await?;
    assert_eq!(
        archived_matches,
        ThreadAttachmentOwnerListResponse {
            data: vec![archived_match],
            next_cursor: None,
        }
    );

    let first_page: ThreadAttachmentListResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentList {
            request_id,
            params: ThreadAttachmentListParams {
                thread_id: first_thread.clone(),
                cursor: None,
                limit: Some(1),
            },
        })
        .await?;
    assert_eq!(first_page.data.len(), 1);
    let second_page: ThreadAttachmentListResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentList {
            request_id,
            params: ThreadAttachmentListParams {
                thread_id: first_thread.clone(),
                cursor: first_page.next_cursor.clone(),
                limit: Some(1),
            },
        })
        .await?;
    assert_eq!(second_page.next_cursor, None);

    let mut actual = first_page.data;
    actual.extend(second_page.data);
    let expected = vec![first_attachment, second_attachment];
    assert_eq!(actual, expected);

    let empty: ThreadAttachmentListResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentList {
            request_id,
            params: ThreadAttachmentListParams {
                thread_id: uuid::Uuid::now_v7().to_string(),
                cursor: None,
                limit: Some(10),
            },
        })
        .await?;
    assert_eq!(empty.data, Vec::<ThreadAttachment>::new());
    Ok(())
}

#[tokio::test]
async fn attachment_removal_is_idempotent_and_allows_reattachment() -> Result<()> {
    let responses = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    MockResponsesConfig::new(&responses.uri())
        .enable_feature(Feature::Sqlite)
        .write(codex_home.path())?;
    let thread_id = create_fake_rollout(
        codex_home.path(),
        "2025-01-06T08-00-00",
        "2025-01-06T08:00:00Z",
        "Stored thread",
        Some("mock_provider"),
        /*git_info*/ None,
    )?;
    let mut server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .build_initialized()
        .await?;
    let request_id = server
        .send_raw_request("thread/list", Some(json!({ "limit": 20 })))
        .await?;
    let _: ThreadListResponse =
        timeout(DEFAULT_READ_TIMEOUT, server.read_response(request_id)).await??;

    let create_params = ThreadAttachmentAddParams {
        thread_id: thread_id.clone(),
        attachment_type: "pull_request".to_string(),
        identity_key: r#"["github.com","openai","codex",123]"#.to_string(),
        payload: json!({ "url": "https://github.com/openai/codex/pull/123" }),
    };
    let created: ThreadAttachmentAddResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentAdd {
            request_id,
            params: create_params.clone(),
        })
        .await?;
    assert_eq!(created.outcome, ThreadAttachmentAddOutcome::Created);
    let created_notification: ThreadAttachmentUpdatedNotification = timeout(
        DEFAULT_READ_TIMEOUT,
        server.read_notification("thread/attachment/updated"),
    )
    .await??;
    assert_eq!(created_notification.attachment_id, created.attachment.id);

    let delete_params = ThreadAttachmentRemoveParams {
        thread_id: thread_id.to_ascii_uppercase(),
        attachment_type: create_params.attachment_type.clone(),
        identity_key: create_params.identity_key.clone(),
    };
    let _: ThreadAttachmentRemoveResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentRemove {
            request_id,
            params: delete_params.clone(),
        })
        .await?;
    let removed_notification: ThreadAttachmentUpdatedNotification = timeout(
        DEFAULT_READ_TIMEOUT,
        server.read_notification("thread/attachment/updated"),
    )
    .await??;
    assert_eq!(
        removed_notification,
        ThreadAttachmentUpdatedNotification {
            thread_id: thread_id.clone(),
            attachment_type: delete_params.attachment_type.clone(),
            identity_key: delete_params.identity_key.clone(),
            attachment_id: created.attachment.id,
            operation: ThreadAttachmentOperation::Deleted,
        }
    );

    let _: ThreadAttachmentRemoveResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentRemove {
            request_id,
            params: delete_params,
        })
        .await?;

    let listed: ThreadAttachmentListResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentList {
            request_id,
            params: ThreadAttachmentListParams {
                thread_id,
                cursor: None,
                limit: Some(10),
            },
        })
        .await?;
    assert_eq!(listed.data, Vec::<ThreadAttachment>::new());

    let restored: ThreadAttachmentAddResponse = server
        .request(|request_id| ClientRequest::ThreadAttachmentAdd {
            request_id,
            params: create_params,
        })
        .await?;
    assert_eq!(restored.outcome, ThreadAttachmentAddOutcome::Created);
    let restored_notification: ThreadAttachmentUpdatedNotification = timeout(
        DEFAULT_READ_TIMEOUT,
        server.read_notification("thread/attachment/updated"),
    )
    .await??;
    assert_eq!(
        restored_notification.operation,
        ThreadAttachmentOperation::Created
    );
    assert_eq!(restored_notification.attachment_id, restored.attachment.id);
    Ok(())
}

#[tokio::test]
async fn thread_attachment_requests_reject_invalid_identities_and_cursors() -> Result<()> {
    let responses = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    MockResponsesConfig::new(&responses.uri())
        .enable_feature(Feature::Sqlite)
        .write(codex_home.path())?;
    let mut server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .build_initialized()
        .await?;

    for (method, params) in [
        (
            "thread/attachment/add",
            json!({
                "threadId": "not-a-thread-id",
                "attachmentType": "pull_request",
                "identityKey": r#"["github.com","openai","codex",123]"#,
                "payload": {}
            }),
        ),
        (
            "thread/attachment/add",
            json!({
                "threadId": uuid::Uuid::now_v7().to_string(),
                "attachmentType": " ",
                "identityKey": r#"["github.com","openai","codex",123]"#,
                "payload": {}
            }),
        ),
        (
            "thread/attachment/remove",
            json!({
                "threadId": uuid::Uuid::now_v7().to_string(),
                "attachmentType": "pull_request",
                "identityKey": " "
            }),
        ),
        (
            "thread/attachmentOwner/list",
            json!({ "attachmentType": "pull_request", "identityKey": "key", "cursor": "invalid-cursor" }),
        ),
        ("thread/attachment/list", json!({ "threadId": "invalid" })),
        (
            "thread/attachment/list",
            json!({
                "threadId": uuid::Uuid::now_v7().to_string(),
                "cursor": "invalid-cursor"
            }),
        ),
    ] {
        let request_id = server.send_raw_request(method, Some(params)).await?;
        let error: JSONRPCError = timeout(
            DEFAULT_READ_TIMEOUT,
            server.read_stream_until_error_message(RequestId::Integer(request_id)),
        )
        .await??;
        assert_eq!(error.error.code, -32602);
    }
    Ok(())
}

#[tokio::test]
async fn thread_attachment_mutations_respond_before_broadcasting_updates_to_multiple_clients()
-> Result<()> {
    let responses = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    MockResponsesConfig::new(&responses.uri())
        .enable_feature(Feature::Sqlite)
        .write(codex_home.path())?;
    let thread_id = create_fake_rollout(
        codex_home.path(),
        "2025-01-06T08-00-00",
        "2025-01-06T08:00:00Z",
        "Stored thread",
        Some("mock_provider"),
        /*git_info*/ None,
    )?;
    let (mut process, bind_addr) = spawn_websocket_server(codex_home.path()).await?;

    let result = async {
        let mut first_client = connect_websocket(bind_addr).await?;
        let mut second_client = connect_websocket(bind_addr).await?;
        send_initialize_request(&mut first_client, /*id*/ 1, "attachment-client-one").await?;
        timeout(
            DEFAULT_READ_TIMEOUT,
            read_response_for_id(&mut first_client, /*id*/ 1),
        )
        .await??;
        send_initialize_request(&mut second_client, /*id*/ 2, "attachment-client-two").await?;
        timeout(
            DEFAULT_READ_TIMEOUT,
            read_response_for_id(&mut second_client, /*id*/ 2),
        )
        .await??;

        send_request(
            &mut first_client,
            "thread/list",
            /*id*/ 3,
            Some(json!({ "limit": 20 })),
        )
        .await?;
        let _: ThreadListResponse =
            to_response(read_response_for_id(&mut first_client, /*id*/ 3).await?)?;

        send_request(
            &mut first_client,
            "thread/attachment/add",
            /*id*/ 4,
            Some(json!({
                "threadId": thread_id.to_uppercase(),
                "attachmentType": "pull_request",
                "identityKey": r#"["github.com","openai","codex",123]"#,
                "payload": { "url": "https://github.com/openai/codex/pull/123" }
            })),
        )
        .await?;
        let first_message = read_jsonrpc_message(&mut first_client).await?;
        let JSONRPCMessage::Response(response) = first_message else {
            anyhow::bail!("attachment creation must respond before notifying the requester");
        };
        assert_eq!(response.id, RequestId::Integer(4));
        let created: ThreadAttachmentAddResponse = to_response(response)?;
        let attachment = created.attachment;
        let first_notification =
            read_notification_for_method(&mut first_client, "thread/attachment/updated").await?;
        let first_notification: ThreadAttachmentUpdatedNotification = serde_json::from_value(
            first_notification
                .params
                .context("attachment notification")?,
        )?;
        let second_notification =
            read_notification_for_method(&mut second_client, "thread/attachment/updated").await?;
        let second_notification: ThreadAttachmentUpdatedNotification = serde_json::from_value(
            second_notification
                .params
                .context("attachment notification")?,
        )?;

        assert_eq!(
            first_notification,
            ThreadAttachmentUpdatedNotification {
                thread_id: thread_id.clone(),
                attachment_type: "pull_request".to_string(),
                identity_key: r#"["github.com","openai","codex",123]"#.to_string(),
                attachment_id: attachment.id.clone(),
                operation: ThreadAttachmentOperation::Created,
            }
        );
        assert_eq!(first_notification, second_notification);

        send_request(
            &mut second_client,
            "thread/attachment/remove",
            /*id*/ 5,
            Some(json!({
                "threadId": format!("{{{thread_id}}}"),
                "attachmentType": "pull_request",
                "identityKey": r#"["github.com","openai","codex",123]"#
            })),
        )
        .await?;
        let first_message = read_jsonrpc_message(&mut second_client).await?;
        let JSONRPCMessage::Response(response) = first_message else {
            anyhow::bail!("attachment deletion must respond before notifying the requester");
        };
        assert_eq!(response.id, RequestId::Integer(5));
        let _: ThreadAttachmentRemoveResponse = to_response(response)?;

        let first_notification =
            read_notification_for_method(&mut first_client, "thread/attachment/updated").await?;
        let first_notification: ThreadAttachmentUpdatedNotification = serde_json::from_value(
            first_notification
                .params
                .context("attachment notification")?,
        )?;
        let second_notification =
            read_notification_for_method(&mut second_client, "thread/attachment/updated").await?;
        let second_notification: ThreadAttachmentUpdatedNotification = serde_json::from_value(
            second_notification
                .params
                .context("attachment notification")?,
        )?;
        assert_eq!(
            first_notification,
            ThreadAttachmentUpdatedNotification {
                thread_id,
                attachment_type: "pull_request".to_string(),
                identity_key: r#"["github.com","openai","codex",123]"#.to_string(),
                attachment_id: attachment.id,
                operation: ThreadAttachmentOperation::Deleted,
            }
        );
        assert_eq!(first_notification, second_notification);
        Ok(())
    }
    .await;

    process
        .kill()
        .await
        .context("failed to stop websocket app-server process")?;
    result
}

async fn send_sibling_test_turn(
    client: &mut super::connection_handling_websocket::WsClient,
    thread: &str,
    id: i64,
    text: &str,
) -> Result<()> {
    send_request(
        client,
        "turn/start",
        id,
        Some(json!({
            "threadId": thread,
            "input": [{"type": "text", "text": text}]
        })),
    )
    .await?;
    let (_, completed) =
        read_response_and_notification_for_method(client, id, "turn/completed").await?;
    assert_eq!(
        completed.params.context("turn completed params")?["turn"]["status"],
        "completed"
    );
    Ok(())
}
