use crate::app::test_support::make_test_app;
use crate::app_event::AppEvent;
use crate::app_event::ThreadTitleDestination;
use crate::app_server_session::AppServerSession;
use crate::app_server_session::ThreadParamsMode;
use crate::chatwidget::UserMessage;
use codex_app_server_protocol::JSONRPCMessage;
use codex_thread_store::sibling_context::SIBLING_CHILDREN_ATTACHMENT;
use color_eyre::eyre::Result;
use futures::SinkExt;
use futures::StreamExt;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::collections::HashSet;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn sibling_creation_keeps_roots_and_children_at_their_original_depth() -> Result<()> {
    let mut app = make_test_app().await;
    let mut server = crate::start_embedded_app_server_for_picker(&app.config).await?;
    let root = server.start_thread(&app.config).await?;
    let first = root.session.thread_id;
    app.enqueue_primary_thread_session(root.session, root.turns)
        .await?;
    let mut tui = crate::tui::test_support::make_test_tui()?;
    app.create_sibling(&mut tui, &mut server, None).await?;
    let second = app.chat_widget.thread_id().unwrap();
    assert_ne!(first, second);
    assert!(
        server
            .thread_read(second, false)
            .await?
            .forked_from_id
            .is_none()
    );
    app.create_sibling(&mut tui, &mut server, None).await?;
    let third = app.chat_widget.thread_id().unwrap();
    let roots: HashSet<_> = server
        .sibling_root_peers(first)
        .await?
        .into_iter()
        .collect();
    assert_eq!(roots, HashSet::from([first, second, third]));
    Box::pin(app.handle_event(
        &mut tui,
        &mut server,
        AppEvent::ForkCurrentSession {
            name: None,
            initial_user_message: None,
        },
    ))
    .await?;
    let first_child = app.chat_widget.thread_id().unwrap();
    app.create_sibling(
        &mut tui,
        &mut server,
        Some(UserMessage::from("Compare sibling approaches")),
    )
    .await?;
    let second_child = app.chat_widget.thread_id().unwrap();
    assert_ne!(first_child, second_child);
    for child in [first_child, second_child] {
        assert_eq!(
            server
                .thread_read(child, false)
                .await?
                .forked_from_id
                .as_deref(),
            Some(third.to_string().as_str())
        );
        assert!(
            server
                .sibling_attachments(child)
                .await?
                .iter()
                .any(
                    |attachment| attachment.attachment_type == SIBLING_CHILDREN_ATTACHMENT
                        && attachment.identity_key == third.to_string()
                )
        );
    }
    let roots: HashSet<_> = server
        .sibling_root_peers(third)
        .await?
        .into_iter()
        .collect();
    assert_eq!(roots, HashSet::from([first, second, third]));
    assert!(
        app.pending_thread_titles
            .contains_key(&(second_child, ThreadTitleDestination::Automatic))
    );
    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn sibling_creation_checks_backend_support_before_mutating_threads() -> Result<()> {
    let mut app = make_test_app().await;
    let mut embedded = crate::start_embedded_app_server_for_picker(&app.config).await?;
    let root = embedded.start_thread(&app.config).await?;
    app.enqueue_primary_thread_session(root.session, root.turns)
        .await?;
    let mut tui = crate::tui::test_support::make_test_tui()?;
    for support in [None, Some(false), Some(true)] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = crate::resolve_remote_addr(&format!("ws://{}", listener.local_addr()?))?;
        let backend = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            let mut socket = tokio_tungstenite::accept_async(stream).await?;
            let mut methods = Vec::new();
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let JSONRPCMessage::Request(request) = serde_json::from_str(&text)? else {
                    continue;
                };
                methods.push(request.method.clone());
                let response = if request.method == "initialize" {
                    let mut result = json!({"userAgent": "sibling-test/1.0.0"});
                    if let Some(support) = support {
                        result["supportsSiblingContext"] = support.into();
                    }
                    json!({"id": request.id, "result": result})
                } else {
                    assert_eq!(request.method, "thread/read");
                    json!({"id": request.id, "error": {"code": -32603, "message": "sibling-read-reached"}})
                };
                socket
                    .send(Message::Text(response.to_string().into()))
                    .await?;
            }
            Ok::<_, color_eyre::eyre::Report>(methods)
        });
        let mut server = AppServerSession::new(
            crate::backend_id::BackendId::new(),
            crate::connect_remote_app_server(endpoint).await?,
            ThreadParamsMode::Remote,
        );
        assert!(!server.uses_embedded_app_server());
        let error = app
            .create_sibling(&mut tui, &mut server, None)
            .await
            .unwrap_err();
        if support == Some(true) {
            assert!(
                format!("{error:#}").contains("sibling-read-reached"),
                "{error}"
            );
        } else {
            assert!(
                error
                    .to_string()
                    .contains("does not support live sibling context"),
                "{error}"
            );
        }
        server.shutdown().await?;
        let methods = backend.await??;
        assert_eq!(
            methods,
            if support == Some(true) {
                vec!["initialize", "thread/read"]
            } else {
                vec!["initialize"]
            }
        );
    }
    embedded.shutdown().await?;
    Ok(())
}
