// A small JSON reader/writer scoped to what the record format needs.
// Not a general-purpose library: no streaming, no arbitrary precision
// numbers, no comments. Just enough RFC 8259 to round-trip our records.

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

pub fn parse(input: &str) -> Result<Value, String> {
    let mut parser = Parser {
        chars: input.chars().collect(),
        pos: 0,
    };
    let value = parser.parse_value()?;
    parser.skip_ws();
    if parser.pos != parser.chars.len() {
        return Err(format!("trailing data at position {}", parser.pos));
    }
    Ok(value)
}

pub fn to_string_pretty(value: &Value) -> String {
    let mut out = String::new();
    write_value(value, 0, &mut out);
    out
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), String> {
        match self.bump() {
            Some(c) if c == expected => Ok(()),
            Some(c) => Err(format!("expected '{}' but found '{}'", expected, c)),
            None => Err(format!("expected '{}' but found end of input", expected)),
        }
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        self.skip_ws();
        match self.peek() {
            Some('"') => self.parse_string().map(Value::String),
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('t') | Some('f') => self.parse_bool(),
            Some('n') => self.parse_null(),
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            Some(c) => Err(format!("unexpected character '{}' at position {}", c, self.pos)),
            None => Err("unexpected end of input".to_string()),
        }
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect('"')?;
        let mut s = String::new();
        loop {
            match self.bump() {
                None => return Err("unterminated string".to_string()),
                Some('"') => break,
                Some('\\') => match self.bump() {
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some('/') => s.push('/'),
                    Some('b') => s.push('\u{0008}'),
                    Some('f') => s.push('\u{000C}'),
                    Some('n') => s.push('\n'),
                    Some('r') => s.push('\r'),
                    Some('t') => s.push('\t'),
                    Some('u') => {
                        let code = self.read_hex4()?;
                        s.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                    }
                    Some(other) => return Err(format!("invalid escape \\{}", other)),
                    None => return Err("unterminated escape sequence".to_string()),
                },
                Some(c) => s.push(c),
            }
        }
        Ok(s)
    }

    fn read_hex4(&mut self) -> Result<u32, String> {
        let mut value = 0u32;
        for _ in 0..4 {
            let c = self.bump().ok_or("unterminated unicode escape")?;
            let digit = c
                .to_digit(16)
                .ok_or_else(|| format!("invalid unicode escape digit '{}'", c))?;
            value = value * 16 + digit;
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            self.pos += 1;
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.pos += 1;
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        text.parse::<f64>()
            .map(Value::Number)
            .map_err(|_| format!("invalid number \"{}\"", text))
    }

    fn parse_bool(&mut self) -> Result<Value, String> {
        if self.chars[self.pos..].starts_with(&['t', 'r', 'u', 'e']) {
            self.pos += 4;
            Ok(Value::Bool(true))
        } else if self.chars[self.pos..].starts_with(&['f', 'a', 'l', 's', 'e']) {
            self.pos += 5;
            Ok(Value::Bool(false))
        } else {
            Err(format!("invalid literal at position {}", self.pos))
        }
    }

    fn parse_null(&mut self) -> Result<Value, String> {
        if self.chars[self.pos..].starts_with(&['n', 'u', 'l', 'l']) {
            self.pos += 4;
            Ok(Value::Null)
        } else {
            Err(format!("invalid literal at position {}", self.pos))
        }
    }

    fn parse_array(&mut self) -> Result<Value, String> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            let value = self.parse_value()?;
            items.push(value);
            self.skip_ws();
            match self.bump() {
                Some(',') => {
                    self.skip_ws();
                    continue;
                }
                Some(']') => break,
                _ => return Err("expected ',' or ']' in array".to_string()),
            }
        }
        Ok(Value::Array(items))
    }

    fn parse_object(&mut self) -> Result<Value, String> {
        self.expect('{')?;
        let mut fields = Vec::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(Value::Object(fields));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(':')?;
            let value = self.parse_value()?;
            fields.push((key, value));
            self.skip_ws();
            match self.bump() {
                Some(',') => continue,
                Some('}') => break,
                _ => return Err("expected ',' or '}' in object".to_string()),
            }
        }
        Ok(Value::Object(fields))
    }
}

fn write_value(value: &Value, indent: usize, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(*n, out),
        Value::String(s) => write_string(s, out),
        Value::Array(items) => write_array(items, indent, out),
        Value::Object(fields) => write_object(fields, indent, out),
    }
}

fn write_number(n: f64, out: &mut String) {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        out.push_str(&(n as i64).to_string());
    } else {
        out.push_str(&n.to_string());
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_array(items: &[Value], indent: usize, out: &mut String) {
    if items.is_empty() {
        out.push_str("[]");
        return;
    }
    out.push_str("[\n");
    let inner = indent + 1;
    for (i, item) in items.iter().enumerate() {
        push_indent(inner, out);
        write_value(item, inner, out);
        if i + 1 < items.len() {
            out.push(',');
        }
        out.push('\n');
    }
    push_indent(indent, out);
    out.push(']');
}

fn write_object(fields: &[(String, Value)], indent: usize, out: &mut String) {
    if fields.is_empty() {
        out.push_str("{}");
        return;
    }
    out.push_str("{\n");
    let inner = indent + 1;
    for (i, (key, value)) in fields.iter().enumerate() {
        push_indent(inner, out);
        write_string(key, out);
        out.push_str(": ");
        write_value(value, inner, out);
        if i + 1 < fields.len() {
            out.push(',');
        }
        out.push('\n');
    }
    push_indent(indent, out);
    out.push('}');
}

fn push_indent(level: usize, out: &mut String) {
    for _ in 0..level {
        out.push_str("  ");
    }
}
