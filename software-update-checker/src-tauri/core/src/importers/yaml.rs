//! pnpm-lock.yaml と yarn.lock（Yarn 2 以降）を読むための、YAML のごく一部だけの読み取り。
//!
//! 依存を増やさないため、これらのロックファイルが使う形だけを扱う。
//! - 字下げによる入れ子のマップ（`key:` / `key: value`）
//! - 引用符で囲んだキーと値（`'@types/node@20.0.0':`、`"a@npm:^1.0.0, a@npm:^1.2.0":`）
//! - `- item` の並び（文字列の並びとして読む）
//! - `{...}` や `[...]` の流れ形式は、そのままの文字列として読む（`{}` は空のマップ）
//!
//! アンカー・タグ・複数行の文字列（`|`・`>`）などは使われないので扱わない（読み飛ばす）。

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Scalar(String),
    Map(Vec<(String, Node)>),
    List(Vec<String>),
}

impl Node {
    pub fn get(&self, key: &str) -> Option<&Node> {
        match self {
            Node::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Node::Scalar(s) => Some(s),
            _ => None,
        }
    }

    /// マップの項目。マップでなければ空（`{}` の文字列も空のマップとして扱う）。
    pub fn entries(&self) -> &[(String, Node)] {
        match self {
            Node::Map(entries) => entries,
            _ => &[],
        }
    }
}

struct Line<'a> {
    indent: usize,
    text: &'a str,
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\''))) {
        let inner = &s[1..s.len() - 1];
        // '...' の中の '' は ' 1 つ、"..." の中の \" は " 1 つ
        return if s.starts_with('\'') { inner.replace("''", "'") } else { inner.replace("\\\"", "\"") };
    }
    s.to_string()
}

/// `key: value` / `key:` を分ける。キーは引用符で囲まれていてもよい。
fn split_key(text: &str) -> Option<(String, Option<&str>)> {
    let (key, rest) = if text.starts_with('"') || text.starts_with('\'') {
        let quote = text.chars().next()?;
        // 閉じ引用符を探す（'' と \" は引用符の中の文字）
        let bytes = text.as_bytes();
        let mut i = 1;
        loop {
            if i >= bytes.len() {
                return None;
            }
            if bytes[i] as char == quote {
                if quote == '\'' && bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                if quote == '"' && bytes[i - 1] == b'\\' {
                    i += 1;
                    continue;
                }
                break;
            }
            i += 1;
        }
        (unquote(&text[..=i]), text[i + 1..].trim_start())
    } else {
        let colon = text.find(": ").or_else(|| text.strip_suffix(':').map(|t| t.len()))?;
        (text[..colon].trim().to_string(), &text[colon..])
    };
    let rest = rest.strip_prefix(':')?;
    let value = rest.trim();
    Some((key, (!value.is_empty()).then_some(value)))
}

/// 1 行の値（スカラー）。`{}` は空のマップ。
fn scalar(value: &str) -> Node {
    let v = value.split(" #").next().unwrap_or(value).trim();
    if v == "{}" {
        Node::Map(Vec::new())
    } else {
        Node::Scalar(unquote(v))
    }
}

fn parse_block(lines: &[Line], i: &mut usize, indent: usize) -> Node {
    // 並び（`- item`）
    if lines.get(*i).is_some_and(|l| l.indent == indent && (l.text.starts_with("- ") || l.text == "-")) {
        let mut items = Vec::new();
        while let Some(l) = lines.get(*i).filter(|l| l.indent == indent && l.text.starts_with('-')) {
            items.push(unquote(l.text.trim_start_matches('-')));
            *i += 1;
            // 項目の中の入れ子は使わないので読み飛ばす
            while lines.get(*i).is_some_and(|l| l.indent > indent) {
                *i += 1;
            }
        }
        return Node::List(items);
    }
    let mut entries = Vec::new();
    while let Some(line) = lines.get(*i) {
        if line.indent < indent {
            break;
        }
        if line.indent > indent {
            // 想定外の字下げ（複数行の文字列など）は読み飛ばす
            *i += 1;
            continue;
        }
        *i += 1;
        let Some((key, value)) = split_key(line.text) else { continue };
        let node = match value {
            // 複数行の文字列は使わないので、中身を読み飛ばす
            Some("|") | Some(">") | Some("|-") | Some(">-") => {
                while lines.get(*i).is_some_and(|l| l.indent > indent) {
                    *i += 1;
                }
                Node::Scalar(String::new())
            }
            Some(v) => scalar(v),
            None => match lines.get(*i) {
                Some(next) if next.indent > indent => {
                    let child = next.indent;
                    parse_block(lines, i, child)
                }
                // 並びは親と同じ字下げで書かれることもある（`key:\n- a`）
                Some(next) if next.indent == indent && next.text.starts_with("- ") => parse_block(lines, i, indent),
                _ => Node::Scalar(String::new()),
            },
        };
        entries.push((key, node));
    }
    Node::Map(entries)
}

fn parse_one(text: &str) -> Node {
    let lines: Vec<Line> = text
        .lines()
        .filter_map(|raw| {
            let text = raw.trim_end();
            let trimmed = text.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            Some(Line { indent: text.len() - trimmed.len(), text: trimmed })
        })
        .collect();
    let mut i = 0;
    parse_block(&lines, &mut i, lines.first().map_or(0, |l| l.indent))
}

/// `---` で区切った文書ごとに読む（空の文書は除く）。
pub fn parse_documents(content: &str) -> Vec<Node> {
    let mut docs = Vec::new();
    let mut current = String::new();
    for line in content.lines() {
        if line.trim_end() == "---" {
            docs.push(std::mem::take(&mut current));
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    docs.push(current);
    docs.iter().map(|d| parse_one(d)).filter(|n| !n.entries().is_empty()).collect()
}

/// 文書全体を読む（複数の文書があれば最初のもの）。最上位はマップ。
pub fn parse(content: &str) -> Node {
    parse_documents(content).into_iter().next().unwrap_or(Node::Map(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_nested_maps_quotes_and_lists() {
        let doc = parse(
            "# comment\nlockfileVersion: '9.0'\n\nimporters:\n  .:\n    dependencies:\n      express:\n        specifier: ^4.17.1\n        version: 4.17.1\n\
             packages:\n  '@types/node@20.0.0':\n    resolution: {integrity: sha512-abc}\n    cpu:\n      - x64\n      - arm64\n\
             snapshots:\n  '@types/node@20.0.0': {}\n  \"a@npm:^1.0.0, a@npm:^1.2.0\":\n    version: 1.2.3\n    note: \"it's \\\"quoted\\\"\" # trailing\n",
        );
        assert_eq!(doc.get("lockfileVersion").and_then(Node::as_str), Some("9.0"));
        let express = doc.get("importers").and_then(|n| n.get(".")).and_then(|n| n.get("dependencies")).and_then(|n| n.get("express")).unwrap();
        assert_eq!(express.get("version").and_then(Node::as_str), Some("4.17.1"));
        let pkg = doc.get("packages").and_then(|n| n.get("@types/node@20.0.0")).unwrap();
        assert_eq!(pkg.get("resolution").and_then(Node::as_str), Some("{integrity: sha512-abc}"));
        assert_eq!(pkg.get("cpu"), Some(&Node::List(vec!["x64".into(), "arm64".into()])));
        let snapshots = doc.get("snapshots").unwrap();
        assert_eq!(snapshots.get("@types/node@20.0.0"), Some(&Node::Map(vec![])));
        let a = snapshots.get("a@npm:^1.0.0, a@npm:^1.2.0").unwrap();
        assert_eq!(a.get("version").and_then(Node::as_str), Some("1.2.3"));
        assert_eq!(a.get("note").and_then(Node::as_str), Some("it's \"quoted\""));
    }

    #[test]
    fn splits_documents() {
        let docs = parse_documents("---
a: 1
---
b: 2
");
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[1].get("b").and_then(Node::as_str), Some("2"));
    }

    #[test]
    fn handles_single_quotes_and_block_strings() {
        let doc = parse("'it''s': x\nlong: |\n  line 1\n  line 2\nafter: y\n");
        assert_eq!(doc.get("it's").and_then(Node::as_str), Some("x"));
        assert_eq!(doc.get("after").and_then(Node::as_str), Some("y"));
    }
}
