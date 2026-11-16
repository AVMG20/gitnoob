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

/// How many open elements up an end tag is matched against.
const CLOSE_REACH: usize = 256;

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
        let rest = &text[at..];
        // A `<` that starts nothing (`a < b` in text) is text itself: the
        // run it is in goes on to the next real tag.
        let markup = rest.len() > 1
            && (is_name(bytes[at + 1]) || matches!(bytes[at + 1], b'!' | b'?' | b'/'));
        if !markup {
            at += 1;
            continue;
        }
        pending.extend(trimmed(text, text_from, at, parent));
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
            // Only so far up: a file of tags that never close and end tags
            // that match nothing would otherwise walk the whole stack each time.
            let floor = stack.len().saturating_sub(CLOSE_REACH);
            if let Some(depth) = stack[floor..]
                .iter()
                .rposition(|open| doc.elements[*open].name == name)
            {
                stack.truncate(floor + depth);
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

/// Where the lines of a text start, so many offsets in one file cost one
/// pass over it and a binary search each, not a pass each. A line that is
/// all ASCII has its column by subtraction; a long one that is not (a
/// minified file with an accent in it) is counted on from the last offset
/// asked on it, so offsets asked in order cost one pass over the line.
pub struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
    ascii: Vec<bool>,
    /// The last offset asked on a line that is not ASCII, and its column.
    last: std::cell::Cell<(usize, usize, u32)>,
}

impl<'a> Lines<'a> {
    pub fn new(text: &'a str) -> Self {
        let mut starts = vec![0];
        let mut ascii = Vec::new();
        let mut plain = true;
        for (at, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(at + 1);
                ascii.push(plain);
                plain = true;
            } else if byte >= 0x80 {
                plain = false;
            }
        }
        ascii.push(plain);
        Lines {
            text,
            starts,
            ascii,
            last: std::cell::Cell::new((usize::MAX, 0, 1)),
        }
    }

    /// Line (from 1) and column (UTF-16, from 1) of a byte offset.
    pub fn line_col(&self, byte: usize) -> (u32, u32) {
        let mut byte = byte.min(self.text.len());
        while !self.text.is_char_boundary(byte) {
            byte -= 1;
        }
        let line = self.starts.partition_point(|start| *start <= byte) - 1;
        let start = self.starts[line];
        if self.ascii[line] {
            return (line as u32 + 1, (byte - start) as u32 + 1);
        }
        let (last_line, last_byte, last_column) = self.last.get();
        let (from, column) = if last_line == line && last_byte <= byte {
            (last_byte, last_column)
        } else {
            (start, 1)
        };
        let column = column + self.text[from..byte].encode_utf16().count() as u32;
        self.last.set((line, byte, column));
        (line as u32 + 1, column)
    }
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
        let text = "ab\ncd\n\nxé y";
        let lines = Lines::new(text);
        for at in [0, 1, 2, 3, 4, 5, 6, 7, 8, 11, text.len()] {
            assert_eq!(lines.line_col(at), line_col(text, at), "at {at}");
        }
    }

    #[test]
    fn a_stray_angle_bracket_is_text_read_once() {
        let doc = parse("<a>1 < 2 < 3</a><b>x</b>");
        let texts: Vec<&str> = doc.texts.iter().map(|t| t.value).collect();
        assert_eq!(texts, vec!["1 < 2 < 3", "x"]);
    }

    /// A 5 MB layout file reads in one pass: no quadratic walk over the
    /// nesting, the text or the line numbers.
    #[test]
    fn a_huge_file_reads_in_linear_time() {
        let mut source = String::from("<page>\n");
        while source.len() < 5 * 1024 * 1024 {
            source.push_str(
                "  <block class=\"Foo\\Bar\\Baz\" name=\"a.b\" template=\"A_B::c.phtml\">1 < 2</block>\n",
            );
        }
        // Tags that never close with end tags that match nothing, all on
        // one line, with an accent in it so columns must be counted.
        for _ in 0..100_000 {
            source.push_str("<a t=\"é\"></b>");
        }
        // Never closed, and a comment that never ends.
        source.push_str("<container name=\"open\"><!-- no end");
        let started = std::time::Instant::now();
        let doc = parse(&source);
        let lines = Lines::new(&source);
        let mut last = 0;
        for element in &doc.elements {
            last = lines.line_col(element.start).0;
        }
        assert!(doc.elements.len() > 50_000 && last > 50_000);
        let tail = doc.elements[doc.elements.len() - 2].start;
        assert_eq!(lines.line_col(tail), line_col(&source, tail));
        // Asked out of order, it still counts right.
        let early = doc.elements[doc.elements.len() - 50_000].start;
        assert_eq!(lines.line_col(early), line_col(&source, early));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }
}
