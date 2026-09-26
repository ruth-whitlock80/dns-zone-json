// BIND-style zone file parsing and writing.
//
// This is deliberately a subset of what BIND accepts: parenthesized
// multi-line records are supported, but $INCLUDE is not. That's a real
// gap (see the README) but most hand-written and generated zone files
// don't use it, and the awkward parts that remain -- owner name
// inheritance, $ORIGIN, TXT quoting, line continuations -- are exactly
// what trips up a naive line-splitter, so that's what the tests below
// focus on.

use crate::record::{RData, Record};
use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

#[derive(Debug)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

pub fn parse_zone(input: &str, default_origin: &str) -> Result<Vec<Record>, ParseError> {
    let mut origin = normalize_origin(default_origin);
    let mut ttl: Option<u32> = None;
    let mut last_name: Option<String> = None;
    let mut records = Vec::new();

    for logical in join_continuations(input)? {
        let tokens = tokenize(&logical.text).map_err(|e| ParseError { line: logical.line, message: e })?;
        if tokens.is_empty() {
            continue;
        }

        if tokens[0].eq_ignore_ascii_case("$ORIGIN") {
            let value = tokens.get(1).ok_or_else(|| ParseError {
                line: logical.line,
                message: "$ORIGIN needs an argument".to_string(),
            })?;
            origin = normalize_origin(value);
            continue;
        }
        if tokens[0].eq_ignore_ascii_case("$TTL") {
            let value = tokens.get(1).ok_or_else(|| ParseError {
                line: logical.line,
                message: "$TTL needs an argument".to_string(),
            })?;
            let parsed: u32 = value.parse().map_err(|_| ParseError {
                line: logical.line,
                message: format!("invalid $TTL value \"{}\"", value),
            })?;
            ttl = Some(parsed);
            continue;
        }

        let record = parse_record_line(&tokens, logical.starts_with_space, &origin, ttl, &mut last_name)
            .map_err(|e| ParseError { line: logical.line, message: e })?;
        records.push(record);
    }

    Ok(records)
}

// One line's worth of record, after joining any parenthesized continuation
// lines it spans. `line` is the first physical line of the group, used for
// error messages. `starts_with_space` reflects that first physical line,
// since that's what decides whether the owner name is inherited.
struct LogicalLine {
    line: usize,
    text: String,
    starts_with_space: bool,
}

// Joins parenthesized continuations into single logical lines, stripping
// comments as it goes. A `(` opens a continuation and a `)` closes it;
// while a continuation is open, line breaks are treated as whitespace.
// Parens and semicolons inside quoted strings don't count, matching how
// tokenize() treats them within a single line.
fn join_continuations(input: &str) -> Result<Vec<LogicalLine>, ParseError> {
    let mut result = Vec::new();
    let mut depth: i32 = 0;
    let mut current = String::new();
    let mut start_line: Option<usize> = None;
    let mut starts_with_space = false;
    let total_lines = input.lines().count();

    for (i, raw_line) in input.lines().enumerate() {
        let line_no = i + 1;
        let content = strip_comment_track_parens(raw_line, &mut depth)
            .map_err(|e| ParseError { line: line_no, message: e })?;
        let trimmed = content.trim();

        if start_line.is_none() {
            if trimmed.is_empty() {
                continue;
            }
            start_line = Some(line_no);
            starts_with_space = raw_line.starts_with(' ') || raw_line.starts_with('\t');
            current = trimmed.to_string();
        } else if !trimmed.is_empty() {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(trimmed);
        }

        if depth == 0 {
            result.push(LogicalLine {
                line: start_line.take().unwrap(),
                text: std::mem::take(&mut current),
                starts_with_space,
            });
        }
    }

    if depth != 0 {
        return Err(ParseError {
            line: start_line.unwrap_or(total_lines),
            message: "unterminated parenthesized record".to_string(),
        });
    }

    Ok(result)
}

// Strips a `;` comment (unless inside quotes) and replaces unquoted `(`/`)`
// with spaces while tracking nesting depth in `depth`. Quoted strings must
// still close on the same physical line they open on.
fn strip_comment_track_parens(line: &str, depth: &mut i32) -> Result<String, String> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;
    let n = chars.len();
    let mut in_quote = false;

    while i < n {
        let c = chars[i];
        if in_quote {
            if c == '\\' && i + 1 < n {
                out.push(c);
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_quote = false;
            }
            out.push(c);
            i += 1;
            continue;
        }

        match c {
            '"' => {
                in_quote = true;
                out.push(c);
                i += 1;
            }
            ';' => break,
            '(' => {
                *depth += 1;
                out.push(' ');
                i += 1;
            }
            ')' => {
                *depth -= 1;
                if *depth < 0 {
                    return Err("unmatched ')'".to_string());
                }
                out.push(' ');
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }

    if in_quote {
        return Err("unterminated quoted string".to_string());
    }

    Ok(out)
}

pub fn write_zone(records: &[Record]) -> String {
    let mut out = String::new();
    for record in records {
        out.push_str(&record.name);
        out.push('\t');
        out.push_str(&record.ttl.to_string());
        out.push_str("\tIN\t");
        out.push_str(record.rdata.type_name());
        out.push('\t');
        out.push_str(&format_rdata(&record.rdata));
        out.push('\n');
    }
    out
}

fn parse_record_line(
    tokens: &[String],
    starts_with_space: bool,
    origin: &str,
    default_ttl: Option<u32>,
    last_name: &mut Option<String>,
) -> Result<Record, String> {
    let mut idx = 0;

    let owner = if starts_with_space {
        last_name
            .clone()
            .ok_or_else(|| "record has no owner name and none to inherit".to_string())?
    } else {
        let raw = tokens.get(idx).ok_or("empty record line")?;
        idx += 1;
        qualify(raw, origin)
    };
    *last_name = Some(owner.clone());

    let mut ttl_value: Option<u32> = None;
    if let Some(tok) = tokens.get(idx) {
        if let Ok(n) = tok.parse::<u32>() {
            ttl_value = Some(n);
            idx += 1;
        }
    }

    if let Some(tok) = tokens.get(idx) {
        if tok.eq_ignore_ascii_case("IN") {
            idx += 1;
        }
    }

    let type_tok = tokens.get(idx).ok_or("record is missing a type field")?;
    idx += 1;
    let type_name = type_tok.to_uppercase();

    let rdata = parse_rdata(&type_name, &tokens[idx..], origin)?;

    let ttl = ttl_value
        .or(default_ttl)
        .ok_or_else(|| "record has no TTL and no $TTL directive has set a default".to_string())?;

    Ok(Record { name: owner, ttl, rdata })
}

fn parse_rdata(type_name: &str, tokens: &[String], origin: &str) -> Result<RData, String> {
    match type_name {
        "A" => {
            let raw = tokens.get(0).ok_or("A record needs an address")?;
            let addr = raw
                .parse::<Ipv4Addr>()
                .map_err(|e| format!("bad A address \"{}\": {}", raw, e))?;
            Ok(RData::A(addr))
        }
        "AAAA" => {
            let raw = tokens.get(0).ok_or("AAAA record needs an address")?;
            let addr = raw
                .parse::<Ipv6Addr>()
                .map_err(|e| format!("bad AAAA address \"{}\": {}", raw, e))?;
            Ok(RData::Aaaa(addr))
        }
        "CNAME" => {
            let raw = tokens.get(0).ok_or("CNAME record needs a target")?;
            Ok(RData::Cname(qualify(raw, origin)))
        }
        "NS" => {
            let raw = tokens.get(0).ok_or("NS record needs a target")?;
            Ok(RData::Ns(qualify(raw, origin)))
        }
        "MX" => {
            let pref_raw = tokens.get(0).ok_or("MX record needs a preference and exchange")?;
            let exchange_raw = tokens.get(1).ok_or("MX record needs an exchange host")?;
            let preference: u16 = pref_raw
                .parse()
                .map_err(|_| format!("bad MX preference \"{}\"", pref_raw))?;
            Ok(RData::Mx {
                preference,
                exchange: qualify(exchange_raw, origin),
            })
        }
        "TXT" => {
            if tokens.is_empty() {
                return Err("TXT record needs at least one string".to_string());
            }
            Ok(RData::Txt(tokens.to_vec()))
        }
        other => Err(format!("unsupported record type \"{}\"", other)),
    }
}

fn format_rdata(rdata: &RData) -> String {
    match rdata {
        RData::A(addr) => addr.to_string(),
        RData::Aaaa(addr) => addr.to_string(),
        RData::Cname(target) => target.clone(),
        RData::Ns(target) => target.clone(),
        RData::Mx { preference, exchange } => format!("{} {}", preference, exchange),
        RData::Txt(segments) => segments.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" "),
    }
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

// "@" means "the current origin"; a trailing dot means "already absolute";
// anything else is relative and gets the origin appended.
fn qualify(name: &str, origin: &str) -> String {
    if name == "@" {
        return origin.to_string();
    }
    if name.ends_with('.') {
        return name.to_string();
    }
    if origin == "." {
        format!("{}.", name)
    } else {
        format!("{}.{}", name, origin)
    }
}

fn normalize_origin(raw: &str) -> String {
    if raw.ends_with('.') {
        raw.to_string()
    } else {
        format!("{}.", raw)
    }
}

// Splits a line into whitespace-separated fields, treating a double-quoted
// span as one field (escapes \" and \\ honored) and stopping at the first
// unquoted ';', which starts a comment that runs to the end of the line.
fn tokenize(line: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let n = chars.len();

    while i < n {
        let c = chars[i];
        if c == ' ' || c == '\t' {
            i += 1;
            continue;
        }
        if c == ';' {
            break;
        }
        if c == '"' {
            let mut s = String::new();
            i += 1;
            let mut closed = false;
            while i < n {
                let c2 = chars[i];
                if c2 == '\\' && i + 1 < n {
                    let next = chars[i + 1];
                    match next {
                        '"' => {
                            s.push('"');
                            i += 2;
                        }
                        '\\' => {
                            s.push('\\');
                            i += 2;
                        }
                        other => {
                            s.push('\\');
                            s.push(other);
                            i += 2;
                        }
                    }
                    continue;
                }
                if c2 == '"' {
                    closed = true;
                    i += 1;
                    break;
                }
                s.push(c2);
                i += 1;
            }
            if !closed {
                return Err("unterminated quoted string".to_string());
            }
            tokens.push(s);
            continue;
        }

        let start = i;
        while i < n && chars[i] != ' ' && chars[i] != '\t' && chars[i] != ';' {
            i += 1;
        }
        tokens.push(chars[start..i].iter().collect());
    }

    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    enum Expect {
        Records(Vec<Record>),
        Err,
    }

    struct Case {
        name: &'static str,
        origin: &'static str,
        input: &'static str,
        expect: Expect,
    }

    fn addr4(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn cases() -> Vec<Case> {
        vec![
            Case {
                name: "blank owner name inherits the previous record's owner",
                origin: "example.com.",
                input: "www 300 IN A 203.0.113.10\n    300 IN A 203.0.113.11\n",
                expect: Expect::Records(vec![
                    Record { name: "www.example.com.".into(), ttl: 300, rdata: RData::A(addr4("203.0.113.10")) },
                    Record { name: "www.example.com.".into(), ttl: 300, rdata: RData::A(addr4("203.0.113.11")) },
                ]),
            },
            Case {
                name: "$ORIGIN changes qualification partway through the file",
                origin: ".",
                input: "$ORIGIN example.com.\nwww 300 IN A 203.0.113.10\n$ORIGIN other.example.\nwww 300 IN A 203.0.113.20\n",
                expect: Expect::Records(vec![
                    Record { name: "www.example.com.".into(), ttl: 300, rdata: RData::A(addr4("203.0.113.10")) },
                    Record { name: "www.other.example.".into(), ttl: 300, rdata: RData::A(addr4("203.0.113.20")) },
                ]),
            },
            Case {
                name: "@ resolves to the origin itself, absolute names are left alone",
                origin: ".",
                input: "$ORIGIN example.com.\n@ 3600 IN NS ns1.example.com.\nns1 3600 IN A 203.0.113.53\n",
                expect: Expect::Records(vec![
                    Record { name: "example.com.".into(), ttl: 3600, rdata: RData::Ns("ns1.example.com.".into()) },
                    Record { name: "ns1.example.com.".into(), ttl: 3600, rdata: RData::A(addr4("203.0.113.53")) },
                ]),
            },
            Case {
                name: "escaped quote inside a TXT string is unescaped, not treated as the closing quote",
                origin: "example.com.",
                input: "txt1 300 IN TXT \"say \\\"hi\\\" there\"\n",
                expect: Expect::Records(vec![
                    Record {
                        name: "txt1.example.com.".into(),
                        ttl: 300,
                        rdata: RData::Txt(vec!["say \"hi\" there".into()]),
                    },
                ]),
            },
            Case {
                name: "a semicolon inside quotes is text, not the start of a comment",
                origin: "example.com.",
                input: "note 300 IN TXT \"contains a ; semicolon\" ; trailing comment\n",
                expect: Expect::Records(vec![
                    Record {
                        name: "note.example.com.".into(),
                        ttl: 300,
                        rdata: RData::Txt(vec!["contains a ; semicolon".into()]),
                    },
                ]),
            },
            Case {
                name: "comment-only and blank lines are skipped",
                origin: "example.com.",
                input: "; full line comment\n\nweb 60 IN A 203.0.113.1 ; comment here\n",
                expect: Expect::Records(vec![
                    Record { name: "web.example.com.".into(), ttl: 60, rdata: RData::A(addr4("203.0.113.1")) },
                ]),
            },
            Case {
                name: "record type and class keywords are case-insensitive",
                origin: ".",
                input: "$ORIGIN example.com.\nmail 300 in mx 10 mailhost\n",
                expect: Expect::Records(vec![
                    Record {
                        name: "mail.example.com.".into(),
                        ttl: 300,
                        rdata: RData::Mx { preference: 10, exchange: "mailhost.example.com.".into() },
                    },
                ]),
            },
            Case {
                name: "missing TTL with no prior $TTL directive is an error",
                origin: "example.com.",
                input: "www IN A 203.0.113.10\n",
                expect: Expect::Err,
            },
            Case {
                name: "$TTL sets the default used by records without an explicit TTL",
                origin: ".",
                input: "$ORIGIN example.com.\n$TTL 7200\nwww IN A 203.0.113.10\n",
                expect: Expect::Records(vec![
                    Record { name: "www.example.com.".into(), ttl: 7200, rdata: RData::A(addr4("203.0.113.10")) },
                ]),
            },
            Case {
                name: "unterminated quoted string is an error",
                origin: "example.com.",
                input: "bad 300 IN TXT \"unterminated\n",
                expect: Expect::Err,
            },
            Case {
                name: "parenthesized continuation joins a multi-string TXT record across lines",
                origin: "example.com.",
                input: "dkim 300 IN TXT ( \"v=DKIM1; k=rsa; \"\n    \"p=abcdef\" )\n",
                expect: Expect::Records(vec![Record {
                    name: "dkim.example.com.".into(),
                    ttl: 300,
                    rdata: RData::Txt(vec!["v=DKIM1; k=rsa; ".into(), "p=abcdef".into()]),
                }]),
            },
            Case {
                name: "comments on continuation lines inside parens are stripped",
                origin: ".",
                input: "$ORIGIN example.com.\nmail 300 IN MX ( ; open\n    10         ; preference\n    mailhost   ; exchange\n)                ; close\n",
                expect: Expect::Records(vec![Record {
                    name: "mail.example.com.".into(),
                    ttl: 300,
                    rdata: RData::Mx { preference: 10, exchange: "mailhost.example.com.".into() },
                }]),
            },
            Case {
                name: "a record after an unmatched closing paren is an error",
                origin: "example.com.",
                input: "bad 300 IN A ) 203.0.113.1\n",
                expect: Expect::Err,
            },
            Case {
                name: "an unclosed opening paren at end of file is an error",
                origin: "example.com.",
                input: "dkim 300 IN TXT ( \"v=DKIM1\"\n",
                expect: Expect::Err,
            },
        ]
    }

    #[test]
    fn table_driven_zone_parsing() {
        for case in cases() {
            let result = parse_zone(case.input, case.origin);
            match case.expect {
                Expect::Records(expected) => match result {
                    Ok(got) => assert_eq!(got, expected, "case failed: {}", case.name),
                    Err(e) => panic!("case \"{}\" expected records but got error: {}", case.name, e),
                },
                Expect::Err => {
                    assert!(result.is_err(), "case \"{}\" expected an error but parsing succeeded", case.name);
                }
            }
        }
    }
}
