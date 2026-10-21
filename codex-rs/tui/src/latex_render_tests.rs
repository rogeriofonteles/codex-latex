use super::rewrite_with;
use pretty_assertions::assert_eq;

#[test]
fn preserves_code_links_currency_and_unfinished_formulas() {
    let source = r#"Price $5 and $10; `\[literal\]` and [\[link\]](https://example.com).
```latex
\[code\]
```
Unfinished \[x^2"#;
    assert_eq!(
        rewrite_with(source, |_| panic!("must remain literal")),
        source
    );
}

#[test]
fn renders_complete_math_and_preserves_unicode_prose() {
    let mut expressions = Vec::new();
    let source = r"Área: \[\frac{a}{b}\] então $$x^2$$ and \(α+β\).";
    let rendered = rewrite_with(source, |formula| {
        expressions.push(formula.to_owned());
        Some(vec!["[formula]".to_owned()])
    });
    assert_eq!(expressions, vec![r"\frac{a}{b}", "x^2", "α+β"]);
    assert_eq!(rendered.matches("```text\n[formula]\n```").count(), 3);
    assert!(rendered.starts_with("Área: "));
    assert!(rendered.ends_with('.'));
}

#[test]
fn math_images_stay_inside_markdown_table_cells() {
    let source = "| First | Second |\n| --- | --- |\n| \\(x^2\\) | \\(y^2\\) |\n";
    let rewritten = rewrite_with(source, |formula| {
        Some(match formula {
            "x^2" => vec![
                "[m:0123456789ab:00]".to_owned(),
                "[m:0123456789ab:01]".to_owned(),
            ],
            "y^2" => vec![
                "[m:abcdef012345:00]".to_owned(),
                "[m:abcdef012345:01]".to_owned(),
            ],
            _ => panic!("unexpected formula: {formula}"),
        })
    });
    assert_eq!(rewritten.lines().count(), 3);
    assert!(!rewritten.contains("```text"));

    let rendered = crate::markdown_render::render_markdown_text_with_width(&rewritten, Some(80));
    let visible = rendered
        .lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert_eq!(visible.len(), 4, "{visible:#?}");
    assert!(
        visible
            .iter()
            .any(|line| line.contains("━") || line.contains("─"))
    );
    assert!(
        visible
            .iter()
            .any(|line| line.contains("[m:0123456789ab:00]"))
    );
    assert!(
        visible
            .iter()
            .any(|line| line.contains("[m:abcdef012345:00]"))
    );
}

#[test]
fn failed_render_keeps_source_and_later_formulas_work() {
    let source = r"\[bad\] followed by \[good\]";
    assert_eq!(
        rewrite_with(source, |formula| {
            (formula == "good").then(|| vec!["image".to_owned()])
        }),
        "\\[bad\\] followed by \n\n```text\nimage\n```\n\n"
    );
}

#[test]
fn chat_reserves_formula_rows_snapshot() {
    let rewritten = rewrite_with("Before.\n\n\\[x^2\\]\n\nAfter.", |_| {
        Some(vec![
            "[m:0123456789ab:00]".to_owned(),
            "[m:0123456789ab:01]".to_owned(),
        ])
    });
    let mut lines = Vec::new();
    crate::markdown::append_markdown(&rewritten, Some(60), None, &mut lines);
    let text = lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(text);
}

#[cfg(unix)]
#[test]
fn warm_worker_returns_reserved_rows() {
    use std::io::Read;
    use std::io::Write;
    use std::os::unix::net::UnixListener;

    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("renderer.sock")).unwrap();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        stream.read_to_string(&mut request).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&request).unwrap(),
            serde_json::json!({"source": "x^2", "width": 80})
        );
        stream.write_all(br#"["[m:0123456789ab:00]"]"#).unwrap();
    });
    assert_eq!(
        super::request_render("x^2", 80, directory.path()),
        Some(vec!["[m:0123456789ab:00]".to_owned()])
    );
    worker.join().unwrap();
}

#[cfg(unix)]
#[test]
fn stalled_worker_falls_back_without_waiting_for_it() {
    use std::os::unix::net::UnixListener;
    use std::time::Duration;
    use std::time::Instant;

    let directory = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(directory.path().join("renderer.sock")).unwrap();
    let (release, wait) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        let _ = wait.recv_timeout(Duration::from_secs(2));
    });
    let start = Instant::now();
    assert_eq!(super::request_render("x^2", 80, directory.path()), None);
    assert!(start.elapsed() < Duration::from_secs(1));
    release.send(()).unwrap();
    worker.join().unwrap();
}

#[test]
fn resize_timeout_preserves_images_and_retries() {
    let mut cache = super::FormulaCache::default();
    let old = vec!["[m:0123456789ab:00]".to_owned()];
    let resized = vec!["[m:abcdef012345:00]".to_owned()];
    assert_eq!(
        cache.render("x^2", 80, "old", || Some(old.clone())),
        Some(old.clone())
    );
    assert_eq!(cache.render("x^2", 60, "new", || None), Some(old));
    assert_eq!(
        cache.render("x^2", 60, "new", || Some(resized.clone())),
        Some(resized.clone())
    );
    assert_eq!(
        cache.render("x^2", 60, "new", || panic!("cached")),
        Some(resized)
    );
}

#[test]
fn initial_render_failure_is_retried() {
    let mut cache = super::FormulaCache::default();
    assert_eq!(cache.render("x^2", 80, "size", || None), None);
    let lines = vec!["[m:0123456789ab:00]".to_owned()];
    assert_eq!(
        cache.render("x^2", 80, "size", || Some(lines.clone())),
        Some(lines)
    );
}

#[test]
fn displayed_math_preserves_surrounding_emphasis() {
    for (source, expected) in [
        (
            r"**before \(x\) after**",
            "**before** \n\n```text\nimage\n```\n\n **after**",
        ),
        (r"**\(x\)**", "\n\n```text\nimage\n```\n\n"),
        (r"**value \(x\)**", "**value** \n\n```text\nimage\n```\n\n"),
        (
            r"_before \(x\) after_",
            "_before_ \n\n```text\nimage\n```\n\n _after_",
        ),
        (
            r"***before \(x\) after***",
            "***before*** \n\n```text\nimage\n```\n\n ***after***",
        ),
    ] {
        let rewritten = rewrite_with(source, |_| Some(vec!["image".to_owned()]));
        assert_eq!(rewritten, expected, "source: {source}");
        let mut lines = Vec::new();
        crate::markdown::append_markdown(&rewritten, Some(80), None, &mut lines);
        let visible = lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!visible.contains('*'), "literal emphasis in {visible}");
        assert!(!visible.contains('_'), "literal emphasis in {visible}");
    }
}
