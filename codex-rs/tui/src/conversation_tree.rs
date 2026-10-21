//! Saved fork ancestry, with navigation delegated to the normal resume lifecycle.
use std::collections::HashMap;
use std::collections::HashSet;

use codex_app_server_protocol::ThreadListParams;
use codex_app_server_protocol::ThreadSortKey;
use codex_protocol::ThreadId;
use color_eyre::eyre::Result;
use crossterm::event::KeyCode;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::Paragraph;
use ratatui::widgets::StatefulWidget;
use tokio_stream::StreamExt;

use crate::app_server_session::AppServerSession;
use crate::resume_picker::SessionSelection;
use crate::resume_picker::SessionTarget;
use crate::tui::Tui;
use crate::tui::TuiEvent;

struct Node {
    target: SessionTarget,
    parent: Option<ThreadId>,
    title: String,
}

struct Tree {
    nodes: Vec<Node>,
    children: Vec<Vec<usize>>,
    roots: Vec<usize>,
    collapsed: HashSet<usize>,
}

struct Row {
    node: usize,
    prefix: String,
    parent: Option<usize>,
}

impl Tree {
    /// Scope the forest to the current chat's fork family, including its ancestors
    /// and siblings. Keep edges through missing parents so orphaned siblings remain
    /// together, and use a visited set to tolerate malformed ancestry cycles.
    fn for_conversation(nodes: Vec<Node>, current: ThreadId) -> Self {
        let mut neighbors: HashMap<ThreadId, Vec<ThreadId>> = HashMap::new();
        for node in &nodes {
            if let Some(parent) = node.parent {
                neighbors
                    .entry(node.target.thread_id)
                    .or_default()
                    .push(parent);
                neighbors
                    .entry(parent)
                    .or_default()
                    .push(node.target.thread_id);
            }
        }
        let mut family = HashSet::new();
        let mut pending = vec![current];
        while let Some(id) = pending.pop() {
            if family.insert(id) {
                pending.extend(neighbors.get(&id).into_iter().flatten().copied());
            }
        }
        Self::new(
            nodes
                .into_iter()
                .filter(|node| family.contains(&node.target.thread_id))
                .collect(),
        )
    }

    fn new(nodes: Vec<Node>) -> Self {
        let indices: HashMap<_, _> = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (node.target.thread_id, i))
            .collect();
        let mut children = vec![Vec::new(); nodes.len()];
        let mut roots = Vec::new();
        // Break malformed ancestry cycles, keeping every conversation reachable.
        for (i, node) in nodes.iter().enumerate() {
            let parent = node.parent.and_then(|id| indices.get(&id).copied());
            let mut seen = HashSet::from([i]);
            let mut ancestor = parent;
            let mut cyclic = false;
            while let Some(index) = ancestor {
                if !seen.insert(index) {
                    cyclic = true;
                    break;
                }
                ancestor = nodes[index].parent.and_then(|id| indices.get(&id).copied());
            }
            if let Some(parent) = parent.filter(|_| !cyclic) {
                children[parent].push(i);
            } else {
                roots.push(i);
            }
        }
        Self {
            nodes,
            children,
            roots,
            collapsed: HashSet::new(),
        }
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut stack: Vec<_> = self
            .roots
            .iter()
            .rev()
            .map(|&index| (index, String::new(), None, true))
            .collect();
        while let Some((node, indent, parent, last)) = stack.pop() {
            let prefix = if parent.is_none() {
                String::new()
            } else {
                format!("{indent}{}", if last { "└── " } else { "├── " })
            };
            rows.push(Row {
                node,
                prefix,
                parent,
            });
            if self.collapsed.contains(&node) {
                continue;
            }
            let child_indent = if parent.is_none() {
                String::new()
            } else {
                format!("{indent}{}", if last { "    " } else { "│   " })
            };
            for (position, &child) in self.children[node].iter().enumerate().rev() {
                stack.push((
                    child,
                    child_indent.clone(),
                    Some(node),
                    position + 1 == self.children[node].len(),
                ));
            }
        }
        rows
    }
}

async fn load(app_server: &mut AppServerSession, current: Option<ThreadId>) -> Result<Tree> {
    let Some(current) = current else {
        return Ok(Tree::new(Vec::new()));
    };
    let mut nodes = Vec::new();
    let mut seen = HashSet::new();
    // Database listing includes fresh branches; rollout listing also covers
    // older conversations not yet indexed. Merge both by thread ID.
    for use_state_db_only in [true, false] {
        let mut cursor = None;
        let mut cursors = HashSet::new();
        loop {
            let page = app_server
                .thread_list(ThreadListParams {
                    cursor,
                    limit: Some(100),
                    sort_key: Some(ThreadSortKey::UpdatedAt),
                    sort_direction: None,
                    model_providers: None,
                    source_kinds: Some(crate::resume_source_kinds(false)),
                    originators: None,
                    archived: Some(false),
                    section_id: None,
                    project_id: None,
                    cwd: None,
                    use_state_db_only,
                    search_term: None,
                    parent_thread_id: None,
                    ancestor_thread_id: None,
                    excluded_thread_ids: None,
                })
                .await?;
            for mut thread in page.data {
                let Ok(thread_id) = ThreadId::from_string(&thread.id) else {
                    continue;
                };
                if !seen.insert(thread_id) {
                    continue;
                }
                // Legacy thread/list summaries omit fork ancestry even though the
                // saved thread contains it. Read metadata (without turns) before
                // filtering the family, including siblings and nested branches.
                if thread.forked_from_id.is_none() {
                    thread.forked_from_id = app_server
                        .thread_read(thread_id, /*include_turns*/ false)
                        .await?
                        .forked_from_id;
                }
                let title = thread
                    .name
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or(thread.preview);
                let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
                nodes.push(Node {
                    target: SessionTarget {
                        backend_id: app_server.backend_id(),
                        path: thread.path,
                        thread_id,
                        cwd: Some(thread.cwd.to_path_buf()),
                        history_mode: Some(thread.history_mode),
                    },
                    parent: thread
                        .forked_from_id
                        .and_then(|id| ThreadId::from_string(&id).ok()),
                    title: if title.is_empty() {
                        format!("Untitled · {thread_id}")
                    } else {
                        title
                    },
                });
            }
            cursor = page.next_cursor;
            match &cursor {
                None => break,
                Some(next) if !cursors.insert(next.clone()) => {
                    color_eyre::eyre::bail!(
                        "Conversation list returned a repeated pagination cursor"
                    );
                }
                Some(_) => {}
            }
        }
    }
    let mut pending = vec![current];
    let mut visited = HashSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        if let Some(node) = nodes.iter().find(|node| node.target.thread_id == id) {
            pending.extend(node.parent);
            continue;
        }
        let thread = match app_server.thread_read(id, false).await {
            Ok(thread) => thread,
            Err(error) if id != current => {
                tracing::warn!(%id, %error, "Could not read conversation ancestor");
                continue;
            }
            Err(error) => return Err(error),
        };
        let parent = thread
            .forked_from_id
            .and_then(|id| ThreadId::from_string(&id).ok());
        let title = thread
            .name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(thread.preview);
        nodes.push(Node {
            target: SessionTarget {
                backend_id: app_server.backend_id(),
                path: thread.path,
                thread_id: id,
                cwd: Some(thread.cwd.to_path_buf()),
                history_mode: Some(thread.history_mode),
            },
            parent,
            title: if title.trim().is_empty() {
                format!("Untitled · {id}")
            } else {
                title.split_whitespace().collect::<Vec<_>>().join(" ")
            },
        });
        pending.extend(parent);
    }
    Ok(Tree::for_conversation(nodes, current))
}

struct Screen<'a>(&'a mut Tui);
impl Drop for Screen<'_> {
    fn drop(&mut self) {
        let _ = self.0.leave_alt_screen();
    }
}

pub(crate) async fn run(
    tui: &mut Tui,
    app_server: &mut AppServerSession,
    current: Option<ThreadId>,
) -> Result<SessionSelection> {
    tui.enter_alt_screen()?;
    let screen = Screen(tui);
    let mut events = screen.0.event_stream();
    let loading = load(app_server, current);
    tokio::pin!(loading);
    let mut tree = None;
    let mut selected = 0usize;
    let mut list_state = ListState::default();
    let mut screen_size = screen.0.terminal.last_known_screen_size;
    loop {
        let rows = tree.as_ref().map(Tree::rows).unwrap_or_default();
        selected = selected.min(rows.len().saturating_sub(1));
        list_state.select((!rows.is_empty()).then_some(selected));
        screen.0.draw(screen_size.height, |frame| {
            let [heading, body, footer] = Layout::vertical([
                Constraint::Length(2), Constraint::Min(1), Constraint::Length(2),
            ]).areas(frame.area());
            frame.render_widget_ref(&Paragraph::new("Conversation tree · current chat family").bold(), heading);
            if let Some(tree) = &tree {
                let items: Vec<_> = rows.iter().map(|row| {
                    let node = &tree.nodes[row.node];
                    let marker = if tree.children[row.node].is_empty() { " " }
                        else if tree.collapsed.contains(&row.node) { "▸" } else { "▾" };
                    let active = if Some(node.target.thread_id) == current { " ← current" } else { "" };
                    let item = ListItem::new(format!("{}{marker} {}{active}", row.prefix, node.title));
                    if Some(node.target.thread_id) == current { item.bold() } else { item }
                }).collect();
                if items.is_empty() {
                    frame.render_widget_ref(&Paragraph::new("No saved conversations in this chat family."), body);
                } else {
                    StatefulWidget::render(List::new(items)
                        .highlight_style(Style::default().reversed()).highlight_symbol("› "), body, frame.buffer_mut(), &mut list_state);
                }
            } else {
                frame.render_widget_ref(&Paragraph::new("Loading saved conversations…"), body);
            }
            frame.render_widget_ref(&Paragraph::new("↑/↓ select · ←/→ collapse/expand · Enter open · Esc back\n/branch [prompt] creates a child and starts your prompt"), footer);
        })?;
        tokio::select! {
            result = &mut loading, if tree.is_none() => {
                let loaded = result?;
                selected = loaded.rows().iter().position(|row| Some(loaded.nodes[row.node].target.thread_id) == current).unwrap_or(0);
                tree = Some(loaded);
            }
            event = events.next() => {
                let Some(event) = event else { return Ok(SessionSelection::StartFresh); };
                screen_size = screen.0.screen_size_for_event(&event)?;
                let TuiEvent::Key(key) = event else { continue; };
                if key.kind == KeyEventKind::Release { continue; }
                if key.code == KeyCode::Esc || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)) {
                    return Ok(SessionSelection::StartFresh);
                }
                let Some(tree) = tree.as_mut() else { continue; };
                let Some(row) = rows.get(selected) else { continue; };
                match key.code {
                    KeyCode::Up => selected = selected.saturating_sub(1),
                    KeyCode::Down => selected = (selected + 1).min(rows.len() - 1),
                    KeyCode::Home => selected = 0,
                    KeyCode::End => selected = rows.len() - 1,
                    KeyCode::Left => {
                        if tree.children[row.node].is_empty() || !tree.collapsed.insert(row.node) {
                            if let Some(parent) = row.parent {
                                selected = rows.iter().position(|row| row.node == parent).unwrap_or(selected);
                            }
                        }
                    }
                    KeyCode::Right => {
                        if !tree.collapsed.remove(&row.node) && !tree.children[row.node].is_empty() {
                            selected += 1;
                        }
                    }
                    KeyCode::Enter => return Ok(SessionSelection::Resume(tree.nodes[row.node].target.clone())),
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn load_fresh_conversation_uses_its_active_server() -> Result<()> {
        use crate::app_server_session::ThreadParamsMode;
        use crate::legacy_core::config::ConfigBuilder;
        use crate::tests::start_test_embedded_app_server;

        let home = tempfile::TempDir::new()?;
        let config = ConfigBuilder::default()
            .codex_home(home.path().to_path_buf())
            .build()
            .await?;
        let mut active = AppServerSession::new(
            crate::backend_id::BackendId::new(),
            codex_app_server_client::AppServerClient::InProcess(
                start_test_embedded_app_server(config.clone()).await?,
            ),
            ThreadParamsMode::Embedded,
        );
        let started = active.start_thread(&config).await?;
        let current = started.session.thread_id;
        let mut separate = AppServerSession::new(
            crate::backend_id::BackendId::new(),
            codex_app_server_client::AppServerClient::InProcess(
                start_test_embedded_app_server(config).await?,
            ),
            ThreadParamsMode::Embedded,
        );
        // Before the first user message, another server cannot find this chat.
        let error = load(&mut separate, Some(current))
            .await
            .err()
            .expect("fresh chat is not persisted");
        assert!(format!("{error:#}").contains("thread not loaded"));
        let tree = load(&mut active, Some(current)).await?;
        assert_eq!(tree.nodes.len(), 1);
        assert_eq!(tree.nodes[0].target.thread_id, current);
        separate.shutdown().await?;
        active.shutdown().await?;
        Ok(())
    }

    #[tokio::test]
    async fn load_recovers_legacy_fork_ancestry() -> Result<()> {
        use crate::app_server_session::ThreadParamsMode;
        use crate::legacy_core::config::ConfigBuilder;
        use crate::tests::start_test_embedded_app_server;
        use codex_protocol::protocol::SessionMeta;
        use codex_protocol::protocol::SessionMetaLine;
        use codex_protocol::protocol::SessionSource;
        use serde_json::json;

        let home = tempfile::TempDir::new()?;
        let config = ConfigBuilder::default()
            .codex_home(home.path().to_path_buf())
            .build()
            .await?;
        let root = ThreadId::new();
        let child = ThreadId::new();
        let sibling = ThreadId::new();
        let unrelated = ThreadId::new();
        let directory = home.path().join("sessions/2025/02/01");
        std::fs::create_dir_all(&directory)?;
        for (id, parent) in [
            (root, None),
            (child, Some(root)),
            (sibling, Some(root)),
            (unrelated, None),
        ] {
            let meta = SessionMetaLine {
                meta: SessionMeta {
                    id,
                    session_id: id.into(),
                    forked_from_id: parent,
                    timestamp: "2025-02-01T10:00:00Z".to_string(),
                    cwd: home.path().to_path_buf(),
                    source: SessionSource::Cli,
                    model_provider: Some(config.model_provider_id.clone()),
                    ..Default::default()
                },
                git: None,
            };
            let lines = [
                json!({"timestamp": "2025-02-01T10:00:00Z", "type": "session_meta", "payload": meta}),
                json!({"timestamp": "2025-02-01T10:00:00Z", "type": "event_msg", "payload": {"type": "user_message", "message": "branch test", "kind": "plain"}}),
            ];
            std::fs::write(
                directory.join(format!("rollout-2025-02-01T10-00-00-{id}.jsonl")),
                lines[..if id == child { 1 } else { lines.len() }]
                    .iter()
                    .map(serde_json::Value::to_string)
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n",
            )?;
        }
        let mut server = AppServerSession::new(
            crate::backend_id::BackendId::new(),
            codex_app_server_client::AppServerClient::InProcess(
                start_test_embedded_app_server(config).await?,
            ),
            ThreadParamsMode::Embedded,
        );
        let tree = load(&mut server, Some(child)).await?;
        server.shutdown().await?;
        let ids: HashSet<_> = tree
            .nodes
            .iter()
            .map(|node| node.target.thread_id)
            .collect();
        assert_eq!(ids, HashSet::from([root, child, sibling]));
        assert_eq!(tree.roots.len(), 1);
        assert_eq!(tree.nodes[tree.roots[0]].target.thread_id, root);
        Ok(())
    }

    fn node(id: ThreadId, parent: Option<ThreadId>, title: &str) -> Node {
        Node {
            target: SessionTarget {
                backend_id: crate::backend_id::BackendId::new(),
                path: None,
                thread_id: id,
                cwd: None,
                history_mode: None,
            },
            parent,
            title: title.into(),
        }
    }

    #[test]
    fn siblings_nested_branches_and_collapse_preserve_targets() {
        let root = ThreadId::new();
        let child = ThreadId::new();
        let sibling = ThreadId::new();
        let grandchild = ThreadId::new();
        // Newer children can precede their parents in paginated list results.
        let mut tree = Tree::new(vec![
            node(grandchild, Some(child), "nested"),
            node(child, Some(root), "child"),
            node(sibling, Some(root), "sibling"),
            node(root, None, "root"),
        ]);
        let rows = tree.rows();
        assert_eq!(
            rows.iter().map(|row| row.node).collect::<Vec<_>>(),
            vec![3, 1, 0, 2]
        );
        assert_eq!(rows[1].prefix, "├── ");
        assert_eq!(rows[2].prefix, "│   └── ");
        assert_eq!(rows[3].prefix, "└── ");
        assert_eq!(tree.nodes[rows[2].node].target.thread_id, grandchild);
        tree.collapsed.insert(1);
        assert_eq!(
            tree.rows().iter().map(|row| row.node).collect::<Vec<_>>(),
            vec![3, 1, 2]
        );
        tree.collapsed.remove(&1);
        assert_eq!(tree.rows().len(), 4);
    }

    #[test]
    fn missing_parents_and_cycles_remain_reachable() {
        let a = ThreadId::new();
        let b = ThreadId::new();
        let tree = Tree::new(vec![
            node(a, Some(b), "a"),
            node(b, Some(a), "b"),
            node(ThreadId::new(), Some(ThreadId::new()), "orphan"),
        ]);
        let rows = tree.rows();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| row.parent.is_none()));
    }

    #[test]
    fn any_branch_shows_its_root_siblings_and_descendants_only() {
        let root = ThreadId::new();
        let child = ThreadId::new();
        let sibling = ThreadId::new();
        let nested = ThreadId::new();
        let unrelated = ThreadId::new();
        for current in [root, child, sibling, nested] {
            let tree = Tree::for_conversation(
                vec![
                    node(unrelated, None, "unrelated"),
                    node(ThreadId::new(), Some(unrelated), "unrelated child"),
                    node(nested, Some(child), "nested"),
                    node(child, Some(root), "child"),
                    node(sibling, Some(root), "sibling"),
                    node(root, None, "root"),
                ],
                current,
            );
            let rows = tree.rows();
            let ids: Vec<_> = rows
                .iter()
                .map(|row| tree.nodes[row.node].target.thread_id)
                .collect();
            assert_eq!(ids, vec![root, child, nested, sibling]);
            // The same ID used for initial selection and the current marker remains present.
            assert_eq!(ids.iter().filter(|&&id| id == current).count(), 1);
        }
    }

    #[test]
    fn missing_current_never_falls_back_to_unrelated_history() {
        let tree = Tree::for_conversation(
            vec![node(ThreadId::new(), None, "unrelated")],
            ThreadId::new(),
        );
        assert!(tree.rows().is_empty());
    }

    #[test]
    fn family_keeps_siblings_with_a_missing_parent() {
        let missing = ThreadId::new();
        let current = ThreadId::new();
        let sibling = ThreadId::new();
        let tree = Tree::for_conversation(
            vec![
                node(current, Some(missing), "current"),
                node(sibling, Some(missing), "sibling"),
                node(ThreadId::new(), None, "unrelated"),
            ],
            current,
        );
        assert_eq!(tree.rows().len(), 2);
    }
}
