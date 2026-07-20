// cs_assembly/xml_docs.rs — XML ドキュメントコメントの解析関数。

use {
    std::collections::HashMap, std::path::Path,
};


// ---------------------------------------------------------------------------
// XML documentation helpers
// ---------------------------------------------------------------------------

/// Strip XML tags from a string and normalize whitespace.
pub(crate) fn strip_xml_tags(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}


/// Extract the text content of the first occurrence of `<tag>...</tag>` in `text`.
pub(crate) fn extract_xml_element(text: &str, tag: &str) -> String {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    if let Some(start) = text.find(&open) {
        if let Some(gt) = text[start..].find('>') {
            let content_start = start + gt + 1;
            if let Some(end) = text[content_start..].find(&close) {
                return strip_xml_tags(&text[content_start..content_start + end]);
            }
        }
    }
    String::new()
}


/// Convert an XML doc member ID to a simplified lookup key.
/// - `T:Namespace.ClassName`  → `T:ClassName`
/// - `M:Namespace.Class.Method(args)` → `M:ClassName.Method`
/// - `P:Namespace.Class.Prop`  → `P:ClassName.Prop`
pub(crate) fn simplify_member_id(member_id: &str) -> Option<String> {
    if member_id.len() < 2 { return None; }
    let prefix = &member_id[..2];
    let rest   = &member_id[2..];
    match prefix {
        "T:" => {
            let simple = rest.rsplit('.').next().unwrap_or(rest);
            let simple = simple.split('`').next().unwrap_or(simple);
            Some(format!("T:{simple}"))
        }
        "M:" => {
            let without_args = rest.split('(').next().unwrap_or(rest);
            let parts: Vec<&str> = without_args.split('.').collect();
            if parts.len() >= 2 {
                let cls = parts[parts.len() - 2].split('`').next().unwrap_or(parts[parts.len() - 2]);
                let method = parts[parts.len() - 1];
                Some(format!("M:{cls}.{method}"))
            } else { None }
        }
        "P:" => {
            let parts: Vec<&str> = rest.split('.').collect();
            if parts.len() >= 2 {
                let cls = parts[parts.len() - 2].split('`').next().unwrap_or(parts[parts.len() - 2]);
                let prop = parts[parts.len() - 1];
                Some(format!("P:{cls}.{prop}"))
            } else { None }
        }
        _ => None,
    }
}


/// Parse an XML documentation file (companion to a .NET DLL) and return a map
/// of simplified member key → summary text.
/// Returns an empty map if the file does not exist or cannot be read.
pub(crate) fn parse_xml_docs(xml_path: &Path) -> HashMap<String, String> {
    let content = match std::fs::read_to_string(xml_path) {
        Ok(c)  => c,
        Err(_) => return HashMap::new(),
    };
    let mut docs: HashMap<String, String> = HashMap::new();
    let mut search = 0usize;
    while let Some(rel) = content[search..].find("<member ") {
        let m_start = search + rel;
        let after   = m_start + 8;
        let name_val_start = match content[after..].find("name=\"").map(|p| after + p + 6) {
            Some(n) => n,
            None    => { search = m_start + 1; continue; }
        };
        let name_end = match content[name_val_start..].find('"').map(|p| name_val_start + p) {
            Some(e) => e,
            None    => { search = m_start + 1; continue; }
        };
        let member_id = &content[name_val_start..name_end];
        let block_end = content[name_end..].find("</member>")
            .map(|p| name_end + p)
            .unwrap_or(content.len());
        let block   = &content[name_end..block_end];
        let summary = extract_xml_element(block, "summary");
        if !summary.is_empty() {
            if let Some(key) = simplify_member_id(member_id) {
                docs.entry(key).or_insert(summary);
            }
        }
        search = if block_end < content.len() { block_end + 9 } else { content.len() };
    }
    docs
}
