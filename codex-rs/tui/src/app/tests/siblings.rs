use crate::app::test_support::make_test_app;
use crate::app_event::AppEvent;
use crate::app_event::ThreadTitleDestination;
use crate::chatwidget::UserMessage;
use codex_thread_store::sibling_context::SIBLING_CHILDREN_ATTACHMENT;
use color_eyre::eyre::Result;
use pretty_assertions::assert_eq;
use std::collections::HashSet;

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
