//! Texts shown to the agent: server instructions, the review prompt and the
//! rendering of user feedback into MCP content blocks.

use std::fmt::Write as _;
use std::path::Path;

use base64::Engine as _;
use serde_json::{Value, json};

use super::{RpcResult, code};
use crate::document::DocStatus;
use crate::feedback::{FeedbackItem, FeedbackKind, FeedbackShape, ShapeKind};

pub const INSTRUCTIONS: &str = "\
pdfnotator is a PDF viewer next to you for pair-designing documents (usually Typst) with the user. \
When they say \"open in pdfnotator\" (\"öffne in pdfnotator\"), call `open_pdf` with the .typ source, then loop: \
`wait_for_feedback` → edit the source (pdfnotator recompiles and reloads itself) → `reply` with a one-line summary \
and the handled ids → `wait_for_feedback` again, until the user says they are done. Feedback is compact by default; \
call `get_comments` only when you need full quotes, surrounding page text or full-resolution screenshots.";

const REVIEW_PROMPT: &str = "\
Open {FILE} in pdfnotator (call `open_pdf` with the absolute path of the .typ source if there is one) and \
pair-design it with me: loop `wait_for_feedback` → edit the source → `reply` (summary + resolved_ids) → \
`wait_for_feedback` until I say we are done. pdfnotator recompiles and reloads automatically.";

pub fn review_prompt_definition() -> Value {
    json!({
        "name": "review",
        "title": "Pair-design in pdfnotator",
        "description": "Open a Typst/PDF document in pdfnotator and loop on the user's feedback",
        "arguments": [{
            "name": "file",
            "description": "Document (.typ or .pdf); default: the one you are working on",
            "required": false
        }]
    })
}

pub fn review_prompt(params: &Value) -> RpcResult {
    if params.get("name").and_then(Value::as_str) != Some("review") {
        return Err((code::INVALID_PARAMS, "unknown prompt".into()));
    }
    let file = params
        .pointer("/arguments/file")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map_or_else(
            || "the Typst document we are working on".to_owned(),
            |f| format!("`{f}`"),
        );
    Ok(json!({
        "description": "Pair-design in pdfnotator",
        "messages": [{ "role": "user", "content": { "type": "text", "text": REVIEW_PROMPT.replace("{FILE}", &file) } }]
    }))
}

pub fn text(s: impl Into<String>) -> Value {
    json!({ "type": "text", "text": s.into() })
}

/// Warning appended to tool results while the document does not compile.
/// Default limits keep tool results small; `get_comments` returns everything.
const QUOTE_LIMIT: usize = 400;
const AREA_TEXT_LIMIT: usize = 200;
const SHAPE_TEXT_LIMIT: usize = 80;
const ERROR_LIMIT: usize = 1500;

/// Shorten `s` to about `limit` characters (at a word boundary). Returns the
/// text and whether it was cut.
pub fn shorten(s: &str, limit: usize) -> (String, bool) {
    if s.chars().count() <= limit {
        return (s.to_owned(), false);
    }
    let cut: String = s.chars().take(limit).collect();
    let cut = cut
        .rfind(char::is_whitespace)
        .filter(|&i| i > limit / 2)
        .map_or(cut.as_str(), |i| &cut[..i]);
    let rest = s.chars().count() - cut.chars().count();
    (format!("{} … [+{rest} chars]", cut.trim_end()), true)
}

/// Warning appended to tool results while the document does not compile.
pub fn compile_note(error: Option<&str>) -> String {
    error.map_or_else(String::new, |e| {
        let (e, _) = shorten(e, ERROR_LIMIT);
        format!("\n\n⚠️ The document does NOT compile — fix this first:\n```\n{e}\n```")
    })
}

/// How much of a comment to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// Default for `wait_for_feedback`: shortened texts, small preview image.
    Compact,
    /// For `get_comments`: everything, full-resolution image.
    Full,
}

/// Render user feedback for `wait_for_feedback` (compact).
pub fn feedback_content(items: &[FeedbackItem], status: &DocStatus, image_dir: &Path) -> Vec<Value> {
    let note = compile_note(status.compile_error.as_deref());
    if items.is_empty() {
        let doc = status
            .source
            .as_deref()
            .or(status.path.as_deref())
            .unwrap_or_default();
        return vec![text(format!(
            "No feedback yet (user on page {}/{}). Keep waiting: call wait_for_feedback again with path \"{doc}\" \
             until the user says they are done.{note}",
            status.current_page, status.page_count
        ))];
    }
    let doc = status
        .source
        .as_deref()
        .or(status.path.as_deref())
        .unwrap_or("the document");
    let mut out = vec![text(format!(
        "{} comment(s) on {doc} (user on page {}):",
        items.len(),
        status.current_page
    ))];
    let mut shortened = false;
    for item in items {
        let (blocks, cut) = render_item(item, Detail::Compact, image_dir);
        shortened |= cut;
        out.extend(blocks);
    }
    let more = if shortened {
        " Some texts were shortened — call get_comments(ids) if you need them in full."
    } else {
        ""
    };
    out.push(text(format!(
        "Next: edit the source, `reply` with resolved_ids, then wait_for_feedback again.{more}{note}"
    )));
    out
}

/// Render comments with all details (for `get_comments`).
pub fn details_content(items: &[FeedbackItem], image_dir: &Path) -> Vec<Value> {
    items
        .iter()
        .flat_map(|item| render_item(item, Detail::Full, image_dir).0)
        .collect()
}

/// Content blocks for one comment and whether something was left out.
fn render_item(item: &FeedbackItem, detail: Detail, image_dir: &Path) -> (Vec<Value>, bool) {
    let full = detail == Detail::Full;
    let mut cut = false;
    let mut s = String::new();
    let _ = match item.kind {
        FeedbackKind::Text => writeln!(s, "## [{}] page {} · text", item.id, item.page),
        FeedbackKind::Area if !item.shapes.is_empty() => {
            writeln!(s, "## [{}] page {} · sketch", item.id, item.page)
        }
        FeedbackKind::Area => writeln!(s, "## [{}] page {} · area", item.id, item.page),
        FeedbackKind::General => writeln!(s, "## [{}] general", item.id),
    };
    if let Some(quote) = item.quote.as_deref().filter(|q| !q.is_empty()) {
        let (quote, c) = if full {
            (quote.to_owned(), false)
        } else {
            shorten(quote, QUOTE_LIMIT)
        };
        cut |= c;
        for line in quote.lines() {
            let _ = writeln!(s, "> {line}");
        }
    }
    if let Some(ctx) = item.context.as_deref().filter(|c| full && !c.is_empty()) {
        let _ = writeln!(s, "Page text around it: \"{ctx}\"");
    }
    if let Some(t) = item.area_text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        let (t, c) = if full {
            (t.to_owned(), false)
        } else {
            shorten(t, AREA_TEXT_LIMIT)
        };
        cut |= c;
        let _ = writeln!(s, "Text in area: \"{t}\"");
    }
    if !item.shapes.is_empty() {
        let _ = writeln!(
            s,
            "Sketch (drawn in red on the screenshot; notes are typed by the user; mm from the page's top-left corner):"
        );
        for shape in &item.shapes {
            let (line, c) = describe_shape(shape, full);
            cut |= c;
            let _ = writeln!(s, "- {line}");
        }
    }
    if let Some([x, y, w, h]) = item.rect.filter(|_| full) {
        let _ = writeln!(
            s,
            "Area (page fractions, top-left): x={x:.3} y={y:.3} w={w:.3} h={h:.3}"
        );
    }
    let comment = if item.comment.is_empty() {
        "(no comment)"
    } else {
        &item.comment
    };
    let _ = writeln!(s, "{comment}");

    let mut blocks = Vec::new();
    if let Some(full_png) = item.image.as_deref() {
        // keep the full-resolution screenshot on disk for clients without image support
        if let Some(file) = save_png(full_png, &item.id, image_dir) {
            let _ = writeln!(s, "Screenshot file: {}", file.display());
        }
        let shown = if full {
            Some(full_png)
        } else {
            item.preview.as_deref().or(Some(full_png))
        };
        cut |= !full && item.preview.is_some();
        let clean = item.clean.as_deref().filter(|_| full);
        if let Some(file) = item
            .clean
            .as_deref()
            .and_then(|png| save_png(png, &format!("{}-clean", item.id), image_dir))
        {
            let _ = writeln!(s, "Without the sketch: {}", file.display());
        }
        cut |= !full && item.clean.is_some();
        blocks.push(text(s));
        blocks.extend(
            shown
                .into_iter()
                .chain(clean)
                .map(|b64| json!({ "type": "image", "data": b64, "mimeType": "image/png" })),
        );
    } else {
        cut |= !full && item.context.is_some();
        blocks.push(text(s));
    }
    (blocks, cut)
}

/// One line per sketch shape, e.g. `arrow from (21, 30) "Figure 3" to (189, 149) "Section 2.1"`.
fn describe_shape(shape: &FeedbackShape, full: bool) -> (String, bool) {
    let mut cut = false;
    let mut quoted = |t: Option<&str>| match t.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) if full => format!(" \"{t}\""),
        Some(t) => {
            let (t, c) = shorten(t, SHAPE_TEXT_LIMIT);
            cut |= c;
            format!(" \"{t}\"")
        }
        None => String::new(),
    };
    let at = |[x, y]: [f64; 2]| format!("({x:.0}, {y:.0})");
    let [x, y, w, h] = shape.box_mm;
    let line = match shape.kind {
        ShapeKind::Arrow => {
            let from = quoted(shape.from_text.as_deref());
            let to = quoted(shape.to_text.as_deref());
            format!(
                "arrow from {}{from} to {}{to}",
                at(shape.from_mm.unwrap_or([x, y])),
                at(shape.to_mm.unwrap_or([x + w, y + h]))
            )
        }
        ShapeKind::Rect => {
            let inside = quoted(shape.text.as_deref());
            let inside = if inside.is_empty() {
                " (empty)".to_owned()
            } else {
                format!(" containing{inside}")
            };
            format!("rectangle {w:.0} × {h:.0} mm at {}{inside}", at([x, y]))
        }
        ShapeKind::Text => {
            let label = quoted(shape.label.as_deref());
            let label = if label.is_empty() {
                " \"\"".to_owned()
            } else {
                label
            };
            format!("note{label} at {}", at([x, y]))
        }
        ShapeKind::Pen => {
            let what = if shape.closed {
                "loop around"
            } else {
                "freehand stroke over"
            };
            let text = quoted(shape.text.as_deref());
            let text = if text.is_empty() {
                " empty space".to_owned()
            } else {
                text
            };
            format!("{what}{text}, {w:.0} × {h:.0} mm at {}", at([x, y]))
        }
    };
    (line, cut)
}

fn save_png(b64: &str, id: &str, dir: &Path) -> Option<std::path::PathBuf> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let file = dir.join(format!("{}.png", sanitize(id)));
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&file, bytes))
        .ok()?;
    Some(file)
}

/// Ids come from the UI; never let them escape the image directory.
fn sanitize(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feedback::item;

    fn status() -> DocStatus {
        DocStatus {
            id: 1,
            path: Some("/d/a.pdf".into()),
            source: Some("/d/a.typ".into()),
            compile_error: None,
            version: 1,
            listeners: 0,
            agents: Vec::new(),
            pending: 0,
            current_page: 2,
            page_count: 5,
        }
    }

    fn joined(blocks: &[Value]) -> String {
        blocks
            .iter()
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn empty_feedback_mentions_page_and_compile_error() {
        let mut st = status();
        st.compile_error = Some("error: boom".into());
        let out = feedback_content(&[], &st, Path::new("/nonexistent"));
        let t = out[0]["text"].as_str().unwrap();
        assert!(t.contains("page 2/5"));
        assert!(t.contains("does NOT compile") && t.contains("error: boom"));
    }

    #[test]
    fn long_texts_are_shortened_by_default_and_complete_on_request() {
        let mut it = item("c1");
        it.kind = FeedbackKind::Text;
        it.page = 3;
        it.quote = Some("word ".repeat(300));
        it.context = Some("before word word after".into());
        let compact = joined(&feedback_content(&[it.clone()], &status(), Path::new("/tmp")));
        assert!(compact.contains("[+"), "quote should be cut");
        assert!(compact.len() < 900, "{}", compact.len());
        assert!(!compact.contains("Page text around it"));
        assert!(compact.contains("get_comments"));

        let full = joined(&details_content(&[it], Path::new("/tmp")));
        assert!(full.matches("word").count() >= 300);
        assert!(full.contains("Page text around it: \"before word word after\""));
    }

    #[test]
    fn area_comments_send_the_preview_unless_details_are_requested() {
        let dir = tempfile::tempdir().unwrap();
        let mut area = item("c2");
        area.kind = FeedbackKind::Area;
        area.rect = Some([0.1, 0.2, 0.3, 0.4]);
        area.area_text = Some("  Hello ".into());
        area.image = Some(base64::engine::general_purpose::STANDARD.encode(b"FULL"));
        area.preview = Some(base64::engine::general_purpose::STANDARD.encode(b"SMALL"));

        let out = feedback_content(&[area.clone()], &status(), dir.path());
        assert!(
            out[0]["text"]
                .as_str()
                .unwrap()
                .starts_with("1 comment(s) on /d/a.typ")
        );
        let image = out.iter().find(|b| b["type"] == "image").unwrap();
        assert_eq!(image["data"], area.preview.clone().unwrap());
        assert!(joined(&out).contains("\"Hello\""));
        assert!(!joined(&out).contains("x=0.100"), "coordinates only on request");
        assert!(dir.path().join("c2.png").exists(), "full image saved to disk");

        let full = details_content(&[area.clone()], dir.path());
        let image = full.iter().find(|b| b["type"] == "image").unwrap();
        assert_eq!(image["data"], area.image.unwrap());
        assert!(joined(&full).contains("x=0.100 y=0.200"));
    }

    #[test]
    fn shortens_at_word_boundaries() {
        assert_eq!(shorten("short", 10), ("short".into(), false));
        let (s, cut) = shorten("alpha beta gamma delta", 12);
        assert!(cut);
        assert_eq!(s, "alpha beta … [+12 chars]");
        let (s, _) = shorten("ääääääääää", 4); // multi-byte safe
        assert!(s.starts_with("ääää"));
    }

    #[test]
    fn compile_errors_are_capped() {
        let note = compile_note(Some(&"x".repeat(5000)));
        assert!(note.len() < 1700);
    }

    #[test]
    fn image_ids_cannot_escape_directory() {
        assert_eq!(sanitize("../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize("c-1_x"), "c-1_x");
    }

    fn shape(kind: ShapeKind) -> FeedbackShape {
        FeedbackShape {
            kind,
            box_mm: [20.0, 30.0, 60.0, 40.0],
            from_mm: None,
            to_mm: None,
            closed: false,
            text: None,
            from_text: None,
            to_text: None,
            label: None,
        }
    }

    #[test]
    fn sketches_are_described_shape_by_shape() {
        let dir = tempfile::tempdir().unwrap();
        let mut it = item("c7");
        it.kind = FeedbackKind::Area;
        it.page = 2;
        it.image = Some(base64::engine::general_purpose::STANDARD.encode(b"SKETCH"));
        it.clean = Some(base64::engine::general_purpose::STANDARD.encode(b"CLEAN"));
        let mut arrow = shape(ShapeKind::Arrow);
        arrow.from_mm = Some([21.0, 30.0]);
        arrow.to_mm = Some([189.0, 149.0]);
        arrow.from_text = Some("Figure 3".into());
        arrow.to_text = Some("Section 2.1".into());
        let mut loop_ = shape(ShapeKind::Pen);
        loop_.closed = true;
        loop_.text = Some("Q3 revenue ".repeat(20));
        let mut note = shape(ShapeKind::Text);
        note.label = Some("bigger".into());
        it.shapes = vec![arrow, shape(ShapeKind::Rect), loop_, note];

        let out = feedback_content(&[it.clone()], &status(), dir.path());
        let t = joined(&out);
        assert!(t.contains("## [c7] page 2 · sketch"), "{t}");
        assert!(
            t.contains(r#"- arrow from (21, 30) "Figure 3" to (189, 149) "Section 2.1""#),
            "{t}"
        );
        assert!(t.contains("- rectangle 60 × 40 mm at (20, 30) (empty)"), "{t}");
        assert!(t.contains("- loop around \"Q3 revenue"), "{t}");
        assert!(t.contains("- note \"bigger\" at (20, 30)"), "{t}");
        assert!(t.contains("[+"), "long shape text is shortened: {t}");
        assert_eq!(out.iter().filter(|b| b["type"] == "image").count(), 1);
        assert!(dir.path().join("c7-clean.png").exists());

        let full = details_content(&[it], dir.path());
        let images: Vec<_> = full.iter().filter(|b| b["type"] == "image").collect();
        assert_eq!(images.len(), 2, "with and without the sketch");
        assert!(!joined(&full).contains("[+"));
    }

    #[test]
    fn sketch_shapes_deserialize_from_the_viewer() {
        let json = r#"{"id":"c1","kind":"area","page":1,"comment":"x","shapes":[
            {"type":"arrow","boxMm":[1,2,3,4],"fromMm":[1,2],"toMm":[4,6],"toText":"Hi"},
            {"type":"pen","boxMm":[0,0,9,9],"closed":true},
            {"type":"text","boxMm":[5,5,20,4],"label":"bigger"}]}"#;
        let it: FeedbackItem = serde_json::from_str(json).unwrap();
        assert_eq!(it.shapes.len(), 3);
        assert_eq!(it.shapes[2].kind, ShapeKind::Text);
        assert_eq!(it.shapes[2].label.as_deref(), Some("bigger"));
        assert_eq!(it.shapes[0].kind, ShapeKind::Arrow);
        assert_eq!(it.shapes[0].to_text.as_deref(), Some("Hi"));
        assert!(it.shapes[1].closed);
    }
}
