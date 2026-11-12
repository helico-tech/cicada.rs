//! Turns README.md, the guides in docs/guide and docs/operations.md into rustdoc markdown for their doctests (see
//! `ReadmeDoctests` in src/lib.rs). A Rust block may be preceded by a hidden setup comment that GitHub does not render:
//!
//! ````text
//! <!-- doctest
//! # use cicada::prelude::*;
//! # fn example(turn: &mut Turn<MyActor>) -> HandlerResult {
//! @@2
//! # Ok(())
//! # }
//! -->
//! ```rust
//! turn.send_self(MyMsg::Run)?;
//! turn.stop();
//! ```
//! ````
//!
//! The comment's lines are the doctest; each `@@N` stands for the next N lines of the visible block. The build fails
//! when the counts do not add up to the visible block, so the two cannot drift apart.

use std::path::Path;
use std::{env, fs};

fn main() {
    let root = Path::new(&env::var("CARGO_MANIFEST_DIR").expect("set by cargo")).join("..");
    let out = Path::new(&env::var("OUT_DIR").expect("set by cargo")).to_path_buf();
    for (source, target) in [
        ("README.md", "README.md"),
        ("docs/guide/core.md", "guide-core.md"),
        ("docs/guide/http.md", "guide-http.md"),
        ("docs/guide/app.md", "guide-app.md"),
        ("docs/guide/actors.md", "guide-actors.md"),
        ("docs/operations.md", "operations.md"),
    ] {
        let path = root.join(source);
        println!("cargo:rerun-if-changed={}", path.display());
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{source}: {e}"));
        let doc = expand(&text).unwrap_or_else(|e| panic!("{source}: {e}"));
        fs::write(out.join(target), doc).expect("write the doctest markdown");
    }
}

fn expand(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!-- doctest\n") {
        out.push_str(&rest[..start]);
        let after = &rest[start + "<!-- doctest\n".len()..];
        let end = after.find("\n-->\n").ok_or("an unclosed <!-- doctest comment")?;
        let template: Vec<&str> = after[..end].lines().collect();
        let block = &after[end + "\n-->\n".len()..];
        let fence_end = block.find('\n').ok_or("a doctest comment without a code block")?;
        let fence = &block[..fence_end];
        if !fence.starts_with("```rust") {
            return Err(format!("a doctest comment must precede a ```rust block, not {fence}"));
        }
        let body_and_rest = &block[fence_end + 1..];
        let close = body_and_rest.find("```").ok_or("an unclosed code block")?;
        let mut visible = body_and_rest[..close].lines();
        out.push_str(fence);
        out.push('\n');
        for line in template {
            if let Some(count) = line.strip_prefix("@@") {
                let count: usize = count.parse().map_err(|_| format!("a bad marker {line}"))?;
                for _ in 0..count {
                    let l =
                        visible.next().ok_or(format!("@@ markers ask for more lines than the block has: {fence}"))?;
                    out.push_str(l);
                    out.push('\n');
                }
            } else {
                out.push_str(line);
                out.push('\n');
            }
        }
        if visible.next().is_some() {
            return Err(format!(
                "the block has more lines than its @@ markers place, near:\n{}",
                template_preview(after)
            ));
        }
        out.push_str("```");
        rest = &body_and_rest[close + 3..];
    }
    out.push_str(rest);
    Ok(out)
}

fn template_preview(text: &str) -> String {
    text.lines().take(5).collect::<Vec<_>>().join("\n")
}
