//! A forgiving XML reader for config files.
//!
//! The editor asks about its own text, which is often half typed: an
//! attribute with no closing quote, a tag that never ends. A strict parser
//! gives up on the whole file then. This one only wants the elements, their
//! attributes and their text, each with its byte range, and keeps going past
//! anything it does not understand.

/// One attribute, with the byte range of its value (inside the quotes).
#[derive(Debug, Clone, PartialEq)]
pub struct Attr<'a> {
    pub name: &'a str,
    pub value: &'a str,
    pub start: usize,
    pub end: usize,
}

/// One start tag.
#[derive(Debug, Clone)]
pub struct Element<'a> {
    pub name: &'a str,
    /// Where `<` is.
    pub start: usize,
    pub attrs: Vec<Attr<'a>>,
    /// The element it sits in, as an index into [`Doc::elements`].
    pub parent: Option<usize>,
}

impl<'a> Element<'a> {
    pub fn attr(&self, name: &str) -> Option<&Attr<'a>> {
        self.attrs.iter().find(|attr| attr.name == name)
    }

    /// The name without a namespace prefix.
    pub fn local(&self) -> &'a str {
        self.name.rsplit(':').next().unwrap_or(self.name)
    }
}

/// Text between tags (or in a CDATA section), trimmed, with its range.
#[derive(Debug, Clone, PartialEq)]
pub struct Text<'a> {
    pub value: &'a str,
    pub start: usize,
    pub end: usize,
    pub parent: Option<usize>,
}

#[derive(Debug, Default)]
pub struct Doc<'a> {
    pub elements: Vec<Element<'a>>,
    pub texts: Vec<Text<'a>>,
}

/// What is at a byte offset: an attribute value or a text node.
pub enum At<'d, 'a> {
    Attr(&'d Element<'a>, &'d Attr<'a>),
    Text(Option<&'d Element<'a>>, &'d Text<'a>),
}

impl<'a> Doc<'a> {
    pub fn parent_of(&self, element: &Element<'a>) -> Option<&Element<'a>> {
        element.parent.and_then(|at| self.elements.get(at))
    }

    /// The attribute value or the text under `byte`, ends included.
    pub fn at(&self, byte: usize) -> Option<At<'_, 'a>> {
        for element in &self.elements {
            for attr in &element.attrs {
                if attr.start <= byte && byte <= attr.end {
                    return Some(At::Attr(element, attr));
                }
            }
        }
        self.texts
            .iter()
            .find(|text| text.start <= byte && byte <= text.end)
            .map(|text| At::Text(text.parent.and_then(|at| self.elements.get(at)), text))
    }
}

fn is_name(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':') || byte >= 0x80
}

/// The range of `text[from..to]` without its surrounding whitespace, if any
/// is left.
fn trimmed(
    text: &str,
    from: usize,
    to: usize,
    parent: Option<usize>,
) -> Option<(usize, usize, Option<usize>)> {
    let raw = text.get(from..to)?;
    let inner = raw.trim();
    if inner.is_empty() {
        return None;
    }
    let start = from + (raw.len() - raw.trim_start().len());
    Some((start, start + inner.len(), parent))
}

/// Reads every element, attribute and piece of text. Never fails.
pub fn parse(text: &str) -> Doc<'_> {
    let bytes = text.as_bytes();
    let mut doc = Doc::default();
    let mut stack: Vec<usize> = Vec::new();
    let mut at = 0;
    let mut text_from = 0;
    let mut pending: Vec<(usize, usize, Option<usize>)> = Vec::new();
    while at < bytes.len() {
        if bytes[at] != b'<' {
            at += 1;
            continue;
        }
        let parent = stack.last().copied();
        pending.extend(trimmed(text, text_from, at, parent));
        let rest = &text[at..];
        if rest.starts_with("<!--") {
            at = rest.find("-->").map_or(bytes.len(), |end| at + end + 3);
            text_from = at;
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            let from = at + 9;
            let end = rest.find("]]>").map_or(bytes.len(), |end| at + end);
            pending.extend(trimmed(text, from, end, parent));
            at = (end + 3).min(bytes.len());
            text_from = at;
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            at = rest.find('>').map_or(bytes.len(), |end| at + end + 1);
            text_from = at;
            continue;
        }
        if rest.starts_with("</") {
            let mut end = at + 2;
            while end < bytes.len() && is_name(bytes[end]) {
                end += 1;
            }
            let name = &text[at + 2..end];
            if let Some(depth) = stack
                .iter()
                .rposition(|open| doc.elements[*open].name == name)
            {
                stack.truncate(depth);
            }
            at = text[end..]
                .find('>')
                .map_or(bytes.len(), |close| end + close + 1);
            text_from = at;
            continue;
        }
        // A start tag.
        let start = at;
        let mut i = at + 1;
        while i < bytes.len() && is_name(bytes[i]) {
            i += 1;
        }
        if i == at + 1 {
            at += 1;
            continue;
        }
        let name = &text[at + 1..i];
        let mut attrs = Vec::new();
        let mut closed = false;
        loop {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'>' => {
                    i += 1;
                    break;
                }
                b'/' => {
                    closed = true;
                    i += 1;
                    continue;
                }
                // A tag that never ended: the next one starts here.
                b'<' => break,
                _ => {}
            }
            closed = false;
            let name_from = i;
            while i < bytes.len() && is_name(bytes[i]) {
                i += 1;
            }
            if i == name_from {
                i += 1;
                continue;
            }
            let attr_name = &text[name_from..i];
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() || bytes[i] != b'=' {
                continue;
            }
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() || !matches!(bytes[i], b'"' | b'\'') {
                continue;
            }
            let quote = bytes[i];
            let value_from = i + 1;
            let mut j = value_from;
            // An unclosed quote ends at the line, so the rest of the file
            // still reads.
            while j < bytes.len() && bytes[j] != quote && bytes[j] != b'\n' {
                j += 1;
            }
            attrs.push(Attr {
                name: attr_name,
                value: &text[value_from..j],
                start: value_from,
                end: j,
            });
            i = if j < bytes.len() && bytes[j] == quote {
                j + 1
            } else {
                j
            };
        }
        doc.elements.push(Element {
            name,
            start,
            attrs,
            parent,
        });
        if !closed {
            stack.push(doc.elements.len() - 1);
        }
        at = i;
        text_from = at;
    }
    let parent = stack.last().copied();
    pending.extend(trimmed(text, text_from, bytes.len(), parent));
    for (start, end, parent) in pending {
        doc.texts.push(Text {
            value: &text[start..end],
            start,
            end,
            parent,
        });
    }
    doc
}

/// Line (from 1) and column (UTF-16, from 1) of a byte offset.
pub fn line_col(text: &str, byte: usize) -> (u32, u32) {
    let byte = byte.min(text.len());
    let before = &text[..byte];
    let line = before.matches('\n').count() as u32 + 1;
    let line_start = before.rfind('\n').map_or(0, |at| at + 1);
    let column = text[line_start..byte].encode_utf16().count() as u32 + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_attributes_and_text_with_ranges() {
        let source = r#"<?xml version="1.0"?>
<config>
    <!-- <type name="Commented\Out"/> -->
    <type name="Foo\Bar">
        <arguments>
            <argument name="x" xsi:type="object">Baz\Qux</argument>
        </arguments>
    </type>
</config>"#;
        let doc = parse(source);
        let names: Vec<&str> = doc.elements.iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["config", "type", "arguments", "argument"]);
        let type_attr = doc.elements[1].attr("name").unwrap();
        assert_eq!(&source[type_attr.start..type_attr.end], "Foo\\Bar");
        let text = doc.texts.iter().find(|t| t.value == "Baz\\Qux").unwrap();
        assert_eq!(doc.elements[text.parent.unwrap()].name, "argument");
        assert_eq!(doc.elements[3].attr("xsi:type").unwrap().value, "object");
    }

    #[test]
    fn half_typed_markup_still_reads() {
        let source = "<config>\n<type name=\"Foo\\Ba\n<plugin type=\"X\\Y\"/>";
        let doc = parse(source);
        assert!(doc
            .elements
            .iter()
            .any(|e| e.name == "plugin" && e.attr("type").unwrap().value == "X\\Y"));
    }

    #[test]
    fn line_and_column_count_from_one() {
        assert_eq!(line_col("ab\ncd", 4), (2, 2));
    }
}
