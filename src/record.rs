// The record model both formats convert through, plus the JSON side of
// that conversion. The zone-file side lives in zone.rs.

use crate::json::Value;
use std::net::{Ipv4Addr, Ipv6Addr};

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub name: String,
    pub ttl: u32,
    pub rdata: RData,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RData {
    A(Ipv4Addr),
    Aaaa(Ipv6Addr),
    Cname(String),
    Mx { preference: u16, exchange: String },
    Txt(Vec<String>),
    Ns(String),
}

impl RData {
    pub fn type_name(&self) -> &'static str {
        match self {
            RData::A(_) => "A",
            RData::Aaaa(_) => "AAAA",
            RData::Cname(_) => "CNAME",
            RData::Mx { .. } => "MX",
            RData::Txt(_) => "TXT",
            RData::Ns(_) => "NS",
        }
    }
}

pub fn records_to_json(records: &[Record]) -> Value {
    Value::Array(records.iter().map(Record::to_json).collect())
}

pub fn records_from_json(value: &Value) -> Result<Vec<Record>, String> {
    match value {
        Value::Array(items) => items.iter().map(Record::from_json).collect(),
        _ => Err("top-level JSON value must be an array of records".to_string()),
    }
}

impl Record {
    fn to_json(&self) -> Value {
        let mut fields: Vec<(String, Value)> = vec![
            ("name".to_string(), Value::String(self.name.clone())),
            ("ttl".to_string(), Value::Number(self.ttl as f64)),
            ("type".to_string(), Value::String(self.rdata.type_name().to_string())),
        ];
        match &self.rdata {
            RData::A(addr) => fields.push(("address".to_string(), Value::String(addr.to_string()))),
            RData::Aaaa(addr) => fields.push(("address".to_string(), Value::String(addr.to_string()))),
            RData::Cname(target) => fields.push(("target".to_string(), Value::String(target.clone()))),
            RData::Ns(target) => fields.push(("target".to_string(), Value::String(target.clone()))),
            RData::Mx { preference, exchange } => {
                fields.push(("preference".to_string(), Value::Number(*preference as f64)));
                fields.push(("exchange".to_string(), Value::String(exchange.clone())));
            }
            RData::Txt(segments) => {
                let values = segments.iter().cloned().map(Value::String).collect();
                fields.push(("text".to_string(), Value::Array(values)));
            }
        }
        Value::Object(fields)
    }

    fn from_json(value: &Value) -> Result<Record, String> {
        let obj = match value {
            Value::Object(fields) => fields,
            _ => return Err("record must be a JSON object".to_string()),
        };

        let name = match find_field(obj, "name") {
            Some(Value::String(s)) => s.clone(),
            _ => return Err("record is missing a string \"name\" field".to_string()),
        };
        let ttl = match find_field(obj, "ttl") {
            Some(Value::Number(n)) => *n as u32,
            _ => return Err(format!("{}: missing a numeric \"ttl\" field", name)),
        };
        let type_name = match find_field(obj, "type") {
            Some(Value::String(s)) => s.to_uppercase(),
            _ => return Err(format!("{}: missing a string \"type\" field", name)),
        };

        let rdata = match type_name.as_str() {
            "A" => {
                let addr = match find_field(obj, "address") {
                    Some(Value::String(s)) => s
                        .parse::<Ipv4Addr>()
                        .map_err(|e| format!("{}: bad A address \"{}\": {}", name, s, e))?,
                    _ => return Err(format!("{}: A record needs an \"address\" field", name)),
                };
                RData::A(addr)
            }
            "AAAA" => {
                let addr = match find_field(obj, "address") {
                    Some(Value::String(s)) => s
                        .parse::<Ipv6Addr>()
                        .map_err(|e| format!("{}: bad AAAA address \"{}\": {}", name, s, e))?,
                    _ => return Err(format!("{}: AAAA record needs an \"address\" field", name)),
                };
                RData::Aaaa(addr)
            }
            "CNAME" => {
                let target = match find_field(obj, "target") {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err(format!("{}: CNAME record needs a \"target\" field", name)),
                };
                RData::Cname(target)
            }
            "NS" => {
                let target = match find_field(obj, "target") {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err(format!("{}: NS record needs a \"target\" field", name)),
                };
                RData::Ns(target)
            }
            "MX" => {
                let preference = match find_field(obj, "preference") {
                    Some(Value::Number(n)) => *n as u16,
                    _ => return Err(format!("{}: MX record needs a numeric \"preference\" field", name)),
                };
                let exchange = match find_field(obj, "exchange") {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err(format!("{}: MX record needs an \"exchange\" field", name)),
                };
                RData::Mx { preference, exchange }
            }
            "TXT" => {
                let text = match find_field(obj, "text") {
                    Some(Value::Array(items)) => {
                        let mut segments = Vec::with_capacity(items.len());
                        for item in items {
                            match item {
                                Value::String(s) => segments.push(s.clone()),
                                _ => return Err(format!("{}: TXT \"text\" entries must be strings", name)),
                            }
                        }
                        segments
                    }
                    _ => return Err(format!("{}: TXT record needs a \"text\" array", name)),
                };
                RData::Txt(text)
            }
            other => return Err(format!("{}: unsupported record type \"{}\"", name, other)),
        };

        Ok(Record { name, ttl, rdata })
    }
}

fn find_field<'a>(fields: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    for (k, v) in fields {
        if k.as_str() == key {
            return Some(v);
        }
    }
    None
}
