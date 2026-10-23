//! The sectioned text files (KING.TXT, BLURB*.TXT, HELP.TXT, ERROR.TXT, PRODUCE.TXT) plus
//! STORY.TXT and CREDITS.TXT.
//!
//! Sectioned files start with a 528-byte hash index which we ignore; the body consists of
//! `*NAME` headers followed by text lines until the next `*` line; `*END` terminates. Lines
//! ending in `^` are hard line breaks; lines starting with `_` are menu choices; `$RPLC1`,
//! `$RPLC2`, `$US`, `$THEM`, `$BUCKS` are substitution tokens.

use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct TextFile {
    sections: Vec<(String, Vec<String>)>,
    by_name: HashMap<String, usize>,
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// Split bytes into lines (CRLF or LF), decoding as Latin-1 and dropping a trailing ^Z.
pub fn lines_of(bytes: &[u8]) -> Vec<String> {
    let text = latin1(bytes);
    text.split('\n')
        .map(|l| l.trim_end_matches(['\r', '\u{1a}']).to_string())
        .collect()
}

impl TextFile {
    pub fn parse(bytes: &[u8]) -> TextFile {
        let lines = lines_of(bytes);
        let mut tf = TextFile::default();
        let mut cur: Option<(String, Vec<String>)> = None;
        for raw in lines {
            if let Some(rest) = raw.strip_prefix('*') {
                if let Some(sec) = cur.take() {
                    tf.push(sec);
                }
                let name = rest.trim().to_string();
                if name.eq_ignore_ascii_case("END") {
                    break;
                }
                cur = Some((name, Vec::new()));
            } else if let Some(sec) = cur.as_mut() {
                sec.1.push(raw);
            }
        }
        if let Some(sec) = cur.take() {
            tf.push(sec);
        }
        // trim trailing blank lines of each section
        for (_, l) in tf.sections.iter_mut() {
            while l.last().map(|s| s.trim().is_empty()).unwrap_or(false) {
                l.pop();
            }
            while l.first().map(|s| s.trim().is_empty()).unwrap_or(false) {
                l.remove(0);
            }
        }
        tf
    }

    fn push(&mut self, sec: (String, Vec<String>)) {
        let key = sec.0.to_ascii_uppercase();
        // first definition wins (duplicates exist, e.g. HORSEBACK RIDING)
        self.by_name.entry(key).or_insert(self.sections.len());
        self.sections.push(sec);
    }

    /// Raw lines of a section (case-insensitive name).
    pub fn section(&self, name: &str) -> Option<&[String]> {
        self.by_name
            .get(&name.to_ascii_uppercase())
            .map(|&i| self.sections[i].1.as_slice())
    }

    pub fn has(&self, name: &str) -> bool {
        self.by_name.contains_key(&name.to_ascii_uppercase())
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.sections.iter().map(|(n, _)| n.as_str())
    }

    pub fn len(&self) -> usize {
        self.sections.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }
}

/// Values substituted for the `$` tokens.
#[derive(Clone, Debug, Default)]
pub struct Vars {
    pub rplc1: String,
    pub rplc2: String,
    pub us: String,
    pub them: String,
    pub bucks: i32,
}

pub fn substitute_line(line: &str, v: &Vars) -> String {
    line.replace("$RPLC1", &v.rplc1)
        .replace("$RPLC2", &v.rplc2)
        .replace("$BUCKS", &v.bucks.to_string())
        .replace("$THEM", &v.them)
        .replace("$US", &v.us)
}

pub fn substitute(lines: &[String], v: &Vars) -> Vec<String> {
    lines.iter().map(|l| substitute_line(l, v)).collect()
}

/// A dialog text split into body paragraphs and `_` menu choices.
#[derive(Clone, Debug, Default)]
pub struct MenuText {
    /// Display lines of the body: hard breaks (`^`) and blank lines preserved as separate
    /// entries; consecutive soft lines are joined with a space.
    pub body: Vec<String>,
    pub choices: Vec<String>,
}

/// Convert raw section lines into display lines: lines ending with `^` end a paragraph;
/// blank lines produce an empty entry; other consecutive lines are joined.
pub fn paragraphs(lines: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut acc = String::new();
    let mut open = false;
    for l in lines {
        let t = l.trim_end();
        if t.is_empty() {
            if open {
                out.push(std::mem::take(&mut acc));
                open = false;
            }
            out.push(String::new());
            continue;
        }
        if let Some(stripped) = t.strip_suffix('^') {
            if open && !acc.is_empty() {
                acc.push(' ');
            }
            acc.push_str(stripped.trim_end());
            out.push(std::mem::take(&mut acc));
            open = false;
        } else {
            if open && !acc.is_empty() {
                acc.push(' ');
            }
            acc.push_str(t);
            open = true;
        }
    }
    if open {
        out.push(acc);
    }
    // drop trailing blanks
    while out.last().map(|s| s.is_empty()).unwrap_or(false) {
        out.pop();
    }
    out
}

pub fn split_menu(lines: &[String]) -> MenuText {
    let mut body_lines = Vec::new();
    let mut choices = Vec::new();
    for l in lines {
        if let Some(c) = l.strip_prefix('_') {
            choices.push(c.trim_end_matches('^').trim().to_string());
        } else {
            body_lines.push(l.clone());
        }
    }
    MenuText { body: paragraphs(&body_lines), choices }
}

/// STORY.TXT: screens separated by lines consisting of `_`.
pub fn parse_story(bytes: &[u8]) -> Vec<Vec<String>> {
    let mut screens = Vec::new();
    let mut cur = Vec::new();
    for l in lines_of(bytes) {
        let t = l.trim_end();
        if t.starts_with('_') {
            if !cur.is_empty() {
                screens.push(std::mem::take(&mut cur));
            }
        } else if !t.is_empty() || !cur.is_empty() {
            cur.push(t.to_string());
        }
    }
    if cur.iter().any(|s| !s.trim().is_empty()) {
        screens.push(cur);
    }
    for s in screens.iter_mut() {
        while s.last().map(|x| x.trim().is_empty()).unwrap_or(false) {
            s.pop();
        }
    }
    screens
}

/// CREDITS.TXT: one line per credit, read in file order by the intro (`fscanf("%[^\n]\n")`,
/// which also skips blank lines). The text is kept as written: the intro font (FONT_THIN) has
/// its full stop at `%` and a different glyph at `.`, so "JEFFERY L% BRIGGS" is drawn as is.
pub fn parse_credits(bytes: &[u8]) -> Vec<String> {
    lines_of(bytes).into_iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sections() {
        let src = b"garbage\r\n*FOO\r\nline one^\r\nline two\r\nline three\r\n\r\n_Choice A^\r\n_Choice B\r\n*END\r\n";
        let tf = TextFile::parse(src);
        assert_eq!(tf.len(), 1);
        let s = tf.section("foo").unwrap();
        let m = split_menu(s);
        assert_eq!(m.choices, vec!["Choice A", "Choice B"]);
        assert_eq!(m.body, vec!["line one", "line two line three"]);
    }

    #[test]
    fn subst() {
        let v = Vars { rplc1: "Rome".into(), bucks: 50, ..Default::default() };
        assert_eq!(substitute_line("Give $BUCKS$ to $RPLC1", &v), "Give 50$ to Rome");
    }
}
