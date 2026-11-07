//! Package URL (purl) の最小限のパーサー。
//! 仕様: https://github.com/package-url/purl-spec
//!
//! このツールで必要な type / namespace / name / version だけを扱い、
//! qualifiers と subpath は読み飛ばす。

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Purl {
    pub purl_type: String,
    pub namespace: Option<String>,
    pub name: String,
    pub version: Option<String>,
}

impl Purl {
    pub fn parse(input: &str) -> Option<Self> {
        let rest = input.trim().strip_prefix("pkg:")?;
        let rest = rest.split('#').next()?;
        let rest = rest.split('?').next()?;
        let rest = rest.trim_start_matches('/');

        let (purl_type, path) = rest.split_once('/')?;
        // バージョン区切りの '@' は最後の '/' より後にあるものだけ。
        // （npm の "@scope" を誤ってバージョンとして扱わないため）
        let last_slash = path.rfind('/').map(|i| i + 1).unwrap_or(0);
        let (path, version) = match path[last_slash..].rfind('@') {
            Some(at) => {
                let at = last_slash + at;
                (&path[..at], Some(percent_decode(&path[at + 1..])))
            }
            None => (path, None),
        };

        let segments: Vec<String> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(percent_decode)
            .collect();
        let (name, namespace) = segments.split_last()?;
        if name.is_empty() {
            return None;
        }
        Some(Self {
            purl_type: purl_type.to_ascii_lowercase(),
            namespace: if namespace.is_empty() {
                None
            } else {
                Some(namespace.join("/"))
            },
            name: name.clone(),
            version: version.filter(|v| !v.is_empty()),
        })
    }

    /// qualifiers なしの正規形。`with_version` が false ならバージョンも付けない。
    pub fn to_string_canonical(&self, with_version: bool) -> String {
        let mut s = format!("pkg:{}/", self.purl_type);
        if let Some(ns) = &self.namespace {
            for seg in ns.split('/') {
                s.push_str(&percent_encode(seg));
                s.push('/');
            }
        }
        s.push_str(&percent_encode(&self.name));
        if with_version {
            if let Some(v) = &self.version {
                s.push('@');
                s.push_str(&percent_encode(v));
            }
        }
        s
    }

    pub fn build(purl_type: &str, namespace: Option<&str>, name: &str, version: Option<&str>) -> Self {
        Self {
            purl_type: purl_type.to_string(),
            namespace: namespace.map(str::to_string),
            name: name.to_string(),
            version: version.map(str::to_string),
        }
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(b) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' | b'~' | b'+' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_maven_with_qualifiers() {
        let p = Purl::parse("pkg:maven/org.apache.tomcat.embed/tomcat-embed-core@10.1.20?type=jar").unwrap();
        assert_eq!(p.purl_type, "maven");
        assert_eq!(p.namespace.as_deref(), Some("org.apache.tomcat.embed"));
        assert_eq!(p.name, "tomcat-embed-core");
        assert_eq!(p.version.as_deref(), Some("10.1.20"));
    }

    #[test]
    fn parses_npm_scope_with_and_without_version() {
        let p = Purl::parse("pkg:npm/%40angular/core@16.0.0").unwrap();
        assert_eq!(p.namespace.as_deref(), Some("@angular"));
        assert_eq!(p.name, "core");
        assert_eq!(p.version.as_deref(), Some("16.0.0"));

        let p = Purl::parse("pkg:npm/@angular/core").unwrap();
        assert_eq!(p.namespace.as_deref(), Some("@angular"));
        assert_eq!(p.version, None);
    }

    #[test]
    fn canonical_form_drops_qualifiers() {
        let p = Purl::parse("pkg:maven/g.h/a-b@1.0?type=jar#sub").unwrap();
        assert_eq!(p.to_string_canonical(true), "pkg:maven/g.h/a-b@1.0");
        assert_eq!(p.to_string_canonical(false), "pkg:maven/g.h/a-b");
        let n = Purl::parse("pkg:npm/@angular/core@1.0.0").unwrap();
        assert_eq!(n.to_string_canonical(true), "pkg:npm/%40angular/core@1.0.0");
    }

    #[test]
    fn rejects_garbage() {
        assert!(Purl::parse("maven/g/a").is_none());
        assert!(Purl::parse("pkg:maven").is_none());
    }
}
