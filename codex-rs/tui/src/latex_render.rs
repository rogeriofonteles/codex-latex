//! Optional local display-math adapter. The launcher supplies a private image cache and helper.
//! Only presentation changes: source text remains in the conversation and clipboard export.
use pulldown_cmark::Event;
use pulldown_cmark::Options;
use pulldown_cmark::Parser;
use pulldown_cmark::Tag;
use pulldown_cmark::TagEnd;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::OnceLock;

pub(crate) fn rewrite(source: &str, width: Option<usize>) -> String {
    if std::env::var_os("CODEX_LATEX_HELPER").is_none()
        || !["\\[", "$$", "\\("]
            .iter()
            .any(|delimiter| source.contains(delimiter))
    {
        return source.to_owned();
    }
    let Some(directory) = std::env::var_os("CODEX_LATEX_CACHE") else {
        return source.to_owned();
    };
    let directory = std::path::PathBuf::from(directory);
    let geometry = std::fs::read_to_string(directory.join("geometry.json")).unwrap_or_default();
    let width = width.unwrap_or(80).saturating_sub(4);
    rewrite_with(source, |formula| {
        static CACHE: OnceLock<Mutex<FormulaCache>> = OnceLock::new();
        CACHE
            .get_or_init(Mutex::default)
            .lock()
            .ok()?
            .render(formula, width, &geometry, || {
                request_render(formula, width, &directory)
            })
    })
}

#[derive(Default)]
struct FormulaCache {
    images: HashMap<(String, usize, String), Vec<String>>,
}

impl FormulaCache {
    fn render(
        &mut self,
        formula: &str,
        width: usize,
        geometry: &str,
        request: impl FnOnce() -> Option<Vec<String>>,
    ) -> Option<Vec<String>> {
        let key = (formula.to_owned(), width, geometry.to_owned());
        if let Some(lines) = self.images.get(&key) {
            return Some(lines.clone());
        }
        let Some(lines) = request() else {
            // A resize can outpace the image worker. Keep a previous successful
            // image visible, but never cache a transient failure at the new size.
            return self
                .images
                .iter()
                .filter(|((source, _, _), _)| source == formula)
                .min_by_key(|((_, old_width, old_geometry), _)| {
                    (old_geometry != geometry, old_width.abs_diff(width))
                })
                .map(|(_, lines)| lines.clone());
        };
        if self.images.len() >= 128 {
            self.images.clear();
        }
        self.images.insert(key, lines.clone());
        Some(lines)
    }
}

// A warm local worker avoids spawning Node on the UI thread. Bound IPC latency so a
// busy or unavailable renderer falls back to text instead of freezing the chat.
#[cfg(unix)]
fn request_render(formula: &str, width: usize, directory: &Path) -> Option<Vec<String>> {
    use std::io::Read;
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    if formula.len() > 8192 {
        return None;
    }
    let mut stream = UnixStream::connect(directory.join("renderer.sock")).ok()?;
    let deadline = Some(Duration::from_millis(250));
    stream.set_read_timeout(deadline).ok()?;
    stream.set_write_timeout(deadline).ok()?;
    serde_json::to_writer(
        &mut stream,
        &serde_json::json!({"source": formula, "width": width}),
    )
    .ok()?;
    stream.shutdown(Shutdown::Write).ok()?;
    let mut output = Vec::new();
    stream.take(4096).read_to_end(&mut output).ok()?;
    let lines: Vec<String> = serde_json::from_slice(&output).ok()?;
    (!lines.is_empty() && lines.len() <= 20).then_some(lines)
}

#[cfg(not(unix))]
fn request_render(_formula: &str, _width: usize, _directory: &Path) -> Option<Vec<String>> {
    None
}

// A displayed formula splits a Markdown paragraph. Reconstruct emphasis on each
// side rather than leaving an opening/closing delimiter in separate paragraphs.
fn prose_fragment(
    source: &str,
    fragment: std::ops::Range<usize>,
    emphasis: &[(std::ops::Range<usize>, usize)],
) -> String {
    let mut output = String::new();
    let mut cursor = fragment.start;
    for (index, (span, marker_len)) in emphasis.iter().enumerate() {
        if span.end <= cursor || span.start >= fragment.end {
            continue;
        }
        let start = span.start.max(cursor);
        let end = span.end.min(fragment.end);
        output.push_str(&source[cursor..start]);
        if span.start >= fragment.start && span.end <= fragment.end {
            output.push_str(&source[start..end]);
        } else {
            let content_start = (span.start + marker_len).max(start);
            let content_end = (span.end - marker_len).min(end);
            if content_start < content_end {
                let content =
                    prose_fragment(source, content_start..content_end, &emphasis[index + 1..]);
                let trimmed = content.trim();
                if trimmed.is_empty() {
                    output.push_str(&content);
                } else {
                    let leading = content.len() - content.trim_start().len();
                    let trailing = content.trim_end().len();
                    let marker = &source[span.start..span.start + marker_len];
                    output.push_str(&content[..leading]);
                    output.push_str(marker);
                    output.push_str(trimmed);
                    output.push_str(marker);
                    output.push_str(&content[trailing..]);
                }
            }
        }
        cursor = end;
    }
    output.push_str(&source[cursor..fragment.end]);
    output
}

fn rewrite_with(source: &str, mut render: impl FnMut(&str) -> Option<Vec<String>>) -> String {
    // CommonMark offsets protect code, links and HTML from accidental math conversion.
    let mut protected = Vec::new();
    let mut tables = Vec::new();
    let mut table_start = None;
    let mut emphasis = Vec::new();
    let mut code_start = None;
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Table(_)) => table_start = Some(range.start),
            Event::End(TagEnd::Table) => {
                if let Some(start) = table_start.take() {
                    tables.push(start..range.end);
                }
            }
            Event::Start(Tag::Strong) => emphasis.push((range, 2)),
            Event::Start(Tag::Emphasis) => emphasis.push((range, 1)),
            Event::Start(Tag::CodeBlock(_)) => code_start = Some(range.start),
            Event::End(TagEnd::CodeBlock) => {
                if let Some(start) = code_start.take() {
                    protected.push(start..range.end);
                }
            }
            Event::Code(_)
            | Event::Html(_)
            | Event::InlineHtml(_)
            | Event::Start(Tag::Link { .. })
            | Event::Start(Tag::Image { .. }) => protected.push(range),
            _ => {}
        }
    }
    if let Some(start) = code_start {
        protected.push(start..source.len());
    }
    let mut output = String::new();
    let mut copied = 0;
    let mut i = 0;
    while i < source.len() {
        let rest = &source[i..];
        let delimiter = [("\\[", "\\]"), ("$$", "$$"), ("\\(", "\\)")]
            .into_iter()
            .find(|(open, _)| rest.starts_with(open));
        if let Some((open, close)) = delimiter
            && !protected.iter().any(|r| r.contains(&i))
            && (i == 0 || source.as_bytes()[i - 1] != b'\\')
            && let Some(end) = rest[open.len()..].find(close)
        {
            let end = i + open.len() + end;
            let after = end + close.len();
            if !protected.iter().any(|r| r.start < after && r.end > i)
                && let Some(lines) = render(source[i + open.len()..end].trim())
            {
                output.push_str(&prose_fragment(source, copied..i, &emphasis));
                if tables.iter().any(|table| table.contains(&i)) {
                    // Keep the Markdown row intact. The table-cell writer expands this private
                    // separator into terminal lines after pulldown-cmark has parsed the row.
                    output.push_str(&lines.join("\u{e000}"));
                } else {
                    output.push_str("\n\n```text\n");
                    output.push_str(&lines.join("\n"));
                    output.push_str("\n```\n\n");
                }
                copied = after;
                i = after;
                continue;
            }
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    if copied == 0 {
        return source.to_owned();
    }
    output.push_str(&prose_fragment(source, copied..source.len(), &emphasis));
    output
}

#[cfg(test)]
#[path = "latex_render_tests.rs"]
mod tests;
